//! Connection registry + fan-out. Bounded per-connection queues: a client that cannot keep up
//! is dropped rather than slowing everyone else down.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{Person, User};
use sqlx::SqlitePool;
use tokio::sync::{mpsc, oneshot};

use super::audience::{Audience, audience_for};
use crate::presence::{PresenceView, load_all, load_subject, view_for};

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
    /// What each connection was last told about each person's presence, by (person, connection):
    /// per connection, not per viewer, because a viewer's new device starts from its own Ready
    /// while their other devices still show what they were told. Locked only while `conns` is held.
    announced: Mutex<HashMap<(UserId, ConnId), PresenceView>>,
    /// Held across reading presence from the database and telling viewers, by every announcement
    /// and by Ready's people, so a status change and a connect can't reorder what anyone is told.
    presence: tokio::sync::Mutex<()>,
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
        let mut conns = self.inner.conns.lock().unwrap();
        conns.remove(&id);
        self.inner
            .announced
            .lock()
            .unwrap()
            .retain(|(_, conn), _| *conn != id);
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

    /// Close `user`'s sockets opened with any session other than `keep_hash` ("sign out all
    /// other devices"); the caller's own sockets and everyone else's stay open.
    pub fn drop_other_sessions(&self, user: UserId, keep_hash: &str) {
        self.inner.conns.lock().unwrap().retain(|_, c| {
            if c.user == user && c.token_hash != keep_hash {
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

    /// Like [`Hub::publish`], but only to the members of `only` who are also in the event's
    /// audience, so a narrower list (people in a call) can never widen who may see it.
    pub async fn publish_to(&self, db: &SqlitePool, event: Event, only: &HashSet<UserId>) {
        let audience = match audience_for(db, &event).await {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(error = ?e, "audience_for failed; event dropped");
                return;
            }
        };
        let allowed = Audience::Users(
            only.iter()
                .copied()
                .filter(|u| audience.includes(*u))
                .collect(),
        );
        let frame = ServerFrame::Event(event);
        deliver(&mut self.inner.conns.lock().unwrap(), &allowed, &frame);
    }

    /// Tell each connected viewer how `user` looks to them now (#45: per viewer, see
    /// [`view_for`]), but only those whose view changed since they were last told. Connections are
    /// read under the same lock that sends, so racing connects and disconnects can neither repeat
    /// an announcement nor reorder two of them. Returns whether `user` is connected, if anyone was
    /// told anything.
    pub async fn sync_presence(&self, db: &SqlitePool, user: UserId) -> Option<bool> {
        let _order = self.inner.presence.lock().await;
        let subject = match load_subject(db, user).await {
            Ok(Some(s)) => s,
            Ok(None) => return None,
            Err(e) => {
                tracing::error!(error = ?e, "presence not announced");
                return None;
            }
        };
        let now = crate::db::now();
        let mut conns = self.inner.conns.lock().unwrap();
        let connected = conns.values().any(|c| c.user == user);
        let mut views: HashMap<UserId, ServerFrame> = HashMap::new();
        let mut to: HashMap<ConnId, ServerFrame> = HashMap::new();
        {
            let mut announced = self.inner.announced.lock().unwrap();
            for (id, c) in conns.iter() {
                let view = view_for(&subject, c.user, connected, &now);
                if announced.get(&(user, *id)) == Some(&view) {
                    continue;
                }
                let frame = views
                    .entry(c.user)
                    .or_insert_with(|| {
                        ServerFrame::Event(Event::PresenceChanged {
                            user_id: user,
                            online: view.online(),
                            last_seen_at: view.last_seen_at.clone(),
                            status: view.status,
                        })
                    })
                    .clone();
                announced.insert((user, *id), view);
                to.insert(*id, frame);
            }
        }
        if to.is_empty() {
            return None;
        }
        send_each(&mut conns, &to);
        Some(connected)
    }

    /// Everyone in `users` as `viewer` may see them, for the Ready of `viewer`'s connection
    /// `conn`; what it says is what later announcements to that connection are measured against.
    pub async fn people_for(
        &self,
        db: &SqlitePool,
        viewer: UserId,
        conn: Option<ConnId>,
        users: Vec<(User, Option<String>)>,
    ) -> anyhow::Result<Vec<Person>> {
        let _order = self.inner.presence.lock().await;
        let all = load_all(db).await?;
        let now = crate::db::now();
        let conns = self.inner.conns.lock().unwrap();
        let mut announced = self.inner.announced.lock().unwrap();
        Ok(users
            .into_iter()
            .map(|(user, last_seen_at)| {
                let connected = conns.values().any(|c| c.user == user.id);
                let view = match all.get(&user.id) {
                    Some(s) => view_for(s, viewer, connected, &now),
                    // Someone who registered between the two reads: no status yet.
                    None => PresenceView {
                        status: if connected {
                            pulse_protocol::rest::PresenceStatus::Online
                        } else {
                            pulse_protocol::rest::PresenceStatus::Offline
                        },
                        last_seen_at: (!connected).then_some(last_seen_at).flatten(),
                    },
                };
                if let Some(conn) = conn {
                    announced.insert((user.id, conn), view.clone());
                }
                Person {
                    user,
                    online: view.online(),
                    last_seen_at: view.last_seen_at,
                    status: view.status,
                }
            })
            .collect())
    }
}

/// Queue each connection's own frame; drop connections that can't keep up.
fn send_each(conns: &mut HashMap<ConnId, Conn>, frames: &HashMap<ConnId, ServerFrame>) {
    conns.retain(|id, c| {
        let Some(frame) = frames.get(id) else {
            return true;
        };
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

    #[test]
    fn drop_other_sessions_kicks_only_that_users_other_sessions() {
        let hub = Hub::default();
        let alex = UserId::new();
        let sam = UserId::new();
        let mut keep = hub.register(alex, "keep".into());
        let mut other = hub.register(alex, "other".into());
        let mut sams = hub.register(sam, "other".into());
        hub.drop_other_sessions(alex, "keep");
        assert_eq!(other.kick.try_recv(), Ok(CLOSE_UNAUTHORIZED));
        assert!(keep.kick.try_recv().is_err(), "caller's socket stays");
        assert!(sams.kick.try_recv().is_err(), "other users untouched");
        assert_eq!(hub.connections_for(alex), 1);
        assert_eq!(hub.connections_for(sam), 1);
    }
}
