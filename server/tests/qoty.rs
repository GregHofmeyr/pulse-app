//! Quote of the Year: events, the top-5 snapshot, public votes, endings, winner cards, reopening.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId, QotyEventId, ServerId, UserId};
use pulse_protocol::rest::{Channel, Message, MessageKind, QotyEndReason, QotyEvent};
use reqwest::Method;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello_ready(app: &TestApp, token: &str) -> (Socket, Ready) {
    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        match next_frame(&mut ws).await {
            Some(ServerFrame::Ready(r)) => return (ws, r),
            Some(_) => continue,
            None => panic!("expected Ready"),
        }
    }
}

async fn hello(app: &TestApp, token: &str) -> Socket {
    hello_ready(app, token).await.0
}

async fn next_frame(ws: &mut Socket) -> Option<ServerFrame> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(2), ws.next())
            .await
            .ok()??
            .ok()?;
        if let Ws::Text(t) = m {
            return Some(serde_json::from_str(&t).unwrap());
        }
    }
}

/// What the socket says about Quote of the Year and cards until it goes quiet, in order.
#[derive(Debug)]
enum Seen {
    Started(QotyEvent),
    Updated(QotyEvent),
    Ended(QotyEvent),
    Card(Box<Message>),
    Deleted(MessageId),
}

async fn seen(ws: &mut Socket) -> Vec<Seen> {
    let mut out = Vec::new();
    while let Some(f) = next_frame(ws).await {
        match f {
            ServerFrame::Event(Event::QotyStarted { event }) => out.push(Seen::Started(event)),
            ServerFrame::Event(Event::QotyUpdated { event }) => out.push(Seen::Updated(event)),
            ServerFrame::Event(Event::QotyEnded { event }) => out.push(Seen::Ended(event)),
            ServerFrame::Event(Event::MessageCreated { message, .. })
                if message.kind == MessageKind::System =>
            {
                out.push(Seen::Card(Box::new(message)))
            }
            ServerFrame::Event(Event::MessageDeleted { message_id, .. }) => {
                out.push(Seen::Deleted(message_id))
            }
            _ => {}
        }
    }
    out
}

fn this_year() -> i32 {
    chrono::Datelike::year(&chrono::Utc::now())
}

struct Fixture {
    app: TestApp,
    alex: UserId,
    alex_t: String,
    sam: UserId,
    sam_t: String,
    server: ServerId,
    ch: Channel,
}

async fn create_quotes(app: &TestApp, token: &str, server: ServerId, name: &str) -> Channel {
    let r = post_json(
        app,
        token,
        &format!("/servers/{server}/channels"),
        json!({ "kind": "quotes", "name": name }),
    )
    .await;
    assert_eq!(r.status(), 200, "create quotes channel");
    r.json().await.unwrap()
}

/// alex and sam in one server with a quotes channel.
async fn fixture() -> Fixture {
    let app = spawn().await;
    let (alex, alex_t) = register(&app, "alex").await;
    let (sam, sam_t) = register(&app, "sam").await;
    let s = create_server(&app, &alex_t, "Main").await;
    let r = post_json(&app, &sam_t, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let ch = create_quotes(&app, &alex_t, s.id, "quotes").await;
    Fixture {
        app,
        alex,
        alex_t,
        sam,
        sam_t,
        server: s.id,
        ch,
    }
}

impl Fixture {
    /// Another member of the server.
    async fn member(&self, name: &str) -> (UserId, String) {
        let (id, t) = register(&self.app, name).await;
        let r = post_json(
            &self.app,
            &t,
            &format!("/servers/{}/join", self.server),
            json!({}),
        )
        .await;
        assert_eq!(r.status(), 204);
        (id, t)
    }
    async fn call(
        &self,
        method: Method,
        token: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> reqwest::Response {
        let mut r = self
            .app
            .http
            .request(method, self.app.url(path))
            .bearer_auth(token);
        if let Some(b) = body {
            r = r.json(&b);
        }
        r.send().await.unwrap()
    }
    /// A quote alex posts in `ch`: sam said `text`.
    async fn quote_in(&self, ch: ChannelId, text: &str) -> Message {
        let r = post_json(
            &self.app,
            &self.alex_t,
            &format!("/channels/{ch}/quotes"),
            json!({ "lines": [{ "speaker_id": self.sam, "text": text }] }),
        )
        .await;
        assert_eq!(r.status(), 200, "post quote");
        r.json().await.unwrap()
    }
    async fn quote(&self, text: &str) -> Message {
        self.quote_in(self.ch.id, text).await
    }
    async fn sql(&self, q: &str) {
        sqlx::query(q).execute(&self.app.db).await.unwrap();
    }
    /// Give `m` a reaction score of `n`: one reaction from each of `n` registered users.
    async fn score(&self, m: MessageId, fans: &[UserId], n: usize) {
        for f in &fans[..n] {
            self.sql(&format!(
                "INSERT INTO reactions (message_id, user_id, emoji, created_at)
                 VALUES ('{m}', '{f}', '😂', '2026-01-01T00:00:00.000Z')"
            ))
            .await;
        }
    }
    /// `n` registered users (not members), for reactions.
    async fn fans(&self, n: usize) -> Vec<UserId> {
        let mut out = Vec::new();
        for i in 0..n {
            out.push(register(&self.app, &format!("fan{i}")).await.0);
        }
        out
    }
    async fn start(&self, token: &str, year: i32, minutes: Option<u32>) -> reqwest::Response {
        self.call(
            Method::POST,
            token,
            &format!("/servers/{}/qoty", self.server),
            Some(json!({ "year": year, "minutes": minutes })),
        )
        .await
    }
    /// alex starts this year's vote, open until ended.
    async fn started(&self) -> QotyEvent {
        let r = self.start(&self.alex_t, this_year(), None).await;
        assert_eq!(r.status(), 200, "start");
        r.json().await.unwrap()
    }
    async fn vote(&self, token: &str, e: QotyEventId, m: MessageId) -> reqwest::Response {
        self.call(
            Method::PUT,
            token,
            &format!("/qoty/{e}/vote"),
            Some(json!({ "message_id": m })),
        )
        .await
    }
    async fn voted(&self, token: &str, e: QotyEventId, m: MessageId) -> QotyEvent {
        let r = self.vote(token, e, m).await;
        assert_eq!(r.status(), 200, "vote");
        r.json().await.unwrap()
    }
    async fn unvote(&self, token: &str, e: QotyEventId) -> reqwest::Response {
        self.call(Method::DELETE, token, &format!("/qoty/{e}/vote"), None)
            .await
    }
    async fn add(&self, token: &str, e: QotyEventId, m: MessageId) -> reqwest::Response {
        self.call(
            Method::POST,
            token,
            &format!("/qoty/{e}/options"),
            Some(json!({ "message_id": m })),
        )
        .await
    }
    async fn end(&self, token: &str, e: QotyEventId) -> reqwest::Response {
        self.call(Method::POST, token, &format!("/qoty/{e}/end"), None)
            .await
    }
    async fn ended(&self, token: &str, e: QotyEventId) -> QotyEvent {
        let r = self.end(token, e).await;
        assert_eq!(r.status(), 200, "end");
        r.json().await.unwrap()
    }
    async fn reopen(&self, token: &str, e: QotyEventId, minutes: Option<u32>) -> reqwest::Response {
        self.call(
            Method::POST,
            token,
            &format!("/qoty/{e}/reopen"),
            Some(json!({ "minutes": minutes })),
        )
        .await
    }
    /// The system messages of `ch`'s main list, oldest first.
    async fn cards(&self, ch: ChannelId) -> Vec<Message> {
        let page: Vec<Message> =
            get_json(&self.app, &self.alex_t, &format!("/channels/{ch}/messages")).await;
        let mut out: Vec<Message> = page
            .into_iter()
            .filter(|m| m.kind == MessageKind::System)
            .collect();
        out.reverse();
        out
    }
    async fn count(&self, q: &str) -> i64 {
        sqlx::query_scalar(q).fetch_one(&self.app.db).await.unwrap()
    }
    async fn edit(&self, m: &Message, lines: serde_json::Value) -> u16 {
        self.call(
            Method::PUT,
            &self.alex_t,
            &format!("/quotes/{}", m.id),
            Some(json!({ "lines": lines })),
        )
        .await
        .status()
        .as_u16()
    }
}

fn option_ids(e: &QotyEvent) -> Vec<MessageId> {
    e.options.iter().map(|o| o.message.id).collect()
}

/// Each option's voters, in the event's option order.
fn voters(e: &QotyEvent) -> Vec<Vec<UserId>> {
    e.options.iter().map(|o| o.voters.clone()).collect()
}

async fn message_of(r: reqwest::Response) -> String {
    r.json::<pulse_protocol::rest::ApiError>()
        .await
        .unwrap()
        .message
}

// ---------- starting ----------

#[tokio::test]
async fn top_five_with_ties_at_fifth() {
    let f = fixture().await;
    let fans = f.fans(5).await;
    let mut quotes = Vec::new();
    for (text, score) in [
        ("five", 5),
        ("four", 4),
        ("three", 3),
        ("two", 2),
        ("one a", 1),
        ("one b", 1),
        ("zero", 0),
    ] {
        let q = f.quote(text).await;
        f.score(q.id, &fans, score).await;
        quotes.push(q.id);
    }
    let e = f.started().await;
    let ids = option_ids(&e);
    assert_eq!(ids.len(), 6, "ties at fifth all go in; the 0 stays out");
    assert_eq!(ids[..4], quotes[..4], "best first");
    let mut tied = ids[4..].to_vec();
    tied.sort();
    let mut want = quotes[4..6].to_vec();
    want.sort();
    assert_eq!(tied, want);
    for o in &e.options {
        assert_eq!((o.added_by, o.votes, o.voters.len()), (None, 0, 0));
        assert!(o.message.quote.is_some(), "options carry the quote");
    }
    assert_eq!(e.year, this_year());
    assert_eq!(e.started_by, f.alex);
    assert_eq!((e.ends_at.as_deref(), e.ended_at.as_deref()), (None, None));
    assert_eq!((e.voters, e.members), (0, 2));
}

#[tokio::test]
async fn fewer_quotes_fewer_options() {
    let f = fixture().await;
    let fans = f.fans(3).await;
    let a = f.quote("a").await;
    let b = f.quote("b").await;
    let last_year = f.quote("last year").await;
    let deleted = f.quote("deleted").await;
    let c = f.quote("c").await;
    for q in [&a, &b, &last_year, &deleted] {
        f.score(q.id, &fans, 3).await;
    }
    f.score(c.id, &fans, 1).await;
    f.sql(&format!(
        "UPDATE quotes SET year = {} WHERE message_id = '{}'",
        this_year() - 1,
        last_year.id
    ))
    .await;
    let r = f
        .call(
            Method::DELETE,
            &f.alex_t,
            &format!("/messages/{}", deleted.id),
            None,
        )
        .await;
    assert_eq!(r.status(), 204);
    let e = f.started().await;
    let mut ids = option_ids(&e);
    ids.sort();
    let mut want = vec![a.id, b.id, c.id];
    want.sort();
    assert_eq!(ids, want, "only this year's live quotes with a score");
}

#[tokio::test]
async fn zero_score_quotes_are_never_automatic() {
    let f = fixture().await;
    let q = f.quote("quiet").await;
    let e = f.started().await;
    assert!(
        e.options.is_empty(),
        "a quiet year has no automatic options"
    );
    let r = f.add(&f.sam_t, e.id, q.id).await;
    assert_eq!(r.status(), 200);
    let e: QotyEvent = r.json().await.unwrap();
    assert_eq!(option_ids(&e), vec![q.id]);
    assert_eq!(e.options[0].added_by, Some(f.sam));
}

/// A finished event for `year` that `winner` won (seeded straight into the database).
async fn seed_win(f: &Fixture, year: i32, winner: MessageId) {
    let event = QotyEventId::new();
    f.sql(&format!(
        "INSERT INTO qoty_events (id, server_id, year, started_by, started_at, ended_at, end_reason)
         VALUES ('{event}', '{}', {year}, '{}', '2025-01-01T00:00:00.000Z', '2025-01-02T00:00:00.000Z', 'time')",
        f.server, f.alex
    ))
    .await;
    f.sql(&format!(
        "INSERT INTO qoty_options (event_id, message_id) VALUES ('{event}', '{winner}')"
    ))
    .await;
    f.sql(&format!(
        "INSERT INTO qoty_winners (event_id, message_id, votes) VALUES ('{event}', '{winner}', 1)"
    ))
    .await;
}

#[tokio::test]
async fn previous_winners_are_not_options() {
    let f = fixture().await;
    let fans = f.fans(3).await;
    let won = f.quote("won before").await;
    let other = f.quote("other").await;
    f.score(won.id, &fans, 3).await;
    f.score(other.id, &fans, 1).await;
    seed_win(&f, this_year() - 1, won.id).await;
    let e = f.started().await;
    assert_eq!(option_ids(&e), vec![other.id]);
    let r = f.add(&f.alex_t, e.id, won.id).await;
    assert_eq!(r.status(), 409, "a previous winner can't be added");
}

#[tokio::test]
async fn one_live_event_per_server() {
    let f = fixture().await;
    f.started().await;
    let r = f.start(&f.sam_t, this_year() - 1, None).await;
    assert_eq!(r.status(), 409);
    // Another server is free to start its own.
    let other = create_server(&f.app, &f.sam_t, "Other").await;
    let r = f
        .call(
            Method::POST,
            &f.sam_t,
            &format!("/servers/{}/qoty", other.id),
            Some(json!({ "year": this_year(), "minutes": null })),
        )
        .await;
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn one_event_per_year() {
    let f = fixture().await;
    let e = f.started().await;
    f.ended(&f.alex_t, e.id).await;
    let r = f.start(&f.sam_t, this_year(), Some(60)).await;
    assert_eq!(r.status(), 409);
    assert_eq!(
        message_of(r).await,
        format!("Quote of the Year {} has already happened.", this_year())
    );
    // Another year is fine.
    let r = f.start(&f.sam_t, this_year() - 1, Some(60)).await;
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn start_input_is_checked() {
    let f = fixture().await;
    for (year, minutes) in [
        (1999, None),
        (this_year() + 1, None),
        (this_year(), Some(4)),
        (this_year(), Some(20161)),
    ] {
        let r = f.start(&f.alex_t, year, minutes).await;
        assert_eq!(r.status(), 400, "{year} {minutes:?}");
    }
    assert_eq!(f.count("SELECT COUNT(*) FROM qoty_events").await, 0);
    let r = f.start(&f.alex_t, 2000, Some(20160)).await;
    assert_eq!(r.status(), 200);
    let e: QotyEvent = r.json().await.unwrap();
    let ends = chrono::DateTime::parse_from_rfc3339(e.ends_at.as_deref().unwrap()).unwrap();
    let started = chrono::DateTime::parse_from_rfc3339(&e.started_at).unwrap();
    assert_eq!((ends - started).num_minutes(), 20160);
}

// ---------- voting ----------

/// Two options, `a` then `b`, and a live event.
async fn two_options(f: &Fixture) -> (QotyEvent, MessageId, MessageId) {
    let fans = f.fans(2).await;
    let a = f.quote("a").await;
    let b = f.quote("b").await;
    f.score(a.id, &fans, 2).await;
    f.score(b.id, &fans, 1).await;
    let e = f.started().await;
    assert_eq!(option_ids(&e), vec![a.id, b.id]);
    (e, a.id, b.id)
}

#[tokio::test]
async fn votes_move_and_remove() {
    let f = fixture().await;
    let (kim, kim_t) = f.member("kim").await;
    let (e, a, b) = two_options(&f).await;
    let mut ws = hello(&f.app, &kim_t).await;

    f.voted(&f.alex_t, e.id, a).await;
    f.voted(&f.sam_t, e.id, a).await;
    let moved = f.voted(&f.alex_t, e.id, b).await;
    assert_eq!(voters(&moved), vec![vec![f.sam], vec![f.alex]]);
    // Same vote again: nothing changes.
    let same = f.voted(&f.alex_t, e.id, b).await;
    assert_eq!(voters(&same), voters(&moved));
    // Back to a: alex goes to the end of a's list.
    f.voted(&f.alex_t, e.id, a).await;
    f.voted(&kim_t, e.id, b).await;
    let r = f.unvote(&f.sam_t, e.id).await;
    assert_eq!(r.status(), 200);
    // Removing a vote you don't have changes nothing.
    assert_eq!(f.unvote(&f.sam_t, e.id).await.status(), 200);

    /// (each option's voters, each option's votes, voters, members)
    type Update = (Vec<Vec<UserId>>, Vec<u32>, u32, u32);
    let updates: Vec<Update> = seen(&mut ws)
        .await
        .into_iter()
        .map(|s| match s {
            Seen::Updated(e) => (
                voters(&e),
                e.options.iter().map(|o| o.votes).collect(),
                e.voters,
                e.members,
            ),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        updates,
        vec![
            (vec![vec![f.alex], vec![]], vec![1, 0], 1, 3),
            (vec![vec![f.alex, f.sam], vec![]], vec![2, 0], 2, 3),
            (vec![vec![f.sam], vec![f.alex]], vec![1, 1], 2, 3),
            (vec![vec![f.sam, f.alex], vec![]], vec![2, 0], 2, 3),
            (vec![vec![f.sam, f.alex], vec![kim]], vec![2, 1], 3, 3),
            (vec![vec![f.alex], vec![kim]], vec![1, 1], 2, 3),
        ],
        "kim sees every change, with who voted in vote order"
    );
}

#[tokio::test]
async fn votes_only_go_to_options_of_a_live_event() {
    let f = fixture().await;
    let (e, a, _) = two_options(&f).await;
    let not_an_option = f.quote("c").await;
    assert_eq!(
        f.vote(&f.alex_t, e.id, not_an_option.id).await.status(),
        400
    );
    assert_eq!(f.vote(&f.alex_t, QotyEventId::new(), a).await.status(), 404);
    f.ended(&f.alex_t, e.id).await;
    assert_eq!(f.vote(&f.alex_t, e.id, a).await.status(), 409);
    assert_eq!(f.unvote(&f.alex_t, e.id).await.status(), 409);
    assert_eq!(f.add(&f.alex_t, e.id, not_an_option.id).await.status(), 409);
}

#[tokio::test]
async fn self_votes_are_allowed() {
    let f = fixture().await;
    let (e, a, _) = two_options(&f).await;
    // sam said `a`.
    let e2 = f.voted(&f.sam_t, e.id, a).await;
    assert_eq!(e2.options[0].voters, vec![f.sam]);
}

#[tokio::test]
async fn adding_any_years_quote() {
    let f = fixture().await;
    let e = f.started().await;
    let old = f.quote("from december").await;
    f.sql(&format!(
        "UPDATE quotes SET year = {} WHERE message_id = '{}'",
        this_year() - 1,
        old.id
    ))
    .await;
    let r = f.add(&f.sam_t, e.id, old.id).await;
    assert_eq!(r.status(), 200);
    let e2: QotyEvent = r.json().await.unwrap();
    assert_eq!(option_ids(&e2), vec![old.id]);
    assert_eq!(e2.options[0].added_by, Some(f.sam));
    assert_eq!(
        f.add(&f.alex_t, e.id, old.id).await.status(),
        409,
        "already in"
    );

    // Only live quotes of this server.
    let deleted = f.quote("gone").await;
    f.call(
        Method::DELETE,
        &f.alex_t,
        &format!("/messages/{}", deleted.id),
        None,
    )
    .await;
    let other = create_server(&f.app, &f.alex_t, "Other").await;
    let other_ch = create_quotes(&f.app, &f.alex_t, other.id, "quotes").await;
    let r = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/quotes", other_ch.id),
        json!({ "lines": [{ "speaker_name": "A Friend", "text": "elsewhere" }] }),
    )
    .await;
    let elsewhere: Message = r.json().await.unwrap();
    let reply: Message = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/messages", f.ch.id),
        json!({ "content": "a reply", "thread_root_id": old.id }),
    )
    .await
    .json()
    .await
    .unwrap();
    for m in [deleted.id, elsewhere.id, reply.id, MessageId::new()] {
        assert_eq!(f.add(&f.alex_t, e.id, m).await.status(), 400, "{m}");
    }
}

#[tokio::test]
async fn concurrent_votes_end_on_the_true_count() {
    let f = fixture().await;
    let (e, a, _) = two_options(&f).await;
    let mut tokens = vec![f.alex_t.clone(), f.sam_t.clone()];
    for i in 0..6 {
        tokens.push(f.member(&format!("voter{i}")).await.1);
    }
    let mut ws = hello(&f.app, &f.alex_t).await;
    let mut tasks = Vec::new();
    for t in tokens.clone() {
        let (http, url) = (
            f.app.http.clone(),
            f.app.url(&format!("/qoty/{}/vote", e.id)),
        );
        tasks.push(tokio::spawn(async move {
            http.put(url)
                .bearer_auth(t)
                .json(&json!({ "message_id": a }))
                .send()
                .await
                .unwrap()
                .status()
        }));
    }
    for t in tasks {
        assert_eq!(t.await.unwrap(), 200);
    }
    let counts: Vec<u32> = seen(&mut ws)
        .await
        .into_iter()
        .filter_map(|s| match s {
            Seen::Updated(e) => Some(e.options[0].votes),
            _ => None,
        })
        .collect();
    assert_eq!(counts, (1..=8).collect::<Vec<u32>>(), "in commit order");
}

#[tokio::test]
async fn non_members_cant_take_part() {
    let f = fixture().await;
    let (_, out_t) = register(&f.app, "outsider").await;
    let (e, a, _) = two_options(&f).await;
    let c = f.quote("c").await;
    assert_eq!(f.start(&out_t, this_year() - 1, None).await.status(), 403);
    assert_eq!(f.vote(&out_t, e.id, a).await.status(), 403);
    assert_eq!(f.unvote(&out_t, e.id).await.status(), 403);
    assert_eq!(f.add(&out_t, e.id, c.id).await.status(), 403);
    assert_eq!(f.end(&out_t, e.id).await.status(), 403);
    f.ended(&f.alex_t, e.id).await;
    assert_eq!(f.reopen(&out_t, e.id, None).await.status(), 403);
}

// ---------- ending ----------

#[tokio::test]
async fn ending_early_by_anyone() {
    let f = fixture().await;
    let (kim, kim_t) = f.member("kim").await;
    let (e, a, _) = two_options(&f).await;
    f.voted(&kim_t, e.id, a).await;
    f.voted(&f.sam_t, e.id, a).await;
    let before = unread(&f, &[f.alex, f.sam, kim]).await;
    let ended = f.ended(&kim_t, e.id).await;
    assert_eq!(ended.end_reason, Some(QotyEndReason::Early));
    assert_eq!(ended.ended_by, Some(kim));
    let at = chrono::DateTime::parse_from_rfc3339(ended.ended_at.as_deref().unwrap()).unwrap();
    let until =
        chrono::DateTime::parse_from_rfc3339(ended.reopen_until.as_deref().unwrap()).unwrap();
    assert_eq!((until - at).num_hours(), 24);
    assert_eq!(f.end(&f.alex_t, e.id).await.status(), 409, "already ended");

    let cards = f.cards(f.ch.id).await;
    assert_eq!(cards.len(), 1);
    let card = cards[0].qoty_card.clone().expect("a winner card");
    assert_eq!(card.event_id, e.id);
    assert_eq!(card.year, this_year());
    assert_eq!(card.message_id, Some(a));
    assert_eq!((card.votes, card.voters), (2, vec![kim, f.sam]));
    assert_eq!(card.reason, QotyEndReason::Early);
    assert_eq!(cards[0].author_id, None);

    assert_eq!(
        f.count(&format!(
            "SELECT votes FROM qoty_winners WHERE event_id = '{}' AND message_id = '{a}'",
            e.id
        ))
        .await,
        2
    );
    // The quote now carries its win.
    let page: Vec<Message> = get_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/messages", f.ch.id),
    )
    .await;
    let q = page.iter().find(|m| m.id == a).unwrap();
    assert_eq!(q.quote.as_ref().unwrap().wins, vec![this_year()]);
    // The card counts as unread for nobody.
    assert_eq!(unread(&f, &[f.alex, f.sam, kim]).await, before);
}

/// Each user's unread count in the quotes channel.
async fn unread(f: &Fixture, users: &[UserId]) -> Vec<u32> {
    let mut out = Vec::new();
    for user in users {
        let states = pulse_server::reads::states_for(&f.app.db, *user)
            .await
            .unwrap();
        out.push(
            states
                .iter()
                .find(|s| s.channel_id == f.ch.id)
                .unwrap()
                .unread,
        );
    }
    out
}

#[tokio::test]
async fn co_winners_get_a_card_each() {
    let f = fixture().await;
    let ch2 = create_quotes(&f.app, &f.alex_t, f.server, "more-quotes").await;
    let (_, kim_t) = f.member("kim").await;
    let e = f.started().await;
    let a = f.quote("a").await;
    let c = f.quote("c").await;
    let b = f.quote_in(ch2.id, "b").await;
    let loser = f.quote("loser").await;
    for m in [&a, &b, &c, &loser] {
        assert_eq!(f.add(&f.alex_t, e.id, m.id).await.status(), 200);
    }
    f.voted(&f.alex_t, e.id, a.id).await;
    f.voted(&f.sam_t, e.id, b.id).await;
    f.voted(&kim_t, e.id, c.id).await;
    f.ended(&f.alex_t, e.id).await;

    let mut in_ch: Vec<MessageId> = f
        .cards(f.ch.id)
        .await
        .iter()
        .map(|m| m.qoty_card.clone().unwrap().message_id.unwrap())
        .collect();
    in_ch.sort();
    let mut want = vec![a.id, c.id];
    want.sort();
    assert_eq!(in_ch, want, "two winners in one channel: two cards");
    let in_ch2: Vec<MessageId> = f
        .cards(ch2.id)
        .await
        .iter()
        .map(|m| m.qoty_card.clone().unwrap().message_id.unwrap())
        .collect();
    assert_eq!(in_ch2, vec![b.id]);
    assert_eq!(
        f.count(&format!(
            "SELECT COUNT(*) FROM qoty_winners WHERE event_id = '{}'",
            e.id
        ))
        .await,
        3
    );
}

#[tokio::test]
async fn no_votes_says_so() {
    let f = fixture().await;
    let ch2 = create_quotes(&f.app, &f.alex_t, f.server, "more-quotes").await;
    let (e, _, _) = two_options(&f).await;
    f.ended(&f.sam_t, e.id).await;
    for ch in [f.ch.id, ch2.id] {
        let cards = f.cards(ch).await;
        assert_eq!(cards.len(), 1, "one line in each quotes channel");
        let card = cards[0].qoty_card.clone().unwrap();
        assert_eq!(
            (card.message_id, card.votes, card.voters.len()),
            (None, 0, 0)
        );
        assert!(!cards[0].content.is_empty());
    }
    let general = general(&f.app, &f.alex_t, f.server).await;
    assert!(f.cards(general.id).await.is_empty());
    assert_eq!(f.count("SELECT COUNT(*) FROM qoty_winners").await, 0);
}

#[tokio::test]
async fn end_by_time_on_boot() {
    let f = fixture().await;
    let (e, a, _) = two_options_timed(&f).await;
    f.voted(&f.sam_t, e.id, a).await;
    let s = f.app.state();
    let now = pulse_server::db::now();
    assert_eq!(pulse_server::qoty::end_overdue(&s, &now).await.unwrap(), 0);
    f.sql(&format!(
        "UPDATE qoty_events SET ends_at = '2000-01-01T00:00:00.000Z' WHERE id = '{}'",
        e.id
    ))
    .await;
    let mut ws = hello(&f.app, &f.alex_t).await;
    assert_eq!(pulse_server::qoty::end_overdue(&s, &now).await.unwrap(), 1);
    assert_eq!(pulse_server::qoty::end_overdue(&s, &now).await.unwrap(), 0);
    let ended = seen(&mut ws)
        .await
        .into_iter()
        .find_map(|s| match s {
            Seen::Ended(e) => Some(e),
            _ => None,
        })
        .expect("QotyEnded");
    assert_eq!(ended.end_reason, Some(QotyEndReason::Time));
    assert_eq!((ended.ended_by, ended.reopen_until), (None, None));
    let card = f.cards(f.ch.id).await[0].qoty_card.clone().unwrap();
    assert_eq!(card.reason, QotyEndReason::Time);
    assert_eq!(f.reopen(&f.alex_t, e.id, None).await.status(), 409);
}

/// Like [`two_options`], with an hour-long event.
async fn two_options_timed(f: &Fixture) -> (QotyEvent, MessageId, MessageId) {
    let fans = f.fans(2).await;
    let a = f.quote("a").await;
    let b = f.quote("b").await;
    f.score(a.id, &fans, 2).await;
    f.score(b.id, &fans, 1).await;
    let r = f.start(&f.alex_t, this_year(), Some(60)).await;
    assert_eq!(r.status(), 200);
    let e: QotyEvent = r.json().await.unwrap();
    (e, a.id, b.id)
}

#[tokio::test]
async fn ending_races_post_one_card() {
    let f = fixture().await;
    let (e, a, _) = two_options_timed(&f).await;
    f.voted(&f.sam_t, e.id, a).await;
    f.sql(&format!(
        "UPDATE qoty_events SET ends_at = '2000-01-01T00:00:00.000Z' WHERE id = '{}'",
        e.id
    ))
    .await;
    let mut ws = hello(&f.app, &f.alex_t).await;
    let s = f.app.state();
    let now = pulse_server::db::now();
    let (x, y, r) = tokio::join!(
        pulse_server::qoty::end_overdue(&s, &now),
        pulse_server::qoty::end_overdue(&s, &now),
        f.end(&f.sam_t, e.id),
    );
    let swept = x.unwrap() + y.unwrap();
    let by_hand = r.status() == 200;
    assert_eq!(swept + u32::from(by_hand), 1, "exactly one ending");
    assert_eq!(f.cards(f.ch.id).await.len(), 1);
    let ends = seen(&mut ws)
        .await
        .into_iter()
        .filter(|s| matches!(s, Seen::Ended(_)))
        .count();
    assert_eq!(ends, 1);
}

#[tokio::test]
async fn reopen_within_a_day_of_an_early_end() {
    let f = fixture().await;
    let (kim, kim_t) = f.member("kim").await;
    let (e, a, b) = two_options(&f).await;
    f.voted(&f.sam_t, e.id, a).await;
    f.voted(&kim_t, e.id, b).await;
    f.voted(&f.alex_t, e.id, a).await;
    f.ended(&f.alex_t, e.id).await;
    let card = f.cards(f.ch.id).await[0].id;

    let r = f.reopen(&kim_t, e.id, Some(60)).await;
    assert_eq!(r.status(), 200);
    let back: QotyEvent = r.json().await.unwrap();
    assert_eq!(
        (back.ended_at.as_deref(), back.end_reason, back.ended_by),
        (None, None, None)
    );
    assert!(back.ends_at.is_some());
    assert_eq!(option_ids(&back), vec![a, b], "options kept");
    assert_eq!(
        voters(&back),
        vec![vec![f.sam, f.alex], vec![kim]],
        "votes kept"
    );
    assert_eq!(
        f.count(&format!(
            "SELECT COUNT(*) FROM messages WHERE id = '{card}'"
        ))
        .await,
        0,
        "the old card is gone"
    );
    assert_eq!(f.count("SELECT COUNT(*) FROM qoty_winners").await, 0);
    assert_eq!(
        f.count("SELECT COUNT(*) FROM qoty_winner_speakers").await,
        0
    );
    assert_eq!(
        f.reopen(&kim_t, e.id, None).await.status(),
        409,
        "already live"
    );

    // Ended again, more than a day ago: too late.
    f.ended(&f.alex_t, e.id).await;
    assert_eq!(f.cards(f.ch.id).await.len(), 1, "one card, the new one");
    f.sql(&format!(
        "UPDATE qoty_events SET ended_at = '{}' WHERE id = '{}'",
        (chrono::Utc::now() - chrono::Duration::hours(25))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        e.id
    ))
    .await;
    assert_eq!(f.reopen(&kim_t, e.id, None).await.status(), 409);
}

#[tokio::test]
async fn reopen_needs_no_other_live_event_and_a_sane_length() {
    let f = fixture().await;
    let (e, _, _) = two_options(&f).await;
    f.ended(&f.alex_t, e.id).await;
    assert_eq!(f.reopen(&f.alex_t, e.id, Some(4)).await.status(), 400);
    assert_eq!(
        f.reopen(&f.alex_t, QotyEventId::new(), None).await.status(),
        404
    );
    let r = f.start(&f.alex_t, this_year() - 1, None).await;
    assert_eq!(r.status(), 200);
    assert_eq!(f.reopen(&f.alex_t, e.id, None).await.status(), 409);
}

#[tokio::test]
async fn reopen_drops_quotes_deleted_after_the_end() {
    let f = fixture().await;
    let (e, a, b) = two_options(&f).await;
    f.voted(&f.sam_t, e.id, a).await;
    f.voted(&f.alex_t, e.id, b).await;
    f.ended(&f.alex_t, e.id).await;
    let r = f
        .call(Method::DELETE, &f.alex_t, &format!("/messages/{a}"), None)
        .await;
    assert_eq!(r.status(), 204);
    // The ended event keeps it as history...
    assert_eq!(
        f.count(&format!(
            "SELECT COUNT(*) FROM qoty_options WHERE event_id = '{}'",
            e.id
        ))
        .await,
        2
    );
    // ...but a reopened one can't let a deleted quote win.
    let r = f.reopen(&f.sam_t, e.id, None).await;
    assert_eq!(r.status(), 200);
    let back: QotyEvent = r.json().await.unwrap();
    assert_eq!(option_ids(&back), vec![b]);
    assert_eq!((voters(&back), back.voters), (vec![vec![f.alex]], 1));
}

#[tokio::test]
async fn deleting_an_option_frees_its_voters() {
    let f = fixture().await;
    let (_, kim_t) = f.member("kim").await;
    let (e, a, b) = two_options(&f).await;
    f.voted(&f.sam_t, e.id, a).await;
    let mut ws = hello(&f.app, &kim_t).await;
    let r = f
        .call(Method::DELETE, &f.alex_t, &format!("/messages/{a}"), None)
        .await;
    assert_eq!(r.status(), 204);
    let updated: Vec<QotyEvent> = seen(&mut ws)
        .await
        .into_iter()
        .filter_map(|s| match s {
            Seen::Updated(e) => Some(e),
            _ => None,
        })
        .collect();
    assert_eq!(updated.len(), 1, "everyone hears the option went");
    assert_eq!(option_ids(&updated[0]), vec![b]);
    assert_eq!(updated[0].voters, 0);
    // sam's vote is free again.
    let e2 = f.voted(&f.sam_t, e.id, b).await;
    assert_eq!(voters(&e2), vec![vec![f.sam]]);
}

#[tokio::test]
async fn deleting_a_quote_publishes_under_the_order_lock() {
    let f = fixture().await;
    let (_, a, _) = two_options(&f).await;
    let guard = f.app.hub.in_order().await;
    let (http, url, token) = (
        f.app.http.clone(),
        f.app.url(&format!("/messages/{a}")),
        f.alex_t.clone(),
    );
    let del = tokio::spawn(async move {
        http.delete(url)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!del.is_finished(), "the delete waits on in_order");
    drop(guard);
    assert_eq!(del.await.unwrap(), 204);
}

// ---------- speakers ----------

#[tokio::test]
async fn speakers_lock_while_an_option() {
    let f = fixture().await;
    let (e, a, _) = two_options(&f).await;
    let page: Vec<Message> = get_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/messages", f.ch.id),
    )
    .await;
    let q = page.into_iter().find(|m| m.id == a).unwrap();
    let line = q.quote.as_ref().unwrap().lines[0].id;
    let text_fix = json!([{ "id": line, "speaker_id": f.sam, "text": "a, fixed" }]);
    let new_speaker = json!([{ "id": line, "speaker_id": f.alex, "text": "a" }]);
    let extra_line = json!([
        { "id": line, "speaker_id": f.sam, "text": "a" },
        { "speaker_name": "A Friend", "text": "more" },
    ]);
    assert_eq!(f.edit(&q, new_speaker.clone()).await, 409);
    assert_eq!(f.edit(&q, extra_line).await, 409);
    assert_eq!(f.edit(&q, text_fix).await, 200);
    // A quote that isn't an option isn't locked.
    let free = f.quote("free").await;
    let free_line = free.quote.as_ref().unwrap().lines[0].id;
    assert_eq!(
        f.edit(
            &free,
            json!([{ "id": free_line, "speaker_id": f.alex, "text": "free" }])
        )
        .await,
        200
    );
    // Once the vote is over, the lock goes.
    f.ended(&f.alex_t, e.id).await;
    assert_eq!(f.edit(&q, new_speaker).await, 200);
}

/// The winner speaker rows of `m`, as (position, member, name).
async fn winner_speakers(f: &Fixture, m: MessageId) -> Vec<(i64, Option<String>, Option<String>)> {
    sqlx::query_as(
        "SELECT position, user_id, name FROM qoty_winner_speakers WHERE message_id = ? ORDER BY position",
    )
    .bind(m.to_string())
    .fetch_all(&f.app.db)
    .await
    .unwrap()
}

#[tokio::test]
async fn wins_keep_their_speaker_snapshot() {
    let f = fixture().await;
    let e = f.started().await;
    let r = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/quotes", f.ch.id),
        json!({ "lines": [
            { "speaker_id": f.sam, "text": "one" },
            { "speaker_name": "A Friend", "text": "two" },
        ] }),
    )
    .await;
    let q: Message = r.json().await.unwrap();
    assert_eq!(f.add(&f.alex_t, e.id, q.id).await.status(), 200);
    f.voted(&f.alex_t, e.id, q.id).await;
    f.ended(&f.alex_t, e.id).await;
    let want = vec![
        (0, Some(f.sam.to_string()), None),
        (1, None, Some("A Friend".to_string())),
    ];
    assert_eq!(winner_speakers(&f, q.id).await, want);
    let lines = q.quote.as_ref().unwrap().lines.clone();
    assert_eq!(
        f.edit(
            &q,
            json!([
                { "id": lines[0].id, "speaker_id": f.alex, "text": "one" },
                { "id": lines[1].id, "speaker_id": f.alex, "text": "two" },
            ])
        )
        .await,
        200
    );
    assert_eq!(
        winner_speakers(&f, q.id).await,
        want,
        "unchanged by the edit"
    );
}

#[tokio::test]
async fn linking_works_during_a_live_vote() {
    let f = fixture().await;
    let (kim, _) = f.member("kim").await;
    let e = f.started().await;
    let r = post_json(
        &f.app,
        &f.alex_t,
        &format!("/channels/{}/quotes", f.ch.id),
        json!({ "lines": [{ "speaker_name": "A Friend", "text": "one" }] }),
    )
    .await;
    let q: Message = r.json().await.unwrap();
    assert_eq!(f.add(&f.alex_t, e.id, q.id).await.status(), 200);
    let r = f
        .call(
            Method::PUT,
            &f.sam_t,
            &format!("/servers/{}/speaker-links", f.server),
            Some(json!({ "name": "A Friend", "user_id": kim })),
        )
        .await;
    assert_eq!(r.status(), 204, "linking isn't an edit");
    f.voted(&f.sam_t, e.id, q.id).await;
    f.ended(&f.alex_t, e.id).await;
    // The win records the effective speaker at the end, and keeps the name.
    assert_eq!(
        winner_speakers(&f, q.id).await,
        vec![(0, Some(kim.to_string()), Some("A Friend".to_string()))]
    );
    // Undoing the link afterwards doesn't touch the snapshot.
    let r = f
        .call(
            Method::DELETE,
            &f.sam_t,
            &format!("/servers/{}/speaker-links/a friend", f.server),
            None,
        )
        .await;
    assert_eq!(r.status(), 204);
    assert_eq!(
        winner_speakers(&f, q.id).await,
        vec![(0, Some(kim.to_string()), Some("A Friend".to_string()))]
    );
}

// ---------- live ----------

#[tokio::test]
async fn everyone_sees_votes_end_and_reopen_live() {
    let f = fixture().await;
    let (_, kim_t) = f.member("kim").await;
    let fans = f.fans(1).await;
    let a = f.quote("a").await;
    f.score(a.id, &fans, 1).await;
    let mut ws = hello(&f.app, &kim_t).await;
    let e = f.started().await;
    f.voted(&f.sam_t, e.id, a.id).await;
    f.ended(&f.alex_t, e.id).await;
    assert_eq!(f.reopen(&f.sam_t, e.id, None).await.status(), 200);

    let got = seen(&mut ws).await;
    assert_eq!(got.len(), 6, "{got:?}");
    assert!(matches!(&got[0], Seen::Started(x) if x.id == e.id && x.options.len() == 1));
    assert!(matches!(&got[1], Seen::Updated(x) if x.options[0].voters == vec![f.sam]));
    let Seen::Card(card) = &got[2] else {
        panic!("card first: {:?}", got[2])
    };
    assert_eq!(card.qoty_card.as_ref().unwrap().voters, vec![f.sam]);
    assert!(matches!(&got[3], Seen::Ended(x) if x.end_reason == Some(QotyEndReason::Early)));
    assert!(matches!(&got[4], Seen::Deleted(id) if *id == card.id));
    assert!(
        matches!(&got[5], Seen::Started(x) if x.ended_at.is_none() && x.options[0].voters == vec![f.sam])
    );
}

#[tokio::test]
async fn ready_has_the_live_event_with_voters() {
    let f = fixture().await;
    let (_, kim_t) = f.member("kim").await;
    let (e, a, _) = two_options(&f).await;
    f.voted(&f.sam_t, e.id, a).await;
    let (_, ready) = hello_ready(&f.app, &kim_t).await;
    assert_eq!(ready.qoty.len(), 1);
    assert_eq!(ready.qoty[0].id, e.id);
    assert_eq!(voters(&ready.qoty[0]), vec![vec![f.sam], vec![]]);
    assert_eq!((ready.qoty[0].voters, ready.qoty[0].members), (1, 3));

    // Ended early: still there, so the reopen button can show...
    f.ended(&f.alex_t, e.id).await;
    let (_, ready) = hello_ready(&f.app, &kim_t).await;
    assert_eq!(ready.qoty.len(), 1);
    assert!(ready.qoty[0].reopen_until.is_some());
    // ...until a day has gone by.
    f.sql(&format!(
        "UPDATE qoty_events SET ended_at = '2000-01-01T00:00:00.000Z' WHERE id = '{}'",
        e.id
    ))
    .await;
    let (_, ready) = hello_ready(&f.app, &kim_t).await;
    assert!(ready.qoty.is_empty());
}

#[tokio::test]
async fn serve_ends_overdue_votes_before_it_listens() {
    let f = fixture().await;
    let (e, a, _) = two_options_timed(&f).await;
    f.voted(&f.sam_t, e.id, a).await;
    f.sql(&format!(
        "UPDATE qoty_events SET ends_at = '2000-01-01T00:00:00.000Z' WHERE id = '{}'",
        e.id
    ))
    .await;
    // The real binary on the same database: by the time it answers, the vote is over.
    let _server = spawn_binary_on(f.app.cfg.db_url.clone(), &[], None).await;
    let reason: Option<String> =
        sqlx::query_scalar("SELECT end_reason FROM qoty_events WHERE id = ?")
            .bind(e.id.to_string())
            .fetch_one(&f.app.db)
            .await
            .unwrap();
    assert_eq!(reason.as_deref(), Some("time"));
    assert_eq!(f.cards(f.ch.id).await.len(), 1);
}
