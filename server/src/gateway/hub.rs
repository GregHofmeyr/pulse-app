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
/// Close code for a client too old for this server ("plz update :)").
pub const CLOSE_UPDATE_REQUIRED: u16 = 4005;
/// Close code for a connection over the per-user cap (existing connections are never kicked).
pub const CLOSE_TOO_MANY_CONNECTIONS: u16 = 4006;

pub type ConnId = u64;

pub struct Registration {
    pub id: ConnId,
    pub rx: mpsc::Receiver<ServerFrame>,
    /// Fires with a close code when the hub wants this socket gone.
    pub kick: oneshot::Receiver<u16>,
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
    /// What everyone was last told about each user's presence. Locked only while `conns` is held.
    announced: Mutex<HashMap<UserId, bool>>,
    /// See [`Hub::in_order`].
    ordered: tokio::sync::Mutex<()>,
}

impl Hub {
    /// `token_hash` ties the socket to its session so logout can close it.
    pub fn register(&self, user: UserId, token_hash: String) -> Registration {
        self.try_register(user, token_hash, usize::MAX)
            .expect("no cap")
    }

    /// Register unless `user` already has `max` connections. The count and the insert happen
    /// under one lock, so simultaneous connects can't both take the last slot.
    pub fn try_register(
        &self,
        user: UserId,
        token_hash: String,
        max: usize,
    ) -> Option<Registration> {
        let mut conns = self.inner.conns.lock().unwrap();
        if conns.values().filter(|c| c.user == user).count() >= max {
            return None;
        }
        let (tx, rx) = mpsc::channel(QUEUE);
        let (ktx, krx) = oneshot::channel();
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        conns.insert(
            id,
            Conn {
                user,
                token_hash,
                tx,
                kick: Some(ktx),
            },
        );
        Some(Registration { id, rx, kick: krx })
    }

    /// Remove a connection; returns (user, whether it was their last connection). `None` if the
    /// hub already dropped it (logout, too slow, shutdown).
    pub fn unregister(&self, id: ConnId) {
        self.inner.conns.lock().unwrap().remove(&id);
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

    /// Hold this from before a write transaction until its events are published, so events
    /// carrying absolute state (reaction counts) are delivered in commit order. Taken before
    /// `begin_write`, so waiters hold no pool connection.
    pub async fn in_order(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.inner.ordered.lock().await
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
        deliver(&mut self.inner.conns.lock().unwrap(), &audience, &frame);
    }

    /// Tell everyone whether `user` is online, but only if that changed since the last time.
    /// The truth is read under the same lock that sends, so racing connects and disconnects
    /// can neither repeat an announcement nor reorder two of them. `last_seen_at` goes out with
    /// an "offline". Returns what was announced, if anything.
    pub async fn sync_presence(
        &self,
        db: &SqlitePool,
        user: UserId,
        last_seen_at: String,
    ) -> Option<bool> {
        let probe = Event::PresenceChanged {
            user_id: user,
            online: true,
            last_seen_at: None,
        };
        let audience = match audience_for(db, &probe).await {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(error = ?e, "audience_for failed; presence not announced");
                return None;
            }
        };
        let mut conns = self.inner.conns.lock().unwrap();
        let online = conns.values().any(|c| c.user == user);
        let before = self.inner.announced.lock().unwrap().insert(user, online);
        if before.unwrap_or(false) == online {
            return None;
        }
        let frame = ServerFrame::Event(Event::PresenceChanged {
            user_id: user,
            online,
            last_seen_at: (!online).then_some(last_seen_at),
        });
        deliver(&mut conns, &audience, &frame);
        Some(online)
    }
}

/// Queue `frame` for every connection in `audience`; drop connections that can't keep up.
fn deliver(
    conns: &mut HashMap<ConnId, Conn>,
    audience: &super::audience::Audience,
    frame: &ServerFrame,
) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_register_caps_connections_per_user_even_under_a_race() {
        let hub = Hub::default();
        let user = UserId::new();
        let threads: Vec<_> = (0..32)
            .map(|_| {
                let hub = hub.clone();
                std::thread::spawn(move || hub.try_register(user, String::new(), 8))
            })
            .collect();
        let regs: Vec<Registration> = threads
            .into_iter()
            .filter_map(|t| t.join().unwrap())
            .collect();
        assert_eq!(regs.len(), 8, "exactly the cap got in");
        assert_eq!(hub.connections_for(user), 8);
        assert!(
            hub.try_register(UserId::new(), String::new(), 8).is_some(),
            "others have their own allowance"
        );
        hub.unregister(regs[0].id);
        assert!(
            hub.try_register(user, String::new(), 8).is_some(),
            "a slot freed"
        );
        assert!(hub.try_register(user, String::new(), 8).is_none());
    }
}
