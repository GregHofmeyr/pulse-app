//! Connection registry + fan-out. Bounded per-connection queues: a client that cannot keep up
//! is dropped rather than slowing everyone else down.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{Listening, Person, User};
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

/// One connection's "Listening to" (#275).
struct Held {
    user: UserId,
    /// When it was set (the hub's listening counter): a person's most recent one is theirs.
    at: u64,
    /// Others may see it as the person's activity; either way it feeds their Jam on the stage.
    show: bool,
    listening: Listening,
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
    /// The inbox unread count each connected user was last told (#306: Ready, then inbox events), so a read
    /// point that moves without changing it tells them nothing.
    inbox_told: Mutex<HashMap<UserId, u32>>,
    /// Each connection's "Listening to" (#275). Locked only while `conns` is held (after it).
    listening: Mutex<HashMap<ConnId, Held>>,
    /// Each connection's newest listening update, by ticket ([`Hub::listening_ticket`]): one that took longer to
    /// get its art than a later one never lands over it. Locked like `listening`.
    listening_seq: Mutex<HashMap<ConnId, u64>>,
    listening_next: AtomicU64,
    /// What each connection was last told each person is listening to (absent: nothing), like `announced`.
    announced_listening: Mutex<HashMap<(UserId, ConnId), Listening>>,
}

/// `user`'s most recent "Listening to" among the connections still in `conns` (`shown_only`: only one they share).
fn latest(
    conns: &HashMap<ConnId, Conn>,
    held: &HashMap<ConnId, Held>,
    user: UserId,
    shown_only: bool,
) -> Option<Listening> {
    held.iter()
        .filter(|(id, h)| h.user == user && (h.show || !shown_only) && conns.contains_key(id))
        .max_by_key(|(_, h)| h.at)
        .map(|(_, h)| h.listening.clone())
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
        let gone = conns.remove(&id);
        self.inner
            .announced
            .lock()
            .unwrap()
            .retain(|(_, conn), _| *conn != id);
        self.inner.listening.lock().unwrap().remove(&id);
        self.inner.listening_seq.lock().unwrap().remove(&id);
        self.inner
            .announced_listening
            .lock()
            .unwrap()
            .retain(|(_, conn), _| *conn != id);
        if let Some(gone) = gone
            && !conns.values().any(|c| c.user == gone.user)
        {
            self.inner.inbox_told.lock().unwrap().remove(&gone.user);
        }
    }

    /// A ticket for connection `conn`'s next listening update (#275), taken as the frame arrives: only the newest
    /// ticket's update may land ([`Hub::set_listening`]).
    pub fn listening_ticket(&self, conn: ConnId) -> u64 {
        let conns = self.inner.conns.lock().unwrap();
        let seq = self.inner.listening_next.fetch_add(1, Ordering::Relaxed) + 1;
        // a connection already gone gets a ticket that can never land
        if conns.contains_key(&conn) {
            self.inner.listening_seq.lock().unwrap().insert(conn, seq);
        }
        seq
    }

    /// Set (or clear, `None`) what `user`'s connection `conn` is listening to, if it's still connected and `ticket`
    /// is its newest; whether it was set. Announcing it is [`Hub::sync_presence`]'s.
    pub fn set_listening(
        &self,
        conn: ConnId,
        user: UserId,
        ticket: u64,
        listening: Option<(Listening, bool)>,
    ) -> bool {
        let conns = self.inner.conns.lock().unwrap();
        if conns.get(&conn).is_none_or(|c| c.user != user)
            || self.inner.listening_seq.lock().unwrap().get(&conn) != Some(&ticket)
        {
            return false;
        }
        let mut held = self.inner.listening.lock().unwrap();
        match listening {
            Some((listening, show)) => {
                let at = self.inner.listening_next.fetch_add(1, Ordering::Relaxed) + 1;
                held.insert(
                    conn,
                    Held {
                        user,
                        at,
                        show,
                        listening,
                    },
                );
            }
            None => {
                held.remove(&conn);
            }
        }
        true
    }

    /// What `user` is listening to (#275), from their most recently updated connection: only one they share
    /// (`shown_only`, for their activity), or any (for their Jam on the stage).
    pub fn listening_of(&self, user: UserId, shown_only: bool) -> Option<Listening> {
        let conns = self.inner.conns.lock().unwrap();
        latest(
            &conns,
            &self.inner.listening.lock().unwrap(),
            user,
            shown_only,
        )
    }

    /// What connection `conn` itself holds (a pause and resume of the same song keeps its art without fetching it
    /// again).
    pub fn listening_held(&self, conn: ConnId) -> Option<Listening> {
        let _conns = self.inner.conns.lock().unwrap();
        self.inner
            .listening
            .lock()
            .unwrap()
            .get(&conn)
            .map(|h| h.listening.clone())
    }

    /// Note that `user` is being told their inbox has `unread` unread items (#306); true when that differs
    /// from what they were last told (or they were told nothing yet).
    pub fn tell_inbox(&self, user: UserId, unread: u32) -> bool {
        self.inner.inbox_told.lock().unwrap().insert(user, unread) != Some(unread)
    }

    /// [`Hub::tell_inbox`] for a count that may not have changed: true (and noted) only when `user` was told a
    /// count before (their Ready) and this one differs from it.
    pub fn inbox_changed(&self, user: UserId, unread: u32) -> bool {
        match self.inner.inbox_told.lock().unwrap().get_mut(&user) {
            Some(told) if *told != unread => {
                *told = unread;
                true
            }
            _ => false,
        }
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

    /// Deliver `event` to every connection whose user is in its audience, then the inbox events it implies
    /// (#306, `inbox::follow_ups`) to their own users' sessions, right behind it.
    pub async fn publish(&self, db: &SqlitePool, event: Event) {
        let audience = match audience_for(db, &event).await {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(error = ?e, "audience_for failed; event dropped");
                return;
            }
        };
        let follow = crate::inbox::follow_ups(db, self, &event).await;
        let frame = ServerFrame::Event(event);
        let mut conns = self.inner.conns.lock().unwrap();
        deliver(&mut conns, &audience, &frame);
        for e in follow {
            // Inbox events are always one user's own (`Event::only_for`).
            let Some(owner) = e.only_for() else { continue };
            let to = Audience::Users([owner].into_iter().collect());
            deliver(&mut conns, &to, &ServerFrame::Event(e));
        }
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
    /// told anything new about their presence.
    ///
    /// Then what they're listening to (#275), by the same rule: only to viewers they look online to (and to
    /// themselves), so going Invisible or hiding from someone takes it from them (`None`) as it takes the dot.
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
        let mut listening_to: HashMap<ConnId, ServerFrame> = HashMap::new();
        {
            let mut announced = self.inner.announced.lock().unwrap();
            let shown = latest(&conns, &self.inner.listening.lock().unwrap(), user, true);
            let mut told = self.inner.announced_listening.lock().unwrap();
            for (id, c) in conns.iter() {
                let view = view_for(&subject, c.user, connected, &now);
                let want = shown
                    .as_ref()
                    .filter(|_| c.user == user || view.online())
                    .cloned();
                if told.get(&(user, *id)) != want.as_ref() {
                    match &want {
                        Some(l) => told.insert((user, *id), l.clone()),
                        None => told.remove(&(user, *id)),
                    };
                    listening_to.insert(
                        *id,
                        ServerFrame::Event(Event::ListeningChanged {
                            user_id: user,
                            listening: want,
                        }),
                    );
                }
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
        // the presence first: a client drops the activity of someone who went offline
        send_each(&mut conns, &to);
        send_each(&mut conns, &listening_to);
        (!to.is_empty()).then_some(connected)
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
        let held = self.inner.listening.lock().unwrap();
        let mut told = self.inner.announced_listening.lock().unwrap();
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
                let listening = (user.id == viewer || view.online())
                    .then(|| latest(&conns, &held, user.id, true))
                    .flatten();
                if let Some(conn) = conn {
                    announced.insert((user.id, conn), view.clone());
                    match &listening {
                        Some(l) => told.insert((user.id, conn), l.clone()),
                        None => told.remove(&(user.id, conn)),
                    };
                }
                Person {
                    user,
                    online: view.online(),
                    last_seen_at: view.last_seen_at,
                    status: view.status,
                    listening,
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
