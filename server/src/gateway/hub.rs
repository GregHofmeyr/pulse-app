//! Connection registry + fan-out. Bounded per-connection queues: a client that cannot keep up
//! is dropped rather than slowing everyone else down.
//!
//! Ordering of one [`Hub::publish_all`] (#349; clients rely on it, the inbox's `arrived` among
//! them):
//! 1. Every event's audience is worked out first, before any of them is delivered, all from the
//!    database as the caller's write left it.
//! 2. The events are then delivered in the order given, under the connection lock, so no other
//!    publish interleaves with them on any connection.
//! 3. Only after that are the inbox events they imply (`InboxItemAdded`, `InboxUpdated`) queued for
//!    the inbox worker, which reads each item and count afresh and sends it later. So a device
//!    always has the message (and its chat) before the inbox item for it, but a read point, a
//!    thread seen point or a delete published meanwhile can reach it before that item: the
//!    client checks an arriving item against what it already knows.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{Listening, Person, Playing, ServerPresence, SteamJoin, User};
use sqlx::SqlitePool;
use tokio::sync::{mpsc, oneshot};

use super::audience::{Audience, route};
use crate::presence::{PresenceView, load_all, load_subject, view_for};

/// What a connection was last told of someone's presence: their view, and inside which servers it differs (#154).
type Told = (PresenceView, Vec<ServerPresence>);

pub const QUEUE: usize = 256;
/// Close code sent to a client the hub dropped for falling behind (it should resync via Ready).
pub const CLOSE_TOO_SLOW: u16 = 4003;
pub const CLOSE_UNAUTHORIZED: u16 = 4001;
/// Close code for a client too old for this server ("plz update :)").
pub const CLOSE_UPDATE_REQUIRED: u16 = 4005;
/// Close code for a connection over the per-user cap (existing connections are never kicked).
pub const CLOSE_TOO_MANY_CONNECTIONS: u16 = 4006;
/// Close code for a client that should connect again for a fresh Ready: a version-3 client that just joined a server
/// (#80), whose `ServerJoined` it would skip. Older clients reconnect on any code they don't know.
pub const CLOSE_RESYNC: u16 = 4007;
/// The first protocol version that reads `ServerJoined` (#80, [`pulse_protocol::PROTOCOL_VERSION`] 4).
const SERVER_JOINED_SINCE: u32 = 4;

pub type ConnId = u64;

/// A connected user's inbox (#306, #327): what they were last told their unread count is (`None`: nothing yet, or
/// forgotten because it changed with no event of its own), behind their own lock. Every count is read from the
/// database and sent while it is held, so counts reach their devices in the order they were read: a stale count
/// can never land after a fresh one, nor make the hub skip one as unchanged.
pub type InboxSlot = Arc<tokio::sync::Mutex<Option<u32>>>;

/// Inbox work for the hub's inbox worker ([`Hub::publish_all`]).
type InboxBatch = (SqlitePool, Vec<crate::inbox::Job>);

/// The most batches an inbox worker takes in one go (it coalesces their counts, `inbox::run`).
const INBOX_BATCHES: usize = 256;
/// How many inbox workers share the work, each person's always on the same one (#328, `inbox::Job::shard`): a
/// slow count for one person holds up only those on their worker.
const INBOX_SHARDS: usize = 4;
/// The most batches waiting for one inbox worker. Publishers never wait (they may hold `in_order`): work that
/// doesn't fit is collapsed into the worker's [`crate::inbox::Overflow`] instead.
const INBOX_QUEUE: usize = 1024;

/// One inbox worker's queue, and the work that didn't fit it.
struct InboxShard {
    tx: mpsc::Sender<InboxBatch>,
    overflow: Arc<Mutex<crate::inbox::Overflow>>,
}

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
    /// Its Ready's inbox count has been read: inbox events reach it from then on (one read before would be
    /// older than its Ready, which supersedes it).
    inbox: bool,
    /// This device says it has had no input for a while (#53, `ClientFrame::Idle`).
    idle: bool,
    /// The protocol version its Hello said ([`Hub::set_client_version`]; this build's until then).
    client_version: u32,
}

impl Conn {
    fn kick(&mut self, code: u16) {
        if let Some(k) = self.kick.take() {
            let _ = k.send(code);
        }
    }
}

/// One connection's activity of one kind: its "Listening to" (#275) or its "Playing" (#380).
struct Held<T> {
    user: UserId,
    /// When it was set (the slot's counter): a person's most recent one is theirs.
    at: u64,
    /// Others may see it as the person's activity (a "Listening to" that isn't shown still feeds their Jam on the
    /// stage; a "Playing" is always shown).
    show: bool,
    value: T,
}

/// One kind of activity, held per connection (#275, #380): what each connection holds, each one's newest update by
/// ticket ([`Slot::ticket`]: one that took longer to get its art than a later one never lands over it), and what each
/// connection was last told each person has (absent: nothing), like `Inner::announced`. Its locks are taken only while
/// `conns` is held (after it), and never two slots' at once.
struct Slot<T> {
    held: Mutex<HashMap<ConnId, Held<T>>>,
    seq: Mutex<HashMap<ConnId, u64>>,
    next: AtomicU64,
    announced: Mutex<HashMap<(UserId, ConnId), T>>,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Slot {
            held: Mutex::default(),
            seq: Mutex::default(),
            next: AtomicU64::new(0),
            announced: Mutex::default(),
        }
    }
}

impl<T: Clone + PartialEq> Slot<T> {
    /// Everything about connection `conn`, which has gone.
    fn forget(&self, conn: ConnId) {
        self.held.lock().unwrap().remove(&conn);
        self.seq.lock().unwrap().remove(&conn);
        self.announced
            .lock()
            .unwrap()
            .retain(|(_, c), _| *c != conn);
    }

    /// A ticket for `conn`'s next update: only the newest may land. A connection already gone (not in `conns`) gets
    /// one that never can.
    fn ticket(&self, conns: &HashMap<ConnId, Conn>, conn: ConnId) -> u64 {
        let seq = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        if conns.contains_key(&conn) {
            self.seq.lock().unwrap().insert(conn, seq);
        }
        seq
    }

    /// Set (or clear) what `user`'s connection `conn` holds, if it's still theirs and `ticket` is its newest.
    fn set(
        &self,
        conns: &HashMap<ConnId, Conn>,
        conn: ConnId,
        user: UserId,
        ticket: u64,
        value: Option<(T, bool)>,
    ) -> bool {
        if conns.get(&conn).is_none_or(|c| c.user != user)
            || self.seq.lock().unwrap().get(&conn) != Some(&ticket)
        {
            return false;
        }
        let mut held = self.held.lock().unwrap();
        match value {
            Some((value, show)) => {
                let at = self.next.fetch_add(1, Ordering::Relaxed) + 1;
                held.insert(
                    conn,
                    Held {
                        user,
                        at,
                        show,
                        value,
                    },
                );
            }
            None => {
                held.remove(&conn);
            }
        }
        true
    }

    /// `user`'s most recent one among the connections still in `conns` (`shown_only`: only one they share).
    fn latest(&self, conns: &HashMap<ConnId, Conn>, user: UserId, shown_only: bool) -> Option<T> {
        self.held
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, h)| h.user == user && (h.show || !shown_only) && conns.contains_key(id))
            .max_by_key(|(_, h)| h.at)
            .map(|(_, h)| h.value.clone())
    }

    /// What connection `conn` itself holds.
    fn held(&self, conn: ConnId) -> Option<T> {
        self.held
            .lock()
            .unwrap()
            .get(&conn)
            .map(|h| h.value.clone())
    }

    /// Record that connection `conn` is told `user` has `want`; whether that's news to it.
    fn tell(&self, user: UserId, conn: ConnId, want: Option<&T>) -> bool {
        let mut told = self.announced.lock().unwrap();
        if told.get(&(user, conn)) == want {
            return false;
        }
        match want {
            Some(v) => told.insert((user, conn), v.clone()),
            None => told.remove(&(user, conn)),
        };
        true
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
    /// Every connection whose socket is still open, by its user: registered and not yet unregistered. The hub drops
    /// connections from `conns` itself (a logout, too slow, shutdown) before their sockets finish closing, so for a
    /// moment a user can have open sockets and no connection (#449, [`Hub::is_closing`]). Locked only while `conns` is
    /// held (after it).
    open: Mutex<HashMap<ConnId, UserId>>,
    /// What each connection was last told about each person's presence, by (person, connection):
    /// per connection, not per viewer, because a viewer's new device starts from its own Ready
    /// while their other devices still show what they were told. Locked only while `conns` is held.
    /// With it, how they look inside servers where they've set a status of their own (#154).
    announced: Mutex<HashMap<(UserId, ConnId), Told>>,
    /// Whose profile each connection has, by (person, connection) (#486): Ready's people, or a `UserCreated` since.
    /// Only people who share a server, a DM or a group with them (and they themselves) are in it; the presence sync
    /// adds someone who comes to share one, with their profile, and forgets someone who no longer does. Locked only
    /// while `conns` is held.
    introduced: Mutex<HashSet<(UserId, ConnId)>>,
    /// Held across reading presence from the database and telling viewers, by every announcement
    /// and by Ready's people, so a status change and a connect can't reorder what anyone is told.
    presence: tokio::sync::Mutex<()>,
    /// See [`Hub::in_order`].
    ordered: tokio::sync::Mutex<()>,
    /// Each connected user's [`InboxSlot`]. Locked only while `conns` is held (after it), so a slot is made only
    /// for someone connected and goes with their last connection.
    inbox: Mutex<HashMap<UserId, InboxSlot>>,
    /// The inbox workers' queues (#327, #328, [`Hub::publish_all`]), each started on first use (and again if it
    /// ever ends).
    inbox_queue: Mutex<[Option<InboxShard>; INBOX_SHARDS]>,
    /// Each connection's "Listening to" (#275).
    listening: Slot<Listening>,
    /// Each connection's "Playing" (#380).
    playing: Slot<Playing>,
    /// Each person's joinable Steam lobby or game server (#381), from the Steam poller: shown inside their "Playing"
    /// (so only to those it's shown to, and only while they play). Locked only while `conns` is held (after it).
    steam_join: Mutex<HashMap<UserId, SteamJoin>>,
}

/// Whether `user` is connected, and whether every connection of theirs is idle (#53): one device in use is enough
/// to look active.
fn live(conns: &HashMap<ConnId, Conn>, user: UserId) -> (bool, bool) {
    let mut theirs = conns.values().filter(|c| c.user == user).peekable();
    let connected = theirs.peek().is_some();
    (connected, connected && theirs.all(|c| c.idle))
}

/// See [`Hub::is_closing`].
fn closing(conns: &HashMap<ConnId, Conn>, open: &HashMap<ConnId, UserId>, user: UserId) -> bool {
    !conns.values().any(|c| c.user == user) && open.values().any(|u| *u == user)
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
                inbox: false,
                idle: false,
                client_version: pulse_protocol::PROTOCOL_VERSION,
            },
        );
        self.inner.open.lock().unwrap().insert(id, user);
        Some(Registration { id, rx, kick: krx })
    }

    /// The protocol version `id`'s client speaks, from its Hello: a client too old for an event is reconnected
    /// rather than sent it ([`CLOSE_RESYNC`]).
    pub fn set_client_version(&self, id: ConnId, version: u32) {
        if let Some(c) = self.inner.conns.lock().unwrap().get_mut(&id) {
            c.client_version = version;
        }
    }

    /// Remove a connection; returns (user, whether it was their last connection). `None` if the
    /// hub already dropped it (logout, too slow, shutdown).
    pub fn unregister(&self, id: ConnId) {
        let mut conns = self.inner.conns.lock().unwrap();
        let gone = conns.remove(&id);
        self.inner.open.lock().unwrap().remove(&id);
        self.inner
            .announced
            .lock()
            .unwrap()
            .retain(|(_, conn), _| *conn != id);
        self.inner
            .introduced
            .lock()
            .unwrap()
            .retain(|(_, conn)| *conn != id);
        self.inner.listening.forget(id);
        self.inner.playing.forget(id);
        if gone.is_some() {
            self.prune_inbox(&conns);
        }
    }

    /// Drop the inbox slots of users with no connection left in `conns` (the `conns` lock held), whichever way
    /// their last one went: unregistered, or dropped by the hub itself (a kicked session, too slow, shutdown).
    fn prune_inbox(&self, conns: &HashMap<ConnId, Conn>) {
        self.inner
            .inbox
            .lock()
            .unwrap()
            .retain(|u, _| conns.values().any(|c| c.user == *u));
    }

    /// Set whether `user`'s connection `conn` is idle (#53); whether that changed whether `user` looks idle (then
    /// announce it with [`Hub::sync_presence`]). A connection that's gone, or isn't theirs, changes nothing.
    pub fn set_idle(&self, conn: ConnId, user: UserId, idle: bool) -> bool {
        let mut conns = self.inner.conns.lock().unwrap();
        let before = live(&conns, user).1;
        match conns.get_mut(&conn) {
            Some(c) if c.user == user => c.idle = idle,
            _ => return false,
        }
        live(&conns, user).1 != before
    }

    /// Whether `user` is connected and every connection of theirs is idle (#53).
    pub fn is_idle(&self, user: UserId) -> bool {
        live(&self.inner.conns.lock().unwrap(), user).1
    }

    /// A ticket for connection `conn`'s next listening update (#275), taken as the frame arrives: only the newest
    /// ticket's update may land ([`Hub::set_listening`]).
    pub fn listening_ticket(&self, conn: ConnId) -> u64 {
        let conns = self.inner.conns.lock().unwrap();
        self.inner.listening.ticket(&conns, conn)
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
        self.inner
            .listening
            .set(&conns, conn, user, ticket, listening)
    }

    /// What `user` is listening to (#275), from their most recently updated connection: only one they share
    /// (`shown_only`, for their activity), or any (for their Jam on the stage).
    pub fn listening_of(&self, user: UserId, shown_only: bool) -> Option<Listening> {
        let conns = self.inner.conns.lock().unwrap();
        self.inner.listening.latest(&conns, user, shown_only)
    }

    /// What connection `conn` itself holds (a pause and resume of the same song keeps its art without fetching it
    /// again).
    pub fn listening_held(&self, conn: ConnId) -> Option<Listening> {
        let _conns = self.inner.conns.lock().unwrap();
        self.inner.listening.held(conn)
    }

    /// A ticket for connection `conn`'s next "Playing" update (#380), as [`Hub::listening_ticket`].
    pub fn playing_ticket(&self, conn: ConnId) -> u64 {
        let conns = self.inner.conns.lock().unwrap();
        self.inner.playing.ticket(&conns, conn)
    }

    /// Set (or clear) what game `user`'s connection `conn` is playing (#380), as [`Hub::set_listening`]; always shown.
    /// Another game (or none) drops their Steam Join (#381) until the poller finds one for it.
    pub fn set_playing(
        &self,
        conn: ConnId,
        user: UserId,
        ticket: u64,
        playing: Option<Playing>,
    ) -> bool {
        let conns = self.inner.conns.lock().unwrap();
        let game = |p: Option<Playing>| p.map(|p| (p.name, p.steam_app_id));
        let before = game(self.inner.playing.latest(&conns, user, true));
        let set = self
            .inner
            .playing
            .set(&conns, conn, user, ticket, playing.map(|p| (p, true)));
        if set && game(self.inner.playing.latest(&conns, user, true)) != before {
            self.inner.steam_join.lock().unwrap().remove(&user);
        }
        set
    }

    /// What game `user` is playing (#380), from their most recently updated connection, with its Join (#381).
    pub fn playing_of(&self, user: UserId) -> Option<Playing> {
        let conns = self.inner.conns.lock().unwrap();
        self.shown_playing(&conns, user)
    }

    /// `user`'s "Playing" as others are shown it: the latest one held, with their Steam Join (#381) if any.
    fn shown_playing(&self, conns: &HashMap<ConnId, Conn>, user: UserId) -> Option<Playing> {
        let mut p = self.inner.playing.latest(conns, user, true)?;
        p.join = self.inner.steam_join.lock().unwrap().get(&user).cloned();
        Some(p)
    }

    /// Set (or clear) `user`'s joinable Steam lobby or server (#381); whether it changed (then
    /// [`Hub::sync_presence`] tells those who see their "Playing").
    pub fn set_steam_join(&self, user: UserId, join: Option<SteamJoin>) -> bool {
        let _conns = self.inner.conns.lock().unwrap();
        let mut joins = self.inner.steam_join.lock().unwrap();
        match join {
            Some(j) => joins.insert(user, j.clone()) != Some(j),
            None => joins.remove(&user).is_some(),
        }
    }

    /// Who has a joinable Steam lobby or server now (#381).
    pub fn steam_joiners(&self) -> Vec<UserId> {
        let _conns = self.inner.conns.lock().unwrap();
        self.inner
            .steam_join
            .lock()
            .unwrap()
            .keys()
            .copied()
            .collect()
    }

    /// What connection `conn` itself holds (the same game again keeps its art without fetching it).
    pub fn playing_held(&self, conn: ConnId) -> Option<Playing> {
        let _conns = self.inner.conns.lock().unwrap();
        self.inner.playing.held(conn)
    }

    /// `user`'s [`InboxSlot`] (made on first use) while they are connected; `None` when they aren't, so a count
    /// read for someone who has just left never leaves a slot behind.
    pub fn inbox_slot(&self, user: UserId) -> Option<InboxSlot> {
        let conns = self.inner.conns.lock().unwrap();
        if !conns.values().any(|c| c.user == user) {
            return None;
        }
        Some(
            self.inner
                .inbox
                .lock()
                .unwrap()
                .entry(user)
                .or_default()
                .clone(),
        )
    }

    /// Send inbox event `event` to its owner's connections that have had their Ready count, if `slot` (held by
    /// the caller) is still theirs: after a disconnect and reconnect it isn't, and the new Ready supersedes it.
    pub(crate) fn send_inbox(&self, slot: &InboxSlot, event: Event) {
        let Some(owner) = event.only_for() else {
            return;
        };
        let mut conns = self.inner.conns.lock().unwrap();
        if !self
            .inner
            .inbox
            .lock()
            .unwrap()
            .get(&owner)
            .is_some_and(|s| Arc::ptr_eq(s, slot))
        {
            return;
        }
        if deliver_where(
            &mut conns,
            |c| c.user == owner && c.inbox,
            &ServerFrame::Event(event),
        ) {
            self.prune_inbox(&conns);
        }
    }

    /// Connection `conn`'s Ready count has been read (under its user's [`InboxSlot`]): inbox events reach it now.
    pub(crate) fn inbox_ready(&self, conn: ConnId) {
        if let Some(c) = self.inner.conns.lock().unwrap().get_mut(&conn) {
            c.inbox = true;
        }
    }

    /// Everyone connected.
    pub fn online_users(&self) -> HashSet<UserId> {
        self.inner
            .conns
            .lock()
            .unwrap()
            .values()
            .map(|c| c.user)
            .collect()
    }

    pub fn is_online(&self, user: UserId) -> bool {
        self.inner
            .conns
            .lock()
            .unwrap()
            .values()
            .any(|c| c.user == user)
    }

    /// `user` has no connection left but some of their sockets are still closing: the hub dropped them all at once
    /// (a logout, a restart) and not every one has been unregistered yet. The last of them to go records last-seen
    /// and announces them offline (#449); until then nothing about them is announced.
    pub fn is_closing(&self, user: UserId) -> bool {
        let conns = self.inner.conns.lock().unwrap();
        closing(&conns, &self.inner.open.lock().unwrap(), user)
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
        let mut conns = self.inner.conns.lock().unwrap();
        conns.retain(|_, c| {
            if c.token_hash == token_hash {
                c.kick(CLOSE_UNAUTHORIZED);
                false
            } else {
                true
            }
        });
        self.prune_inbox(&conns);
    }

    /// Close `user`'s sockets opened with any session other than `keep_hash` ("sign out all
    /// other devices"); the caller's own sockets and everyone else's stay open.
    pub fn drop_other_sessions(&self, user: UserId, keep_hash: &str) {
        let mut conns = self.inner.conns.lock().unwrap();
        conns.retain(|_, c| {
            if c.user == user && c.token_hash != keep_hash {
                c.kick(CLOSE_UNAUTHORIZED);
                false
            } else {
                true
            }
        });
        self.prune_inbox(&conns);
    }

    /// Close every socket with `code` (1012 = server restarting; clients reconnect).
    pub fn close_all(&self, code: u16) {
        let mut conns = self.inner.conns.lock().unwrap();
        for (_, mut c) in conns.drain() {
            c.kick(code);
        }
        self.prune_inbox(&conns);
    }

    /// Hold this from before a write transaction until its events are published, so events
    /// carrying absolute state (reaction counts) are delivered in commit order. Taken before
    /// `begin_write`, so waiters hold no pool connection.
    pub async fn in_order(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.inner.ordered.lock().await
    }

    /// Deliver `event` to every connection whose user is in its audience; the inbox events it implies follow
    /// ([`Hub::publish_all`]).
    pub async fn publish(&self, db: &SqlitePool, event: Event) {
        self.publish_all(db, vec![event]).await;
    }

    /// Deliver each of `events`, in order, to every connection whose user is in its audience. The inbox events
    /// they imply together (#306, `inbox::follow_ups`: one per person for the whole batch, so a quote's delete
    /// with all its replies counts each inbox once) are queued for the inbox workers, which read and send them
    /// after: never under the caller's [`Hub::in_order`] (#327). Each person's work goes to one worker (#328),
    /// which takes it in publish order, and each person's counts under their [`InboxSlot`]. Audiences are all computed before the
    /// first delivery, and inbox work is queued only after the last: see the module docs.
    pub async fn publish_all(&self, db: &SqlitePool, events: Vec<Event>) {
        let mut out = Vec::with_capacity(events.len());
        for event in events {
            match route(db, &event).await {
                Ok((a, since)) => {
                    // a member's `User` reaches the people they've blocked emptied (#508)
                    let views = match crate::blocks::member_views(db, &event).await {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::error!(error = ?e, "member_views failed; event dropped");
                            continue;
                        }
                    };
                    out.push((a, since, event, views))
                }
                Err(e) => tracing::error!(error = ?e, "audience_for failed; event dropped"),
            }
        }
        let jobs = crate::inbox::follow_ups(self, out.iter().map(|(_, _, e, _)| e));
        {
            let mut conns = self.inner.conns.lock().unwrap();
            let mut dropped = false;
            for (audience, since, event, views) in out {
                dropped |= deliver_views(&mut conns, &audience, since, event, views);
            }
            if dropped {
                self.prune_inbox(&conns);
            }
        }
        if !jobs.is_empty() {
            self.queue_inbox(db, jobs);
        }
    }

    /// Hand `jobs` to the inbox workers, each person's to theirs, starting a worker the first time (or again,
    /// logged, if it has ended). Never waits: a full queue collapses the work into that worker's overflow.
    fn queue_inbox(&self, db: &SqlitePool, jobs: Vec<crate::inbox::Job>) {
        let mut by_shard: [Vec<crate::inbox::Job>; INBOX_SHARDS] = Default::default();
        for job in jobs {
            by_shard[job.shard(INBOX_SHARDS)].push(job);
        }
        let mut queue = self.inner.inbox_queue.lock().unwrap();
        for (shard, jobs) in queue.iter_mut().zip(by_shard) {
            if !jobs.is_empty() {
                self.queue_on(shard, db, jobs);
            }
        }
    }

    fn queue_on(
        &self,
        shard: &mut Option<InboxShard>,
        db: &SqlitePool,
        jobs: Vec<crate::inbox::Job>,
    ) {
        use mpsc::error::TrySendError;
        let mut batch = (db.clone(), jobs);
        if let Some(s) = shard.as_ref() {
            match s.tx.try_send(batch) {
                Ok(()) => return,
                Err(TrySendError::Full((db, jobs))) => {
                    s.overflow.lock().unwrap().add(jobs);
                    // The worker reads its overflow after each batch. If it emptied its queue meanwhile, this
                    // empty batch wakes it; if the queue is still full, it has batches to come back from.
                    let _ = s.tx.try_send((db, Vec::new()));
                    return;
                }
                Err(TrySendError::Closed(b)) => {
                    tracing::error!("an inbox worker has ended; starting another");
                    batch = b;
                }
            }
        }
        let s = self.start_inbox_worker(INBOX_QUEUE);
        if s.tx.try_send(batch).is_err() {
            tracing::error!("couldn't queue inbox work; those events are lost (a Ready has them)");
        }
        *shard = Some(s);
    }

    /// An inbox worker: takes batches in publish order, with whatever else is already waiting (up to
    /// [`INBOX_BATCHES`], so `inbox::run` counts each person once for all of them), then any work that overflowed
    /// its queue meanwhile. It holds the hub weakly and ends with it. A batch that panics is logged and the worker
    /// goes on with the next.
    fn start_inbox_worker(&self, capacity: usize) -> InboxShard {
        let (tx, mut rx) = mpsc::channel::<InboxBatch>(capacity);
        let overflow = Arc::new(Mutex::new(crate::inbox::Overflow::default()));
        let late = overflow.clone();
        let hub = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            while let Some((db, mut jobs)) = rx.recv().await {
                // one hub, one database: the first batch's pool serves them all
                for _ in 1..INBOX_BATCHES {
                    let Ok((_, more)) = rx.try_recv() else { break };
                    jobs.extend(more);
                }
                let Some(inner) = hub.upgrade() else { break };
                let hub = Hub { inner };
                run_inbox(db.clone(), hub.clone(), jobs).await;
                let late = late.lock().unwrap().take();
                if !late.is_empty() {
                    tracing::warn!(
                        jobs = late.len(),
                        "inbox work fell behind; counts retold, the items are left to Ready"
                    );
                    run_inbox(db, hub, late).await;
                }
            }
        });
        InboxShard { tx, overflow }
    }

    /// Like [`Hub::publish`], but only to the members of `only` who are also in the event's
    /// audience, so a narrower list (people in a call) can never widen who may see it.
    pub async fn publish_to(&self, db: &SqlitePool, event: Event, only: &HashSet<UserId>) {
        let (audience, since) = match route(db, &event).await {
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
        // a member's `User` reaches the people they've blocked emptied (#508)
        let views = match crate::blocks::member_views(db, &event).await {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = ?e, "member_views failed; event dropped");
                return;
            }
        };
        let mut conns = self.inner.conns.lock().unwrap();
        if deliver_views(&mut conns, &allowed, since, event, views) {
            self.prune_inbox(&conns);
        }
    }

    /// Tell each connected viewer how `user` looks to them now (#45: per viewer, see
    /// [`view_for`]), but only those whose view changed since they were last told. Connections are
    /// read under the same lock that sends, so racing connects and disconnects can neither repeat
    /// an announcement nor reorder two of them. Returns whether `user` is connected, if anyone was
    /// told anything new about their presence. Their custom status (#152) is part of the presence view, so it reaches
    /// exactly those who see them online.
    ///
    /// Only people who share a server, a DM or a group with them see more than [`PresenceView::stranger`] (#80); call it
    /// again for both sides when that changes ([`Self::resync`]).
    ///
    /// Then what they're listening to (#275) and playing (#380), by the same rule: only to viewers they look online to
    /// (and to themselves), so going Invisible or hiding from someone takes it from them (`None`) as it takes the dot.
    /// Their game also skips anyone they hide what they're playing from (#436).
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
        // only people who share a server or a conversation with them see any of it (#80)
        let known = match crate::presence::known_to(db, user).await {
            Ok(k) => k,
            Err(e) => {
                tracing::error!(error = ?e, "presence not announced");
                return None;
            }
        };
        // Someone who comes to know them (and has no copy yet) gets their profile first (#486), read only when needed.
        let unintroduced = {
            let conns = self.inner.conns.lock().unwrap();
            let introduced = self.inner.introduced.lock().unwrap();
            conns.iter().any(|(id, c)| {
                (c.user == user || known.contains(&c.user)) && !introduced.contains(&(user, *id))
            })
        };
        let profile = if unintroduced {
            match crate::auth::routes::load_user(db, user).await {
                Ok(u) => Some(u),
                Err(e) => {
                    tracing::error!(error = ?e, "a profile not introduced");
                    None
                }
            }
        } else {
            None
        };
        // who is in each server where they've set a status of their own (#154): each viewer sees it in the ones they share
        let status_members = if subject.server_status.is_empty() {
            HashMap::new()
        } else {
            match crate::server_status::members_of(db, subject.server_status.keys().copied()).await
            {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!(error = ?e, "presence not announced");
                    return None;
                }
            }
        };
        let now = crate::db::now();
        let mut conns = self.inner.conns.lock().unwrap();
        if closing(&conns, &self.inner.open.lock().unwrap(), user) {
            // Their sockets are still closing (#449): announced now, they'd go offline with the last-seen from
            // before, and again once the last socket records it. That last one announces.
            return None;
        }
        let (connected, idle) = live(&conns, user);
        let mut views: HashMap<UserId, ServerFrame> = HashMap::new();
        let mut to: HashMap<ConnId, ServerFrame> = HashMap::new();
        let mut listening_to: HashMap<ConnId, ServerFrame> = HashMap::new();
        let mut playing_to: HashMap<ConnId, ServerFrame> = HashMap::new();
        let mut intro_to: HashMap<ConnId, ServerFrame> = HashMap::new();
        {
            let mut announced = self.inner.announced.lock().unwrap();
            let mut introduced = self.inner.introduced.lock().unwrap();
            let shown = self.inner.listening.latest(&conns, user, true);
            let game = self.shown_playing(&conns, user);
            for (id, c) in conns.iter() {
                let knows = c.user == user || known.contains(&c.user);
                if !knows {
                    // they share nothing now: a later share introduces them again, with the profile as it is then
                    introduced.remove(&(user, *id));
                } else if let Some(u) = &profile
                    && introduced.insert((user, *id))
                {
                    // someone they've blocked gets them emptied (#474)
                    let u = if subject.blocked.contains(&c.user) {
                        crate::blocks::emptied_user(u.clone())
                    } else {
                        u.clone()
                    };
                    intro_to.insert(*id, ServerFrame::Event(Event::UserCreated { user: u }));
                }
                let view = if knows {
                    view_for(&subject, c.user, connected, idle, &now)
                } else {
                    PresenceView::stranger()
                };
                let sees = c.user == user || view.online();
                let want = shown.as_ref().filter(|_| sees);
                if self.inner.listening.tell(user, *id, want) {
                    listening_to.insert(
                        *id,
                        ServerFrame::Event(Event::ListeningChanged {
                            user_id: user,
                            listening: want.cloned(),
                        }),
                    );
                }
                // hidden from them (#436): online, with no game
                let want = game
                    .as_ref()
                    .filter(|_| sees && subject.shows_game_to(c.user));
                if self.inner.playing.tell(user, *id, want) {
                    playing_to.insert(
                        *id,
                        ServerFrame::Event(Event::PlayingChanged {
                            user_id: user,
                            playing: want.cloned(),
                        }),
                    );
                }
                let in_servers = if knows {
                    crate::server_status::views_for(
                        &subject,
                        c.user,
                        &view,
                        |sv| status_members.get(&sv).is_some_and(|m| m.contains(&c.user)),
                        connected,
                        idle,
                    )
                } else {
                    vec![]
                };
                let told = (view, in_servers);
                if announced.get(&(user, *id)) == Some(&told)
                    // never told of them (#486): a stranger is nobody they have, so there's nothing to say
                    || !knows && !announced.contains_key(&(user, *id))
                {
                    continue;
                }
                let (view, in_servers) = &told;
                let frame = views
                    .entry(c.user)
                    .or_insert_with(|| {
                        ServerFrame::Event(Event::PresenceChanged {
                            user_id: user,
                            online: view.online(),
                            last_seen_at: view.last_seen_at.clone(),
                            status: view.status,
                            idle: view.idle,
                            custom_status: view.custom.clone(),
                            in_servers: in_servers.clone(),
                        })
                    })
                    .clone();
                announced.insert((user, *id), told);
                to.insert(*id, frame);
            }
        }
        // the profile first (a client ignores presence for someone it doesn't have), then the presence: a client drops
        // the activity of someone who went offline
        let dropped = send_each(&mut conns, &intro_to)
            | send_each(&mut conns, &to)
            | send_each(&mut conns, &listening_to)
            | send_each(&mut conns, &playing_to);
        if dropped {
            self.prune_inbox(&conns);
        }
        (!to.is_empty()).then_some(connected)
    }

    /// [`Self::sync_presence`] for each of `users`: after they start or stop sharing a server or a conversation (#80),
    /// so each sees the others' presence, or stops seeing it.
    pub async fn resync(&self, db: &SqlitePool, users: impl IntoIterator<Item = UserId>) {
        let users: Vec<UserId> = users.into_iter().collect();
        for user in users {
            self.sync_presence(db, user).await;
        }
    }

    /// Those of `users` who share a server, a DM or a group with `viewer` (and `viewer`), as `viewer` may see them
    /// (#486), for the Ready of `viewer`'s connection
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
        let known = crate::presence::known_to(db, viewer).await?;
        // the servers they're in: someone's own status in one (#154) shows to them there
        let mine = crate::servers::routes::server_ids_of(db, viewer)
            .await
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let now = crate::db::now();
        let conns = self.inner.conns.lock().unwrap();
        let mut announced = self.inner.announced.lock().unwrap();
        let mut introduced = self.inner.introduced.lock().unwrap();
        Ok(users
            .into_iter()
            // only people who share a server, a DM or a group with them, and themselves (#486)
            .filter(|(user, _)| user.id == viewer || known.contains(&user.id))
            .map(|(user, last_seen_at)| {
                let (connected, idle) = live(&conns, user.id);
                let view = match all.get(&user.id) {
                    // someone who shares nothing with them (#80)
                    _ if user.id != viewer && !known.contains(&user.id) => PresenceView::stranger(),
                    Some(s) => view_for(s, viewer, connected, idle, &now),
                    // Someone who registered between the two reads: no status yet.
                    None => PresenceView {
                        status: if connected {
                            pulse_protocol::rest::PresenceStatus::Online
                        } else {
                            pulse_protocol::rest::PresenceStatus::Offline
                        },
                        last_seen_at: (!connected).then_some(last_seen_at).flatten(),
                        idle: false,
                        custom: None,
                    },
                };
                let sees = user.id == viewer || view.online();
                let listening = sees
                    .then(|| self.inner.listening.latest(&conns, user.id, true))
                    .flatten();
                // hidden from them (#436): online, with no game
                let shows_game = all.get(&user.id).is_none_or(|s| s.shows_game_to(viewer));
                let playing = (sees && shows_game)
                    .then(|| self.shown_playing(&conns, user.id))
                    .flatten();
                let in_servers = match all.get(&user.id) {
                    Some(s) if user.id == viewer || known.contains(&user.id) => {
                        crate::server_status::views_for(
                            s,
                            viewer,
                            &view,
                            |sv| mine.contains(&sv),
                            connected,
                            idle,
                        )
                    }
                    _ => vec![],
                };
                // someone who blocked them shows emptied (#474)
                let user = if all
                    .get(&user.id)
                    .is_some_and(|s| s.blocked.contains(&viewer))
                {
                    crate::blocks::emptied_user(user)
                } else {
                    user
                };
                if let Some(conn) = conn {
                    announced.insert((user.id, conn), (view.clone(), in_servers.clone()));
                    introduced.insert((user.id, conn));
                    self.inner.listening.tell(user.id, conn, listening.as_ref());
                    self.inner.playing.tell(user.id, conn, playing.as_ref());
                }
                Person {
                    user,
                    online: view.online(),
                    last_seen_at: view.last_seen_at,
                    status: view.status,
                    idle: view.idle,
                    listening,
                    playing,
                    custom_status: view.custom,
                    in_servers,
                }
            })
            .collect())
    }
}

/// Queue each connection's own frame; drop connections that can't keep up (true when any was).
fn send_each(conns: &mut HashMap<ConnId, Conn>, frames: &HashMap<ConnId, ServerFrame>) -> bool {
    let before = conns.len();
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
    conns.len() < before
}

/// Queue `frame` for every connection in `audience`; drop connections that can't keep up (true when any was).
fn deliver(
    conns: &mut HashMap<ConnId, Conn>,
    audience: &super::audience::Audience,
    since: u32,
    frame: &ServerFrame,
) -> bool {
    deliver_where(
        conns,
        |c| audience.includes(c.user) && c.client_version >= since,
        frame,
    )
}

/// Queue `event` for every connection in `audience` whose client speaks protocol `since` or later, the people in
/// `views` getting their own copy instead (#508, [`crate::blocks::member_views`]); drop connections that can't keep up
/// (true when any was).
fn deliver_views(
    conns: &mut HashMap<ConnId, Conn>,
    audience: &super::audience::Audience,
    since: u32,
    event: Event,
    views: Vec<(UserId, Event)>,
) -> bool {
    if views.is_empty() {
        return deliver(conns, audience, since, &ServerFrame::Event(event));
    }
    let mut dropped = false;
    let others: HashSet<UserId> = views.iter().map(|(u, _)| *u).collect();
    for (viewer, copy) in views {
        if audience.includes(viewer) {
            dropped |= deliver_where(
                conns,
                |c| c.user == viewer && c.client_version >= since,
                &ServerFrame::Event(copy),
            );
        }
    }
    dropped
        | deliver_where(
            conns,
            |c| {
                audience.includes(c.user) && !others.contains(&c.user) && c.client_version >= since
            },
            &ServerFrame::Event(event),
        )
}

/// Queue `frame` for every connection `to` picks; drop connections that can't keep up (true when any was).
fn deliver_where(
    conns: &mut HashMap<ConnId, Conn>,
    to: impl Fn(&Conn) -> bool,
    frame: &ServerFrame,
) -> bool {
    let before = conns.len();
    let joined = matches!(frame, ServerFrame::Event(Event::ServerJoined { .. }));
    // what a client too old for builds channels gets instead (#616), worked out once
    let mut old: Option<Option<Option<ServerFrame>>> = None;
    conns.retain(|id, c| {
        if !to(c) {
            return true;
        }
        if joined && c.client_version < SERVER_JOINED_SINCE {
            // it would skip the event and miss the server it joined: its next Ready brings it (#80)
            c.kick(CLOSE_RESYNC);
            return false;
        }
        let frame = if crate::builds::compat::speaks_builds(c.client_version) {
            frame
        } else {
            match old.get_or_insert_with(|| crate::builds::compat::for_old_client(frame)) {
                None => frame,
                Some(Some(copy)) => &*copy,
                Some(None) => return true,
            }
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
    conns.len() < before
}

/// One inbox batch, on its own task so a panic is logged and the worker goes on.
async fn run_inbox(db: SqlitePool, hub: Hub, jobs: Vec<crate::inbox::Job>) {
    if jobs.is_empty() {
        return;
    }
    if let Err(e) = tokio::spawn(async move { crate::inbox::run(&db, &hub, jobs).await }).await {
        tracing::error!(error = ?e, "an inbox batch failed; its events are lost (a Ready has them)");
    }
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

    /// #327: a count read for someone who has since left leaves nothing behind, and one read before they came
    /// back (under the old slot) never reaches the new connection, nor a connection before its Ready count.
    #[test]
    fn inbox_slots_go_with_the_last_connection_and_stale_counts_stay_out() {
        let hub = Hub::default();
        let alex = UserId::new();
        let event = |unread| Event::InboxUpdated {
            user_id: alex,
            unread,
            read: vec![],
            read_upto: None,
            removed: vec![],
        };
        assert!(hub.inbox_slot(alex).is_none(), "not connected");
        let first = hub.register(alex, String::new());
        let old = hub.inbox_slot(alex).unwrap();
        hub.unregister(first.id);
        assert!(hub.inbox_slot(alex).is_none(), "no slot after they left");
        assert!(hub.inner.inbox.lock().unwrap().is_empty());
        let mut again = hub.register(alex, String::new());
        hub.send_inbox(&hub.inbox_slot(alex).unwrap(), event(1));
        assert!(again.rx.try_recv().is_err(), "not before its Ready count");
        hub.inbox_ready(again.id);
        hub.send_inbox(&old, event(2));
        assert!(again.rx.try_recv().is_err(), "nothing from the old slot");
        hub.send_inbox(&hub.inbox_slot(alex).unwrap(), event(3));
        assert!(matches!(
            again.rx.try_recv(),
            Ok(ServerFrame::Event(Event::InboxUpdated { unread: 3, .. }))
        ));
    }

    /// #327 review: a connection the hub drops itself (a kicked session, one too slow to keep up) takes its user's
    /// inbox slot with it when it was their last.
    #[test]
    fn inbox_slots_go_when_the_hub_drops_the_last_connection() {
        let hub = Hub::default();
        let (alex, sam) = (UserId::new(), UserId::new());
        let _a = hub.register(alex, "a".into());
        let s = hub.register(sam, "s".into());
        hub.inbox_slot(alex).unwrap();
        hub.inbox_slot(sam).unwrap();
        hub.drop_session("a");
        assert!(
            !hub.inner.inbox.lock().unwrap().contains_key(&alex),
            "kicked"
        );
        hub.drop_other_sessions(sam, "keep");
        assert!(hub.inner.inbox.lock().unwrap().is_empty(), "kicked too");

        let kim = UserId::new();
        let slow = hub.register(kim, String::new());
        let slot = hub.inbox_slot(kim).unwrap();
        hub.inbox_ready(slow.id);
        let event = Event::InboxUpdated {
            user_id: kim,
            unread: 1,
            read: vec![],
            read_upto: None,
            removed: vec![],
        };
        // never read: its queue fills, then the next frame drops it
        for _ in 0..=QUEUE {
            hub.send_inbox(&slot, event.clone());
        }
        assert_eq!(hub.connections_for(kim), 0, "dropped as too slow");
        assert!(hub.inner.inbox.lock().unwrap().is_empty(), "too slow");
        drop(s);
    }

    /// #327 review: inbox work queued after the worker ended starts another, rather than being lost for good.
    #[tokio::test]
    async fn the_inbox_worker_starts_again_if_it_has_ended() {
        let hub = Hub::default();
        let user = UserId::new();
        let i = crate::inbox::Job::Recount { user }.shard(INBOX_SHARDS);
        let (dead, rx) = mpsc::channel(1);
        drop(rx);
        hub.inner.inbox_queue.lock().unwrap()[i] = Some(InboxShard {
            tx: dead,
            overflow: Arc::default(),
        });
        let db = SqlitePool::connect_lazy("sqlite::memory:").unwrap();
        hub.queue_inbox(&db, vec![crate::inbox::Job::Recount { user }]);
        let queue = hub.inner.inbox_queue.lock().unwrap();
        assert!(
            !queue[i].as_ref().unwrap().tx.is_closed(),
            "a live worker again"
        );
    }

    /// #328: each person's inbox work always goes to the same worker, and the work spreads over them.
    #[test]
    fn a_persons_inbox_work_stays_on_one_worker() {
        use crate::inbox::Job;
        let mut used = HashSet::new();
        for _ in 0..64 {
            let user = UserId::new();
            let i = Job::Recount { user }.shard(INBOX_SHARDS);
            assert!(i < INBOX_SHARDS);
            assert_eq!(Job::Retell { user }.shard(INBOX_SHARDS), i);
            assert_eq!(Job::Removed { user, ids: vec![] }.shard(INBOX_SHARDS), i);
            used.insert(i);
        }
        assert_eq!(used.len(), INBOX_SHARDS, "64 people use every worker");
    }

    /// #328: a full queue never makes the publisher wait: the work is collapsed into the worker's overflow (each
    /// person's count retold once), and nothing more queues behind it.
    #[tokio::test]
    async fn a_full_inbox_queue_collapses_into_the_overflow() {
        use crate::inbox::Job;
        let hub = Hub::default();
        let alex = UserId::new();
        // someone on the same worker
        let sam = std::iter::repeat_with(UserId::new)
            .find(|u| {
                Job::Recount { user: *u }.shard(INBOX_SHARDS)
                    == Job::Recount { user: alex }.shard(INBOX_SHARDS)
            })
            .unwrap();
        let i = Job::Recount { user: alex }.shard(INBOX_SHARDS);
        // a worker that never reads, with room for one batch
        let (tx, mut rx) = mpsc::channel(1);
        let overflow = Arc::new(Mutex::new(crate::inbox::Overflow::default()));
        hub.inner.inbox_queue.lock().unwrap()[i] = Some(InboxShard {
            tx,
            overflow: overflow.clone(),
        });
        let db = SqlitePool::connect_lazy("sqlite::memory:").unwrap();
        hub.queue_inbox(&db, vec![Job::Recount { user: alex }]);
        for _ in 0..3 {
            hub.queue_inbox(
                &db,
                vec![
                    Job::Recount { user: sam },
                    Job::Removed {
                        user: alex,
                        ids: vec![],
                    },
                ],
            );
        }
        let (_, first) = rx.try_recv().unwrap();
        assert!(matches!(first[..], [Job::Recount { user }] if user == alex));
        assert!(rx.try_recv().is_err(), "the rest didn't queue");
        let mut late: Vec<UserId> = overflow
            .lock()
            .unwrap()
            .take()
            .into_iter()
            .map(|j| match j {
                Job::Retell { user } => user,
                _ => panic!("a person's work collapses to a retell"),
            })
            .collect();
        late.sort();
        let mut want = vec![alex, sam];
        want.sort();
        assert_eq!(late, want, "once each");
    }

    /// #328: the worker does the work that overflowed, after its batch.
    #[tokio::test]
    async fn the_inbox_worker_takes_its_overflow() {
        let hub = Hub::default();
        let shard = hub.start_inbox_worker(4);
        shard
            .overflow
            .lock()
            .unwrap()
            .add(vec![crate::inbox::Job::Recount {
                user: UserId::new(),
            }]);
        let db = SqlitePool::connect_lazy("sqlite::memory:").unwrap();
        shard.tx.try_send((db, Vec::new())).unwrap();
        for _ in 0..500 {
            if shard.overflow.lock().unwrap().is_empty() && shard.tx.capacity() == 4 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        panic!("the overflow wasn't taken");
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
