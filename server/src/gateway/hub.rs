//! Connection registry + fan-out. Bounded per-connection queues: a client that cannot keep up
//! is dropped rather than slowing everyone else down.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::UserId;
use sqlx::SqlitePool;
use tokio::sync::{mpsc, oneshot};

use super::audience::audience_for;

pub const QUEUE: usize = 256;
/// Close code sent to a client the hub dropped for falling behind (it should resync via Ready).
pub const CLOSE_TOO_SLOW: u16 = 4003;
pub const CLOSE_UNAUTHORIZED: u16 = 4001;

pub type ConnId = u64;

pub struct Registration {
    pub id: ConnId,
    pub rx: mpsc::Receiver<ServerFrame>,
    /// Fires with a close code when the hub wants this socket gone.
    pub kick: oneshot::Receiver<u16>,
    /// The user had no other connection: they just came online.
    pub first: bool,
}

struct Conn {
    user: UserId,
    token_hash: String,
    tx: mpsc::Sender<ServerFrame>,
    kick: Option<oneshot::Sender<u16>>,
}

impl Conn {
    fn kick(&mut self, code: u16) {
        if let Some(k) = self.kick.take() {
            let _ = k.send(code);
        }
    }
}

#[derive(Clone, Default)]
pub struct Hub {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    next: AtomicU64,
    conns: Mutex<HashMap<ConnId, Conn>>,
}

impl Hub {
    /// `token_hash` ties the socket to its session so logout can close it.
    pub fn register(&self, user: UserId, token_hash: String) -> Registration {
        let (tx, rx) = mpsc::channel(QUEUE);
        let (ktx, krx) = oneshot::channel();
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        let mut conns = self.inner.conns.lock().unwrap();
        let first = !conns.values().any(|c| c.user == user);
        conns.insert(
            id,
            Conn {
                user,
                token_hash,
                tx,
                kick: Some(ktx),
            },
        );
        Registration {
            id,
            rx,
            kick: krx,
            first,
        }
    }

    /// Remove a connection; returns (user, whether it was their last connection). `None` if the
    /// hub already dropped it (logout, too slow, shutdown).
    pub fn unregister(&self, id: ConnId) -> Option<(UserId, bool)> {
        let mut conns = self.inner.conns.lock().unwrap();
        let c = conns.remove(&id)?;
        let last = !conns.values().any(|o| o.user == c.user);
        Some((c.user, last))
    }

    pub fn is_online(&self, user: UserId) -> bool {
        self.inner
            .conns
            .lock()
            .unwrap()
            .values()
            .any(|c| c.user == user)
    }

    pub fn connections_for(&self, user: UserId) -> usize {
        self.inner
            .conns
            .lock()
            .unwrap()
            .values()
            .filter(|c| c.user == user)
            .count()
    }

    /// Close every socket opened with this session (logout / revocation).
    pub fn drop_session(&self, token_hash: &str) {
        self.inner.conns.lock().unwrap().retain(|_, c| {
            if c.token_hash == token_hash {
                c.kick(CLOSE_UNAUTHORIZED);
                false
            } else {
                true
            }
        });
    }

    /// Close every socket with `code` (1012 = server restarting; clients reconnect).
    pub fn close_all(&self, code: u16) {
        for (_, mut c) in self.inner.conns.lock().unwrap().drain() {
            c.kick(code);
        }
    }

    /// Deliver `event` to every connection whose user is in its audience.
    pub async fn publish(&self, db: &SqlitePool, event: Event) {
        let audience = match audience_for(db, &event).await {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(error = ?e, "audience_for failed; event dropped");
                return;
            }
        };
        let frame = ServerFrame::Event(event);
        let mut conns = self.inner.conns.lock().unwrap();
        conns.retain(|id, c| {
            if !audience.includes(c.user) {
                return true;
            }
            match c.tx.try_send(frame.clone()) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(conn = id, user = %c.user, "gateway client too slow; dropping");
                    c.kick(CLOSE_TOO_SLOW);
                    false
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        });
    }
}
