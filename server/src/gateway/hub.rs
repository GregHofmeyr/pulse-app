//! Connection registry + fan-out. Bounded per-connection queues: a client that cannot keep up
//! is dropped rather than slowing everyone else down.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::UserId;
use sqlx::SqlitePool;
use tokio::sync::mpsc;

use super::audience::audience_for;

pub const QUEUE: usize = 256;

pub type ConnId = u64;

#[derive(Clone, Default)]
pub struct Hub {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    next: AtomicU64,
    conns: Mutex<HashMap<ConnId, (UserId, mpsc::Sender<ServerFrame>)>>,
}

impl Hub {
    pub fn register(&self, user: UserId) -> (ConnId, mpsc::Receiver<ServerFrame>) {
        let (tx, rx) = mpsc::channel(QUEUE);
        let id = self.inner.next.fetch_add(1, Ordering::Relaxed);
        self.inner.conns.lock().unwrap().insert(id, (user, tx));
        (id, rx)
    }

    pub fn unregister(&self, id: ConnId) {
        self.inner.conns.lock().unwrap().remove(&id);
    }

    pub fn connections_for(&self, user: UserId) -> usize {
        self.inner
            .conns
            .lock()
            .unwrap()
            .values()
            .filter(|(u, _)| *u == user)
            .count()
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
        conns.retain(|id, (user, tx)| {
            if !audience.includes(*user) {
                return true;
            }
            match tx.try_send(frame.clone()) {
                Ok(()) => true,
                Err(mpsc::error::TrySendError::Full(_)) => {
                    tracing::warn!(conn = id, user = %user, "gateway client too slow; dropping");
                    false
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        });
    }
}
