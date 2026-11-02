//! Message search: `GET /servers/{id}/search` and `GET /channels/{id}/search`.
mod common;

use common::*;
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{Channel, HighlightRange, Message, SearchResults};
use serde_json::json;

struct Fx {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    server: ServerId,
    general: Channel,
}

async fn setup() -> Fx {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let s = create_server(&app, &alex.1, "Main").await;
    post_json(&app, &sam.1, &format!("/servers/{}/join", s.id), json!({})).await;
    let general = general(&app, &alex.1, s.id).await;
    Fx {
        app,
        alex,
        sam,
        server: s.id,
        general,
    }
}

async fn channel(f: &Fx, kind: &str, name: &str) -> Channel {
    let r = post_json(
        &f.app,
        &f.alex.1,
        &format!("/servers/{}/channels", f.server),
        json!({ "kind": kind, "name": name }),
    )
    .await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

async fn get(app: &TestApp, token: &str, path: &str) -> reqwest::Response {
    app.http
        .get(app.url(path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

async fn server_search(f: &Fx, token: &str, query: &str) -> SearchResults {
    get_json(
        &f.app,
        token,
        &format!("/servers/{}/search?{query}", f.server),
    )
    .await
}

fn ids(r: &SearchResults) -> Vec<MessageId> {
    r.hits.iter().map(|h| h.message.id).collect()
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn quote(f: &Fx, token: &str, ch: ChannelId, lines: serde_json::Value) -> Message {
    let r = post_json(
        &f.app,
        token,
        &format!("/channels/{ch}/quotes"),
        json!({ "lines": lines }),
    )
    .await;
    assert_eq!(r.status(), 200, "post quote");
    r.json().await.unwrap()
}

/// A clip row straight into the database (posting a real one needs storage and ffmpeg).
async fn clip(f: &Fx, ch: ChannelId, caption: &str) -> MessageId {
    let id = MessageId::new();
    let ts = pulse_server::db::now();
    for q in [
        format!(
            "INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES ('{id}', '{ch}', '{}', '{caption}', '{ts}')",
            f.alex.0
        ),
        format!(
            "INSERT INTO media (hash, kind, location, content_type, size, width, height, frames, created_at) VALUES ('v{id}', 'clip', 'r2', 'video/mp4', 1, 1, 1, 1, '{ts}')"
        ),
        format!(
            "INSERT INTO media (hash, kind, content_type, size, width, height, frames, created_at) VALUES ('t{id}', 'clip_thumb', 'image/webp', 1, 1, 1, 1, '{ts}')"
        ),
        format!(
            "INSERT INTO clips (message_id, video_key, thumb_hash, duration_ms, width, height) VALUES ('{id}', 'v{id}', 't{id}', 1000, 1, 1)"
        ),
    ] {
        sqlx::query(&q).execute(&f.app.db).await.unwrap();
    }
    id
}

#[tokio::test]
async fn finds_words_newest_first() {
    let f = setup().await;
    let a = send(&f.app, &f.alex.1, f.general.id, "pizza tonight?").await;
    send(&f.app, &f.sam.1, f.general.id, "tacos").await;
    let b = send(&f.app, &f.sam.1, f.general.id, "Pizzas are great").await;
    let r = server_search(&f, &f.alex.1, "q=pizza").await;
    assert_eq!(ids(&r), vec![b.id, a.id]);
    assert_eq!(r.total, Some(2));
    assert_eq!(r.next_cursor, None);
    assert_eq!(r.hits[0].text, "Pizzas are great");
    assert_eq!(
        r.hits[0].highlights,
        vec![HighlightRange { start: 0, end: 6 }]
    );
    // All words must appear.
    let r = server_search(&f, &f.alex.1, "q=pizza+great").await;
    assert_eq!(ids(&r), vec![b.id]);
    // A phrase must appear as is.
    let r = server_search(&f, &f.alex.1, &format!("q={}", enc("\"pizza tonight\""))).await;
    assert_eq!(ids(&r), vec![a.id]);
}

#[tokio::test]
async fn filter_only_from() {
    let f = setup().await;
    send(&f.app, &f.alex.1, f.general.id, "one").await;
    let s1 = send(&f.app, &f.sam.1, f.general.id, "two").await;
    let s2 = send(&f.app, &f.sam.1, f.general.id, "three").await;
    let r = server_search(&f, &f.alex.1, &format!("from={}", f.sam.0)).await;
    assert_eq!(ids(&r), vec![s2.id, s1.id]);
    assert!(r.hits[0].highlights.is_empty());
}

#[tokio::test]
async fn from_matches_quote_speakers_and_links() {
    let f = setup().await;
    let q = channel(&f, "quotes", "quotes").await;
    // alex posts both: one where sam is the member speaker, one said by a plain name.
    let said = quote(
        &f,
        &f.alex.1,
        q.id,
        json!([{ "speaker_id": f.sam.0, "text": "I never lose" }]),
    )
    .await;
    let named = quote(
        &f,
        &f.alex.1,
        q.id,
        json!([{ "speaker_name": "Sammy", "text": "rematch" }]),
    )
    .await;
    let r = server_search(&f, &f.alex.1, &format!("from={}", f.sam.0)).await;
    assert_eq!(
        ids(&r),
        vec![said.id],
        "a linked name only counts once linked"
    );
    let r = post_json_put(
        &f,
        &format!("/servers/{}/speaker-links", f.server),
        json!({ "name": "Sammy", "user_id": f.sam.0 }),
    )
    .await;
    assert!(r.status().is_success(), "{}", r.status());
    let r = server_search(&f, &f.alex.1, &format!("from={}", f.sam.0)).await;
    assert_eq!(ids(&r), vec![named.id, said.id]);
    // The poster still counts as "from" too.
    let r = server_search(&f, &f.alex.1, &format!("from={}", f.alex.0)).await;
    assert_eq!(ids(&r), vec![named.id, said.id]);
}

async fn post_json_put(f: &Fx, path: &str, body: serde_json::Value) -> reqwest::Response {
    f.app
        .http
        .put(f.app.url(path))
        .bearer_auth(&f.alex.1)
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn in_one_channel() {
    let f = setup().await;
    let other = channel(&f, "text", "memes").await;
    send(&f.app, &f.alex.1, f.general.id, "pizza here").await;
    let m = send(&f.app, &f.alex.1, other.id, "pizza there").await;
    let r = server_search(&f, &f.alex.1, &format!("q=pizza&in={}", other.id)).await;
    assert_eq!(ids(&r), vec![m.id]);
}

#[tokio::test]
async fn has_clip_quote_link() {
    let f = setup().await;
    let q = channel(&f, "quotes", "quotes").await;
    let c = channel(&f, "clips", "clips").await;
    let link = send(
        &f.app,
        &f.alex.1,
        f.general.id,
        "see https://example.com now",
    )
    .await;
    let www = send(&f.app, &f.alex.1, f.general.id, "or www.example.com").await;
    send(&f.app, &f.alex.1, f.general.id, "no link here").await;
    // Typed in capitals, it is still a link.
    let caps = send(&f.app, &f.alex.1, f.general.id, "HTTPS://EXAMPLE.COM").await;
    let qt = quote(
        &f,
        &f.alex.1,
        q.id,
        json!([{ "speaker_id": f.sam.0, "text": "a link to nothing" }]),
    )
    .await;
    let cl = clip(&f, c.id, "nice link shot").await;
    let r = server_search(&f, &f.alex.1, "has=link").await;
    assert_eq!(ids(&r), vec![caps.id, www.id, link.id]);
    let r = server_search(&f, &f.alex.1, "has=quote").await;
    assert_eq!(ids(&r), vec![qt.id]);
    let r = server_search(&f, &f.alex.1, "has=clip").await;
    assert_eq!(ids(&r), vec![cl]);
    assert!(
        r.hits[0].message.clip.is_some(),
        "the clip part comes along"
    );
    let r = server_search(&f, &f.alex.1, "q=link&has=clip,quote").await;
    assert!(r.hits.is_empty(), "both must hold");
    let r = get(
        &f.app,
        &f.alex.1,
        &format!("/servers/{}/search?has=emoji", f.server),
    )
    .await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn mentions_filter_includes_reply_pings() {
    let f = setup().await;
    let ping = send(&f.app, &f.alex.1, f.general.id, "@sam look").await;
    let mine = send(&f.app, &f.sam.1, f.general.id, "hello").await;
    let r = post_json(
        &f.app,
        &f.alex.1,
        &format!("/channels/{}/messages", f.general.id),
        json!({ "content": "hi back", "reply_to_id": mine.id }),
    )
    .await;
    let reply: Message = r.json().await.unwrap();
    // An @sam added by an edit doesn't count (notifications don't either).
    let later = send(&f.app, &f.alex.1, f.general.id, "nothing").await;
    f.app
        .http
        .patch(f.app.url(&format!("/messages/{}", later.id)))
        .bearer_auth(&f.alex.1)
        .json(&json!({ "content": "@sam now" }))
        .send()
        .await
        .unwrap();
    let r = server_search(&f, &f.alex.1, &format!("mentions={}", f.sam.0)).await;
    assert_eq!(ids(&r), vec![reply.id, ping.id]);
}

#[tokio::test]
async fn combined_filters() {
    let f = setup().await;
    let other = channel(&f, "text", "memes").await;
    send(&f.app, &f.alex.1, f.general.id, "pizza https://a.example").await;
    send(&f.app, &f.sam.1, other.id, "pizza https://b.example").await;
    let hit = send(&f.app, &f.sam.1, f.general.id, "pizza https://c.example").await;
    send(&f.app, &f.sam.1, f.general.id, "pizza without").await;
    let r = server_search(
        &f,
        &f.alex.1,
        &format!("q=pizza&from={}&in={}&has=link", f.sam.0, f.general.id),
    )
    .await;
    assert_eq!(ids(&r), vec![hit.id]);
}

/// A last page that is exactly full says there is nothing after it (no empty "Load more").
#[tokio::test]
async fn an_exactly_full_last_page_has_no_cursor() {
    let f = setup().await;
    for i in 0..25 {
        send(&f.app, &f.alex.1, f.general.id, &format!("pizza {i}")).await;
    }
    let r = server_search(&f, &f.alex.1, "q=pizza").await;
    assert_eq!(r.hits.len(), 25);
    assert_eq!(r.next_cursor, None);
    assert_eq!(r.total, Some(25));
}

/// A DM is searched on its own docs (and a server's `in:` on one channel's), newest first,
/// with words or with filters only.
#[tokio::test]
async fn one_channel_searches_page_newest_first() {
    let f = setup().await;
    let dm = create_dm(&f.app, &f.alex.1, &[f.sam.0]).await;
    let mut mine = Vec::new();
    for i in 0..30 {
        send(&f.app, &f.alex.1, f.general.id, &format!("pizza noise {i}")).await;
        mine.push(send(&f.app, &f.sam.1, dm.id, &format!("pizza dm {i}")).await.id);
    }
    mine.reverse();
    let path = |q: &str| format!("/channels/{}/search?{q}", dm.id);
    let p1: SearchResults = get_json(&f.app, &f.alex.1, &path("q=pizza")).await;
    let p2: SearchResults = get_json(
        &f.app,
        &f.alex.1,
        &path(&format!("q=pizza&cursor={}", p1.next_cursor.clone().unwrap())),
    )
    .await;
    assert_eq!(p1.total, Some(30));
    let got: Vec<MessageId> = ids(&p1).into_iter().chain(ids(&p2)).collect();
    assert_eq!(got, mine);
    let only_from: SearchResults =
        get_json(&f.app, &f.alex.1, &path(&format!("from={}", f.sam.0))).await;
    assert_eq!(ids(&only_from), mine[..25].to_vec());
    let r = server_search(&f, &f.alex.1, &format!("q=pizza&in={}", f.general.id)).await;
    assert_eq!(r.total, Some(30));
    assert!(r.hits.iter().all(|h| h.message.channel_id == f.general.id));
}

#[tokio::test]
async fn paging_is_stable() {
    let f = setup().await;
    let mut sent = Vec::new();
    for i in 0..60 {
        sent.push(
            send(&f.app, &f.alex.1, f.general.id, &format!("pizza {i}"))
                .await
                .id,
        );
    }
    sent.reverse();
    let p1 = server_search(&f, &f.alex.1, "q=pizza").await;
    assert_eq!(p1.hits.len(), 25);
    assert_eq!(p1.total, Some(51), "counted up to 51");
    // A new match arriving between pages doesn't shift the next page.
    send(&f.app, &f.alex.1, f.general.id, "pizza new").await;
    let c1 = p1.next_cursor.clone().unwrap();
    let p2 = server_search(&f, &f.alex.1, &format!("q=pizza&cursor={c1}")).await;
    assert_eq!(p2.total, None, "the count is on the first page only");
    let p3 = server_search(
        &f,
        &f.alex.1,
        &format!("q=pizza&cursor={}", p2.next_cursor.clone().unwrap()),
    )
    .await;
    assert_eq!(p3.next_cursor, None);
    let all: Vec<MessageId> = [&p1, &p2, &p3].iter().flat_map(|p| ids(p)).collect();
    assert_eq!(all, sent);
}

#[tokio::test]
async fn deleted_never_shows() {
    let f = setup().await;
    let m = send(&f.app, &f.alex.1, f.general.id, "secret pizza").await;
    let r = f
        .app
        .http
        .delete(f.app.url(&format!("/messages/{}", m.id)))
        .bearer_auth(&f.alex.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert!(
        server_search(&f, &f.alex.1, "q=pizza")
            .await
            .hits
            .is_empty()
    );
    assert!(
        server_search(&f, &f.alex.1, &format!("from={}", f.alex.0))
            .await
            .hits
            .is_empty()
    );
}

#[tokio::test]
async fn dm_not_member_404() {
    let f = setup().await;
    let (_, jo) = register(&f.app, "jo").await;
    let dm = create_dm(&f.app, &f.alex.1, &[f.sam.0]).await;
    send(&f.app, &f.alex.1, dm.id, "pizza between us").await;
    let mine: SearchResults = get_json(
        &f.app,
        &f.sam.1,
        &format!("/channels/{}/search?q=pizza", dm.id),
    )
    .await;
    assert_eq!(mine.hits.len(), 1);
    let r = get(&f.app, &jo, &format!("/channels/{}/search?q=pizza", dm.id)).await;
    assert_eq!(r.status(), 404);
    // And a DM's messages never show in a server's search.
    assert!(
        server_search(&f, &f.alex.1, "q=pizza")
            .await
            .hits
            .is_empty()
    );
}

#[tokio::test]
async fn left_group_404() {
    let f = setup().await;
    let (jo_id, _) = register(&f.app, "jo").await;
    let group = create_dm(&f.app, &f.alex.1, &[f.sam.0, jo_id]).await;
    send(&f.app, &f.alex.1, group.id, "pizza for three").await;
    let r = f
        .app
        .http
        .delete(
            f.app
                .url(&format!("/channels/{}/members/{}", group.id, f.sam.0)),
        )
        .bearer_auth(&f.sam.1)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
    let r = get(
        &f.app,
        &f.sam.1,
        &format!("/channels/{}/search?q=pizza", group.id),
    )
    .await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn other_server_never_in_results() {
    let f = setup().await;
    let other = create_server(&f.app, &f.sam.1, "Other").await;
    let og = general(&f.app, &f.sam.1, other.id).await;
    send(&f.app, &f.sam.1, og.id, "pizza elsewhere").await;
    let here = send(&f.app, &f.sam.1, f.general.id, "pizza here").await;
    assert_eq!(
        ids(&server_search(&f, &f.alex.1, "q=pizza").await),
        vec![here.id]
    );
    // Naming the other server's channel with `in` finds nothing either.
    let r = server_search(&f, &f.alex.1, &format!("q=pizza&in={}", og.id)).await;
    assert!(r.hits.is_empty());
    let r = get(
        &f.app,
        &f.alex.1,
        &format!("/servers/{}/search?q=pizza", ServerId::new()),
    )
    .await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn server_channel_on_channel_route_400() {
    let f = setup().await;
    let r = get(
        &f.app,
        &f.alex.1,
        &format!("/channels/{}/search?q=pizza", f.general.id),
    )
    .await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn thread_reply_carries_root() {
    let f = setup().await;
    let q = channel(&f, "quotes", "quotes").await;
    let root = quote(
        &f,
        &f.alex.1,
        q.id,
        json!([{ "speaker_id": f.sam.0, "text": "honest" }]),
    )
    .await;
    let r = post_json(
        &f.app,
        &f.sam.1,
        &format!("/channels/{}/messages", q.id),
        json!({ "content": "pizza reply", "thread_root_id": root.id }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let reply: Message = r.json().await.unwrap();
    let r = server_search(&f, &f.alex.1, "q=pizza").await;
    assert_eq!(ids(&r), vec![reply.id]);
    assert_eq!(r.hits[0].root.as_ref().map(|m| m.id), Some(root.id));
    assert!(r.hits[0].root.as_ref().unwrap().quote.is_some());
    // A top-level hit has no root.
    let r = server_search(&f, &f.alex.1, "q=honest").await;
    assert_eq!(ids(&r), vec![root.id]);
    assert!(r.hits[0].root.is_none());
}

#[tokio::test]
async fn custom_emoji_id_doesnt_match() {
    let f = setup().await;
    let m = send(
        &f.app,
        &f.alex.1,
        f.general.id,
        "gg <:party:01JABCDEFGHJKMNPQRSTVWXYZ0>",
    )
    .await;
    let r = server_search(&f, &f.alex.1, "q=party").await;
    assert_eq!(ids(&r), vec![m.id]);
    assert_eq!(r.hits[0].text, "gg :party:");
    let r = server_search(&f, &f.alex.1, "q=01jab").await;
    assert!(r.hits.is_empty(), "the emoji's id isn't searchable text");
}

#[tokio::test]
async fn empty_search_400() {
    let f = setup().await;
    for q in ["", "q=", "q=%20--%20", "q=pizza&cursor=not-a-cursor"] {
        let r = get(
            &f.app,
            &f.alex.1,
            &format!("/servers/{}/search?{q}", f.server),
        )
        .await;
        assert_eq!(r.status(), 400, "{q}");
    }
}

#[tokio::test]
async fn rate_limited_429() {
    let mut cfg = test_config();
    cfg.limits.search_user = pulse_server::limits::Rate::new(2, 0.0001);
    let app = spawn_with(cfg).await;
    let (_, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let path = format!("/servers/{}/search?q=pizza", s.id);
    assert_eq!(get(&app, &alex, &path).await.status(), 200);
    assert_eq!(get(&app, &alex, &path).await.status(), 200);
    assert_eq!(get(&app, &alex, &path).await.status(), 429);
}
