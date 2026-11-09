//! Polls (#157): posting, voting, who sees what, closing (early and by time), the ended line, deletes and limits.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{ApiError, Channel, Message, MessageKind, Poll};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> Socket {
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        if let Some(ServerFrame::Ready(_)) = next_frame(&mut ws).await {
            return ws;
        }
    }
}

async fn next_frame(ws: &mut Socket) -> Option<ServerFrame> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        match m {
            Ws::Text(t) => return Some(serde_json::from_str(&t).unwrap()),
            Ws::Close(_) => return None,
            _ => continue,
        }
    }
}

/// The raw JSON of the next event `pick` takes (skipping others); None when none comes within 3 s.
async fn next_raw(ws: &mut Socket, pick: impl Fn(&Value) -> bool) -> Option<Value> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        if let Ws::Text(t) = m {
            let v: Value = serde_json::from_str(&t).unwrap();
            if v["op"] == "Event" && pick(&v["d"]) {
                return Some(v["d"].clone());
            }
        }
    }
}

async fn req(
    app: &TestApp,
    method: reqwest::Method,
    token: &str,
    path: &str,
    body: Option<Value>,
) -> reqwest::Response {
    let mut r = app.http.request(method, app.url(path)).bearer_auth(token);
    if let Some(b) = body {
        r = r.json(&b);
    }
    r.send().await.unwrap()
}

async fn error(r: reqwest::Response) -> (u16, String) {
    let status = r.status().as_u16();
    let body: ApiError = r.json().await.unwrap();
    (status, body.message)
}

async fn join(app: &TestApp, token: &str, server: ServerId) {
    let r = post_json(app, token, &format!("/servers/{server}/join"), json!({})).await;
    assert!(r.status().is_success());
}

fn poll_body(question: &str, options: &[&str]) -> Value {
    json!({"question": question, "options": options, "ends_in": "day"})
}

async fn create(app: &TestApp, token: &str, channel: ChannelId, body: Value) -> reqwest::Response {
    post_json(app, token, &format!("/channels/{channel}/polls"), body).await
}

async fn create_ok(app: &TestApp, token: &str, channel: ChannelId, body: Value) -> Message {
    let r = create(app, token, channel, body).await;
    assert_eq!(r.status(), 201);
    r.json().await.unwrap()
}

async fn vote(app: &TestApp, token: &str, poll: MessageId, options: &[u32]) -> reqwest::Response {
    req(
        app,
        reqwest::Method::PUT,
        token,
        &format!("/messages/{poll}/poll/votes"),
        Some(json!({ "options": options })),
    )
    .await
}

async fn vote_ok(app: &TestApp, token: &str, poll: MessageId, options: &[u32]) -> Poll {
    let r = vote(app, token, poll, options).await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

async fn view(app: &TestApp, token: &str, poll: MessageId) -> Poll {
    get_json(app, token, &format!("/messages/{poll}/poll")).await
}

async fn close(app: &TestApp, token: &str, poll: MessageId) -> reqwest::Response {
    post_json(
        app,
        token,
        &format!("/messages/{poll}/poll/close"),
        json!({}),
    )
    .await
}

/// The chat's ended lines, oldest first.
async fn ended_lines(app: &TestApp, token: &str, channel: ChannelId) -> Vec<Message> {
    let list: Vec<Message> = get_json(app, token, &format!("/channels/{channel}/messages")).await;
    let mut lines: Vec<Message> = list
        .into_iter()
        .filter(|m| m.poll_ended.is_some())
        .collect();
    lines.sort_by_key(|m| m.id.to_string());
    lines
}

/// alex owns a server sam and kim are in; jo isn't in it. Returns (app, tokens [alex, sam, kim, jo], #general).
async fn setup() -> (TestApp, [String; 4], Channel) {
    let (app, tokens, _, g) = setup_ids().await;
    (app, tokens, g)
}

/// [`setup`], with the four people's ids.
async fn setup_ids() -> (TestApp, [String; 4], [UserId; 4], Channel) {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (kim_id, kim) = register(&app, "kim").await;
    let (jo_id, jo) = register(&app, "jo").await;
    let s = create_server(&app, &alex, "Main").await;
    join(&app, &sam, s.id).await;
    join(&app, &kim, s.id).await;
    let g = general(&app, &alex, s.id).await;
    (
        app,
        [alex, sam, kim, jo],
        [alex_id, sam_id, kim_id, jo_id],
        g,
    )
}

#[tokio::test]
async fn a_poll_is_a_message_with_its_text_for_older_clients() {
    let (app, [alex, sam, ..], g) = setup().await;
    let m = create_ok(
        &app,
        &alex,
        g.id,
        json!({"question": "  Where to eat? ", "options": [" Pizza ", "Sushi"], "multi": true, "ends_in": "hour"}),
    )
    .await;
    assert_eq!(m.kind, MessageKind::Normal);
    assert_eq!(
        m.content,
        "📊 Poll: Where to eat?\n• Pizza\n• Sushi\n(Update Pulse to vote)"
    );
    let p = m.poll.as_deref().unwrap();
    assert_eq!(p.question, "Where to eat?");
    assert_eq!(p.options, vec!["Pizza", "Sushi"]);
    assert!(p.multi && !p.anonymous && !p.closed);
    assert!(p.ends_at.is_some());
    // the creator sees results (all zero) from the start
    let r = p.results.as_ref().unwrap();
    assert_eq!(r.len(), 2);
    assert!(r.iter().all(|t| t.votes == 0 && t.voters.is_empty()));

    // history: as each person sees it
    let list: Vec<Message> = get_json(&app, &sam, &format!("/channels/{}/messages", g.id)).await;
    let mine = list.iter().find(|x| x.id == m.id).unwrap();
    let p = mine.poll.as_deref().unwrap();
    assert_eq!(p.results, None, "sam hasn't voted");
    assert_eq!(p.voters, 0);

    // No end
    let m = create_ok(
        &app,
        &alex,
        g.id,
        json!({"question": "Q", "options": ["a", "b"], "ends_in": "never"}),
    )
    .await;
    assert_eq!(m.poll.unwrap().ends_at, None);
}

#[tokio::test]
async fn posting_checks_the_poll_and_the_chat() {
    let (app, [alex, _, _, jo], g) = setup().await;
    let bad = [
        poll_body("", &["a", "b"]),
        poll_body(&"q".repeat(201), &["a", "b"]),
        poll_body("Q", &["a"]),
        poll_body(
            "Q",
            &["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"],
        ),
        poll_body("Q", &["a", &"o".repeat(81)]),
        poll_body("Q", &["a", "  "]),
        poll_body("Q\nmore", &["a", "b"]),
    ];
    for b in bad {
        assert_eq!(
            create(&app, &alex, g.id, b.clone()).await.status(),
            400,
            "{b}"
        );
    }
    assert_eq!(
        create(
            &app,
            &alex,
            g.id,
            poll_body(&"é".repeat(200), &[&"é".repeat(80), "b"])
        )
        .await
        .status(),
        201
    );
    let ten: Vec<String> = (0..10).map(|i| i.to_string()).collect();
    let ten: Vec<&str> = ten.iter().map(String::as_str).collect();
    assert_eq!(
        create(&app, &alex, g.id, poll_body("Q", &ten))
            .await
            .status(),
        201
    );
    // someone outside the server
    assert_eq!(
        create(&app, &jo, g.id, poll_body("Q", &["a", "b"]))
            .await
            .status(),
        403
    );
}

#[tokio::test]
async fn polls_go_in_chats_but_not_quotes_or_clips_channels() {
    let (app, [alex, ..], g) = setup().await;
    let s = g.server_id.unwrap();
    for kind in ["quotes", "clips"] {
        let r = post_json(
            &app,
            &alex,
            &format!("/servers/{s}/channels"),
            json!({"kind": kind, "name": format!("a-{kind}")}),
        )
        .await;
        let ch: Channel = r.json().await.unwrap();
        assert_eq!(
            create(&app, &alex, ch.id, poll_body("Q", &["a", "b"]))
                .await
                .status(),
            400,
            "{kind}"
        );
    }
    // a DM, for its two only
    let (sam_id, sam) = register(&app, "sam2").await;
    let (_, kim) = register(&app, "kim2").await;
    let dm = create_dm(&app, &alex, &[sam_id]).await;
    let p = create_ok(&app, &sam, dm.id, poll_body("Q", &["a", "b"])).await;
    vote_ok(&app, &alex, p.id, &[0]).await;
    assert_eq!(vote(&app, &kim, p.id, &[0]).await.status(), 404);
    assert_eq!(
        create(&app, &kim, dm.id, poll_body("Q", &["a", "b"]))
            .await
            .status(),
        404
    );
}

#[tokio::test]
async fn single_choice_moves_multi_choice_sets_and_an_empty_vote_takes_it_back() {
    let (app, [alex, sam, kim, _], g) = setup().await;
    let single = create_ok(&app, &alex, g.id, poll_body("Q", &["a", "b", "c"])).await;
    let p = vote_ok(&app, &sam, single.id, &[0]).await;
    assert_eq!(p.mine, vec![0]);
    let p = vote_ok(&app, &sam, single.id, &[2]).await;
    assert_eq!(p.mine, vec![2]);
    let r = p.results.unwrap();
    assert_eq!(r.iter().map(|t| t.votes).collect::<Vec<_>>(), vec![0, 0, 1]);
    assert_eq!(p.voters, 1);
    // two at once on a single-choice poll, and an option that isn't there
    assert_eq!(vote(&app, &sam, single.id, &[0, 1]).await.status(), 400);
    assert_eq!(vote(&app, &sam, single.id, &[3]).await.status(), 400);
    // taking it back
    let p = vote_ok(&app, &sam, single.id, &[]).await;
    assert!(p.mine.is_empty());
    assert_eq!(p.voters, 0);
    assert!(
        p.results.is_some(),
        "results once seen stay: taking the vote back is no way to peek"
    );
    // kim, who never voted, still sees none
    assert_eq!(view(&app, &kim, single.id).await.results, None);

    let multi = create_ok(
        &app,
        &alex,
        g.id,
        json!({"question": "Q", "options": ["a", "b", "c"], "multi": true, "ends_in": "day"}),
    )
    .await;
    let p = vote_ok(&app, &sam, multi.id, &[2, 0, 0]).await;
    assert_eq!(p.mine, vec![0, 2], "sorted, repeats dropped");
    vote_ok(&app, &kim, multi.id, &[0]).await;
    let p = vote_ok(&app, &sam, multi.id, &[0]).await;
    let r = p.results.unwrap();
    assert_eq!(r.iter().map(|t| t.votes).collect::<Vec<_>>(), vec![2, 0, 0]);
    assert_eq!(p.voters, 2);
}

#[tokio::test]
async fn results_show_to_voters_the_creator_and_everyone_once_closed() {
    let (app, [alex, sam, kim, _], [_, sam_id, kim_id, _], g) = setup_ids().await;
    let m = create_ok(&app, &alex, g.id, poll_body("Q", &["a", "b"])).await;
    vote_ok(&app, &sam, m.id, &[1]).await;
    // kim hasn't voted: the total, but no counts
    let p = view(&app, &kim, m.id).await;
    assert_eq!((p.voters, p.results.clone()), (1, None));
    // the creator, who hasn't voted, sees who voted for what
    let p = view(&app, &alex, m.id).await;
    assert_eq!(p.results.as_ref().unwrap()[1].voters, vec![sam_id]);
    // kim votes: now sees it all
    let p = vote_ok(&app, &kim, m.id, &[1]).await;
    assert_eq!(p.results.unwrap()[1].voters, vec![sam_id, kim_id]);

    // closed: a fresh member who never voted sees the final results
    let (_, jo) = register(&app, "jo2").await;
    join(&app, &jo, g.server_id.unwrap()).await;
    assert_eq!(view(&app, &jo, m.id).await.results, None);
    assert_eq!(close(&app, &alex, m.id).await.status(), 200);
    let p = view(&app, &jo, m.id).await;
    assert!(p.closed && p.closed_at.is_some());
    assert_eq!(p.results.unwrap()[1].votes, 2);
}

#[tokio::test]
async fn an_anonymous_poll_never_names_its_voters() {
    let (app, [alex, sam, kim, _], [_, sam_id, ..], g) = setup_ids().await;
    let m = create_ok(
        &app,
        &alex,
        g.id,
        json!({"question": "Q", "options": ["a", "b"], "anonymous": true, "ends_in": "day"}),
    )
    .await;
    let mut ws = hello(&app, &kim).await;
    let p = vote_ok(&app, &sam, m.id, &[0]).await;
    assert_eq!(p.mine, vec![0], "you still see your own choice");
    for who in [&alex, &sam] {
        let p = view(&app, who, m.id).await;
        let r = p.results.unwrap();
        assert_eq!(r[0].votes, 1);
        assert!(r.iter().all(|t| t.voters.is_empty()));
    }
    close(&app, &alex, m.id).await;
    let p = view(&app, &kim, m.id).await;
    assert!(p.results.unwrap().iter().all(|t| t.voters.is_empty()));
    // nor do its events or its line, as raw JSON
    let ev = next_raw(&mut ws, |d| d["t"] == "PollChanged")
        .await
        .unwrap();
    assert_eq!(ev["d"].as_object().unwrap().len(), 2, "{ev}");
    let list: Vec<Value> = get_json(&app, &kim, &format!("/channels/{}/messages", g.id)).await;
    assert!(
        !serde_json::to_string(&list)
            .unwrap()
            .contains(&sam_id.to_string())
    );
}

#[tokio::test]
async fn live_changes_go_out_as_poll_changed_and_the_new_poll_carries_no_results() {
    let (app, [alex, sam, ..], g) = setup().await;
    let mut ws = hello(&app, &sam).await;
    let m = create_ok(&app, &alex, g.id, poll_body("Q", &["a", "b"])).await;
    let created = next_raw(&mut ws, |d| d["t"] == "MessageCreated")
        .await
        .unwrap();
    let poll = &created["d"]["message"]["poll"];
    assert_eq!(poll["question"], "Q");
    assert!(
        poll["results"].is_null() && poll["mine"] == json!([]),
        "{poll}"
    );
    vote_ok(&app, &alex, m.id, &[0]).await;
    let ev = next_raw(&mut ws, |d| d["t"] == "PollChanged")
        .await
        .unwrap();
    assert_eq!(ev["d"]["message_id"], m.id.to_string());
    assert_eq!(ev["d"]["channel_id"], g.id.to_string());
}

#[tokio::test]
async fn only_the_creator_ends_a_poll_and_the_line_says_who_won() {
    let (app, [alex, sam, kim, _], g) = setup().await;
    let m = create_ok(
        &app,
        &alex,
        g.id,
        poll_body("Games?", &["Valheim", "Apex", "Rust"]),
    )
    .await;
    vote_ok(&app, &sam, m.id, &[0]).await;
    vote_ok(&app, &kim, m.id, &[0]).await;
    vote_ok(&app, &alex, m.id, &[1]).await;
    assert_eq!(close(&app, &sam, m.id).await.status(), 403);
    let mut ws = hello(&app, &sam).await;
    let p: Poll = close(&app, &alex, m.id).await.json().await.unwrap();
    assert!(p.closed);
    let line = next_raw(&mut ws, |d| {
        d["t"] == "MessageCreated" && !d["d"]["message"]["poll_ended"].is_null()
    })
    .await
    .unwrap();
    assert_eq!(
        line["d"]["message"]["content"],
        "📊 alex's poll ended: Valheim won (2 votes)"
    );
    let lines = ended_lines(&app, &sam, g.id).await;
    assert_eq!(lines.len(), 1);
    let e = lines[0].poll_ended.as_deref().unwrap();
    assert_eq!(
        (e.poll_message_id, e.winners.clone(), e.votes),
        (m.id, vec!["Valheim".to_string()], 2)
    );
    assert_eq!(lines[0].kind, MessageKind::System);
    // closed: no more votes, no second close
    let (status, _) = error(vote(&app, &sam, m.id, &[1]).await).await;
    assert_eq!(status, 409);
    assert_eq!(close(&app, &alex, m.id).await.status(), 409);
    assert_eq!(ended_lines(&app, &sam, g.id).await.len(), 1);
}

#[tokio::test]
async fn the_same_option_twice_is_two_options() {
    let (app, [alex, sam, kim, _], g) = setup().await;
    let m = create_ok(
        &app,
        &alex,
        g.id,
        poll_body("Lunch?", &["Pizza", "Pizza ", "Tacos"]),
    )
    .await;
    assert_eq!(
        m.poll.as_deref().unwrap().options,
        vec!["Pizza", "Pizza", "Tacos"]
    );
    vote_ok(&app, &sam, m.id, &[0]).await;
    let p = vote_ok(&app, &kim, m.id, &[1]).await;
    let counts: Vec<u32> = p.results.unwrap().iter().map(|t| t.votes).collect();
    assert_eq!(counts, vec![1, 1, 0], "votes count by position");
    close(&app, &alex, m.id).await;
    let lines = ended_lines(&app, &alex, g.id).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0].content,
        "📊 alex's poll ended: Pizza tied (1 vote each)"
    );
    assert_eq!(
        lines[0].poll_ended.as_deref().unwrap().winners,
        vec!["Pizza", "Pizza"]
    );
}

#[tokio::test]
async fn ties_and_no_votes_read_right() {
    let (app, [alex, sam, ..], g) = setup().await;
    let m = create_ok(
        &app,
        &alex,
        g.id,
        poll_body("Q", &["Pizza", "Sushi", "Tacos"]),
    )
    .await;
    vote_ok(&app, &sam, m.id, &[0]).await;
    vote_ok(&app, &alex, m.id, &[1]).await;
    close(&app, &alex, m.id).await;
    let empty = create_ok(&app, &alex, g.id, poll_body("Q", &["a", "b"])).await;
    close(&app, &alex, empty.id).await;
    let lines = ended_lines(&app, &alex, g.id).await;
    assert_eq!(
        lines.iter().map(|l| l.content.as_str()).collect::<Vec<_>>(),
        vec![
            "📊 alex's poll ended: Pizza and Sushi tied (1 vote each)",
            "📊 alex's poll ended with no votes",
        ]
    );
}

#[tokio::test]
async fn a_due_poll_reads_closed_and_refuses_votes_until_the_sweep_closes_it_once() {
    let (app, [alex, sam, ..], g) = setup().await;
    let m = create_ok(&app, &alex, g.id, poll_body("Q", &["a", "b"])).await;
    vote_ok(&app, &sam, m.id, &[1]).await;
    let open = create_ok(&app, &alex, g.id, poll_body("Q", &["a", "b"])).await;
    sqlx::query("UPDATE polls SET ends_at = '2000-01-01T00:00:00.000Z' WHERE message_id = ?")
        .bind(m.id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    let p = view(&app, &alex, m.id).await;
    assert!(
        p.closed && p.closed_at.is_none(),
        "over, though not swept yet"
    );
    assert_eq!(vote(&app, &sam, m.id, &[0]).await.status(), 409);
    assert!(ended_lines(&app, &alex, g.id).await.is_empty());

    let s = app.state();
    let now = pulse_server::db::now();
    assert_eq!(pulse_server::polls::close_due(&s, &now).await.unwrap(), 1);
    assert_eq!(pulse_server::polls::close_due(&s, &now).await.unwrap(), 0);
    let lines = ended_lines(&app, &alex, g.id).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].content, "📊 alex's poll ended: b won (1 vote)");
    assert!(view(&app, &alex, m.id).await.closed_at.is_some());
    assert!(!view(&app, &alex, open.id).await.closed);
    // ending it now is too late
    assert_eq!(close(&app, &alex, m.id).await.status(), 409);
}

#[tokio::test]
async fn closing_twice_at_once_posts_one_line() {
    let (app, [alex, ..], g) = setup().await;
    let m = create_ok(&app, &alex, g.id, poll_body("Q", &["a", "b"])).await;
    sqlx::query("UPDATE polls SET ends_at = '2000-01-01T00:00:00.000Z' WHERE message_id = ?")
        .bind(m.id.to_string())
        .execute(&app.db)
        .await
        .unwrap();
    let s = app.state();
    let now = pulse_server::db::now();
    let (a, b, c) = tokio::join!(
        pulse_server::polls::close_due(&s, &now),
        pulse_server::polls::close_due(&s, &now),
        close(&app, &alex, m.id),
    );
    assert_eq!(a.unwrap() + b.unwrap(), 1);
    assert_eq!(c.status(), 409);
    assert_eq!(ended_lines(&app, &alex, g.id).await.len(), 1);
}

#[tokio::test]
async fn deleting_a_poll_drops_it_and_its_votes() {
    let (app, [alex, sam, ..], g) = setup().await;
    let m = create_ok(&app, &alex, g.id, poll_body("Q", &["a", "b"])).await;
    vote_ok(&app, &sam, m.id, &[1]).await;
    let r = req(
        &app,
        reqwest::Method::DELETE,
        &alex,
        &format!("/messages/{}", m.id),
        None,
    )
    .await;
    assert_eq!(r.status(), 204);
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM polls) + (SELECT COUNT(*) FROM poll_votes) + (SELECT COUNT(*) FROM poll_seen)",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(left, 0);
    assert_eq!(vote(&app, &sam, m.id, &[0]).await.status(), 404);
    let list: Vec<Message> = get_json(&app, &sam, &format!("/channels/{}/messages", g.id)).await;
    let gone = list.iter().find(|x| x.id == m.id).unwrap();
    assert!(gone.deleted && gone.poll.is_none() && gone.content.is_empty());
}

#[tokio::test]
async fn a_poll_cant_be_edited_like_a_message() {
    let (app, [alex, ..], g) = setup().await;
    let m = create_ok(&app, &alex, g.id, poll_body("Q", &["a", "b"])).await;
    let r = req(
        &app,
        reqwest::Method::PATCH,
        &alex,
        &format!("/messages/{}", m.id),
        Some(json!({"content": "something else"})),
    )
    .await;
    assert_eq!(r.status(), 400);
    // a plain message's poll route
    let plain = send(&app, &alex, g.id, "hi").await;
    assert_eq!(vote(&app, &alex, plain.id, &[0]).await.status(), 404);
}

#[tokio::test]
async fn votes_are_rate_limited_per_person() {
    let mut cfg = test_config();
    cfg.limits.poll_vote_user = pulse_server::limits::Rate::new(2, 0.0001);
    let app = spawn_with(cfg).await;
    let (_, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let g = general(&app, &alex, s.id).await;
    let m = create_ok(&app, &alex, g.id, poll_body("Q", &["a", "b"])).await;
    vote_ok(&app, &alex, m.id, &[0]).await;
    vote_ok(&app, &alex, m.id, &[1]).await;
    assert_eq!(vote(&app, &alex, m.id, &[0]).await.status(), 429);
}
