//! Builds (#616): archiving one, bulk archiving, deleting, and the per-game counts.
mod common;

use std::time::Duration;

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId, ServerId, UserId};
use pulse_protocol::rest::{BuildFacets, BuildsBatchResult, BulkArchiveResult, Channel, Message};
use serde_json::json;
use tokio_tungstenite::tungstenite::Message as Ws;

/// alex owns the server; sam is in it; kim isn't.
struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    kim: String,
    server: ServerId,
    ch: ChannelId,
}

async fn world() -> World {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let s = create_server(&app, &alex.1, "Main").await;
    assert_eq!(join_server(&app, &sam.1, s.id).await.status(), 200);
    let r = post_json(
        &app,
        &alex.1,
        &format!("/servers/{}/channels", s.id),
        json!({ "kind": "builds", "name": "builds" }),
    )
    .await;
    let ch: Channel = r.json().await.unwrap();
    World {
        app,
        alex,
        sam,
        kim,
        server: s.id,
        ch: ch.id,
    }
}

impl World {
    async fn build(&self, token: &str, body: serde_json::Value) -> Message {
        let r = post_json(
            &self.app,
            token,
            &format!("/channels/{}/builds", self.ch),
            body,
        )
        .await;
        assert_eq!(r.status(), 200);
        r.json().await.unwrap()
    }

    async fn game(&self, name: &str) -> pulse_protocol::rest::Game {
        post_json(
            &self.app,
            &self.alex.1,
            &format!("/servers/{}/games", self.server),
            json!({ "name": name }),
        )
        .await
        .json()
        .await
        .unwrap()
    }

    async fn archive(&self, token: &str, id: MessageId, archived: bool) -> reqwest::Response {
        self.app
            .http
            .put(self.app.url(&format!("/builds/{id}/archived")))
            .bearer_auth(token)
            .json(&json!({ "archived": archived }))
            .send()
            .await
            .unwrap()
    }

    async fn bulk(&self, token: &str, body: serde_json::Value) -> reqwest::Response {
        post_json(
            &self.app,
            token,
            &format!("/channels/{}/builds/archive", self.ch),
            body,
        )
        .await
    }

    async fn archive_many(
        &self,
        token: &str,
        ch: ChannelId,
        ids: &[MessageId],
        archived: bool,
    ) -> reqwest::Response {
        post_json(
            &self.app,
            token,
            &format!("/channels/{ch}/builds/archive-many"),
            json!({ "message_ids": ids, "archived": archived }),
        )
        .await
    }

    async fn delete_many(
        &self,
        token: &str,
        ch: ChannelId,
        ids: &[MessageId],
    ) -> reqwest::Response {
        post_json(
            &self.app,
            token,
            &format!("/channels/{ch}/builds/delete-many"),
            json!({ "message_ids": ids }),
        )
        .await
    }

    async fn delete(&self, token: &str, id: MessageId) -> reqwest::Response {
        self.app
            .http
            .delete(self.app.url(&format!("/builds/{id}")))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
    }

    async fn list(&self, query: &str) -> Vec<MessageId> {
        let v: Vec<Message> = get_json(
            &self.app,
            &self.alex.1,
            &format!("/channels/{}/builds{query}", self.ch),
        )
        .await;
        v.into_iter().map(|m| m.id).collect()
    }

    async fn comment(&self, token: &str, root: MessageId, content: &str) -> reqwest::Response {
        post_json(
            &self.app,
            token,
            &format!("/channels/{}/messages", self.ch),
            json!({ "content": content, "thread_root_id": root }),
        )
        .await
    }

    /// Backdate a build's message, as if saved `days` ago.
    async fn age(&self, id: MessageId, days: i64) {
        let at = pulse_server::db::stamp(jiff::Timestamp::now() - pulse_server::times::days(days));
        sqlx::query("UPDATE messages SET created_at = ? WHERE id = ?")
            .bind(at)
            .bind(id.to_string())
            .execute(&self.app.db)
            .await
            .unwrap();
    }
}

async fn hello(app: &TestApp, token: &str) -> Socket {
    hello_as(app, token, pulse_protocol::PROTOCOL_VERSION).await
}

/// A gateway connection whose Hello says `version`.
async fn hello_as(app: &TestApp, token: &str, version: u32) -> Socket {
    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: version,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        match next_frame_beating(&mut ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Ready(_)) => return ws,
            Some(_) => continue,
            None => panic!("expected Ready"),
        }
    }
}

async fn wait_for<T>(ws: &mut Socket, what: &str, pick: impl Fn(Event) -> Option<T>) -> T {
    loop {
        match next_frame_beating(ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Event(e)) => {
                if let Some(t) = pick(e) {
                    return t;
                }
            }
            Some(_) => continue,
            None => panic!("expected {what}"),
        }
    }
}

fn cutoff(days_ago: i64) -> String {
    pulse_server::times::rfc3339(jiff::Timestamp::now() - pulse_server::times::days(days_ago))
}

#[tokio::test]
async fn only_the_author_or_owner_archives() {
    let w = world().await;
    let m = w.build(&w.sam.1, json!({ "title": "Sam's" })).await;
    assert_eq!(
        w.archive(&w.kim, m.id, true).await.status(),
        404,
        "an outsider"
    );
    let alex_build = w.build(&w.alex.1, json!({ "title": "Alex's" })).await;
    assert_eq!(
        w.archive(&w.sam.1, alex_build.id, true).await.status(),
        403,
        "someone else's"
    );

    let mut ws = hello(&w.app, &w.alex.1).await;
    let r = w.archive(&w.sam.1, m.id, true).await;
    assert_eq!(r.status(), 200);
    let a: Message = r.json().await.unwrap();
    let b = a.build.unwrap();
    assert!(b.archived_at.is_some());
    assert_eq!(b.archived_by, Some(w.sam.0));
    let got = wait_for(&mut ws, "MessageUpdated", |e| match e {
        Event::MessageUpdated { message } if message.id == m.id => Some(message),
        _ => None,
    })
    .await;
    assert!(got.build.unwrap().archived_at.is_some());

    let r = w.archive(&w.sam.1, m.id, false).await;
    let b = r.json::<Message>().await.unwrap().build.unwrap();
    assert_eq!((b.archived_at, b.archived_by), (None, None), "restored");
    // the owner archives anyone's
    let r = w.archive(&w.alex.1, m.id, true).await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Message>()
            .await
            .unwrap()
            .build
            .unwrap()
            .archived_by,
        Some(w.alex.0)
    );
}

#[tokio::test]
async fn archived_builds_are_hidden_unless_asked() {
    let w = world().await;
    let a = w.build(&w.alex.1, json!({ "title": "A" })).await;
    let b = w.build(&w.alex.1, json!({ "title": "B" })).await;
    assert_eq!(w.archive(&w.alex.1, a.id, true).await.status(), 200);
    assert_eq!(w.list("").await, vec![b.id]);
    assert_eq!(w.list("?archived=true").await, vec![b.id, a.id]);
    let one: Message = get_json(&w.app, &w.sam.1, &format!("/builds/{}", a.id)).await;
    assert!(one.build.unwrap().archived_at.is_some());
}

#[tokio::test]
async fn commenting_on_an_archived_build_works() {
    let w = world().await;
    let m = w.build(&w.alex.1, json!({ "title": "Old" })).await;
    assert_eq!(w.archive(&w.alex.1, m.id, true).await.status(), 200);
    let r = w
        .comment(&w.sam.1, m.id, "still works after the patch")
        .await;
    assert_eq!(r.status(), 200);
    let one: Message = get_json(&w.app, &w.sam.1, &format!("/builds/{}", m.id)).await;
    assert_eq!(one.build.unwrap().reply_count, 1);
}

#[tokio::test]
async fn bulk_archive_by_game_with_a_cutoff() {
    let w = world().await;
    let tfd = w.game("The First Descendant").await;
    let wf = w.game("Warframe").await;
    let old1 = w
        .build(&w.alex.1, json!({ "title": "Old 1", "game_id": tfd.id }))
        .await;
    let old2 = w
        .build(&w.sam.1, json!({ "title": "Old 2", "game_id": tfd.id }))
        .await;
    let fresh = w
        .build(&w.alex.1, json!({ "title": "Fresh", "game_id": tfd.id }))
        .await;
    let other = w
        .build(
            &w.alex.1,
            json!({ "title": "Other game", "game_id": wf.id }),
        )
        .await;
    for id in [old1.id, old2.id, other.id] {
        w.age(id, 60).await;
    }
    let body = |dry| json!({ "game_id": tfd.id, "before": cutoff(30), "dry_run": dry });
    let r = w.bulk(&w.alex.1, body(true)).await;
    assert_eq!(r.status(), 200);
    let dry: BulkArchiveResult = r.json().await.unwrap();
    let ids: Vec<_> = dry.builds.iter().map(|b| b.message_id).collect();
    assert_eq!(ids, vec![old2.id, old1.id], "newest first");
    assert_eq!(dry.archived, 0);
    assert_eq!(dry.builds[0].title, "Old 2");
    assert_eq!(w.list("").await.len(), 4, "a dry run archives nothing");

    let mut ws = hello(&w.app, &w.sam.1).await;
    let r = w.bulk(&w.alex.1, body(false)).await;
    let done: BulkArchiveResult = r.json().await.unwrap();
    assert_eq!(done.archived, 2);
    assert_eq!(w.list("").await, vec![other.id, fresh.id]);
    let (got, by) = wait_for(&mut ws, "BuildsArchived", |e| match e {
        Event::BuildsArchived {
            message_ids,
            archived_by,
            ..
        } => Some((message_ids, archived_by)),
        _ => None,
    })
    .await;
    let mut got = got;
    got.sort();
    let mut want = vec![old1.id, old2.id];
    want.sort();
    assert_eq!((got, by), (want, w.alex.0));
    // a bad time is refused
    let r = w.bulk(&w.alex.1, json!({ "before": "last tuesday" })).await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn bulk_archive_for_all_games_only_touches_allowed_builds() {
    let w = world().await;
    let g = w.game("Warframe").await;
    let sams = w
        .build(&w.sam.1, json!({ "title": "Sam's", "game_id": g.id }))
        .await;
    let alexs = w.build(&w.alex.1, json!({ "title": "Alex's" })).await;
    for id in [sams.id, alexs.id] {
        w.age(id, 100).await;
    }
    let body = |dry| json!({ "before": cutoff(90), "dry_run": dry });
    let r = w.bulk(&w.sam.1, body(true)).await;
    let dry: BulkArchiveResult = r.json().await.unwrap();
    assert_eq!(
        dry.builds.iter().map(|b| b.message_id).collect::<Vec<_>>(),
        vec![sams.id]
    );
    let done: BulkArchiveResult = w.bulk(&w.sam.1, body(false)).await.json().await.unwrap();
    assert_eq!(done.archived, 1);
    assert_eq!(w.list("").await, vec![alexs.id], "alex's is untouched");
    // the owner's covers everyone's
    let done: BulkArchiveResult = w.bulk(&w.alex.1, body(false)).await.json().await.unwrap();
    assert_eq!(done.archived, 1);
    assert!(w.list("").await.is_empty());
    // an outsider can't
    assert_eq!(w.bulk(&w.kim, body(true)).await.status(), 404);
}

#[tokio::test]
async fn owner_deletes_any_build_with_its_screenshots_and_comments() {
    let w = world().await;
    // a screenshot
    let bytes = {
        let mut b = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
        b.resize(64, 7);
        b
    };
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/attachments", w.ch),
        json!({ "name": "s.webp", "size": bytes.len(), "content_type": "image/webp" }),
    )
    .await;
    let up: pulse_protocol::rest::AttachmentUpload = r.json().await.unwrap();
    reqwest::Client::new()
        .put(up.url.clone().unwrap())
        .header("content-type", &up.content_type)
        .body(bytes)
        .send()
        .await
        .unwrap();
    let m = w
        .build(
            &w.sam.1,
            json!({ "title": "Sam's", "attachments": [up.id] }),
        )
        .await;
    let key: String = sqlx::query_scalar("SELECT key FROM attachments WHERE id = ?")
        .bind(up.id.to_string())
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    let object = pulse_server::attachments::object_key(&key, false);
    let c: Message = w
        .comment(&w.alex.1, m.id, "nice")
        .await
        .json()
        .await
        .unwrap();

    let (_, jo) = register(&w.app, "jo").await;
    assert_eq!(join_server(&w.app, &jo, w.server).await.status(), 200);
    assert_eq!(w.delete(&jo, m.id).await.status(), 403, "not theirs");
    assert_eq!(w.delete(&w.kim, m.id).await.status(), 404);

    let mut ws = hello(&w.app, &w.sam.1).await;
    assert_eq!(w.delete(&w.alex.1, m.id).await.status(), 204);
    let mut gone = vec![];
    while gone.len() < 2 {
        let id = wait_for(&mut ws, "MessageDeleted", |e| match e {
            Event::MessageDeleted { message_id, .. } => Some(message_id),
            _ => None,
        })
        .await;
        gone.push(id);
    }
    assert!(gone.contains(&m.id) && gone.contains(&c.id));
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM builds WHERE message_id = ?")
        .bind(m.id.to_string())
        .fetch_one(&w.app.db)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    let comment_live: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE id = ? AND deleted_at IS NULL")
            .bind(c.id.to_string())
            .fetch_one(&w.app.db)
            .await
            .unwrap();
    assert_eq!(comment_live, 0);
    assert!(w.app.s3.as_ref().unwrap().object(&object).is_none());
    let r = w
        .app
        .http
        .get(w.app.url(&format!("/builds/{}", m.id)))
        .bearer_auth(&w.sam.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    assert!(w.list("?archived=true").await.is_empty());
}

#[tokio::test]
async fn the_author_deletes_their_own() {
    let w = world().await;
    let m = w.build(&w.sam.1, json!({ "title": "Sam's" })).await;
    assert_eq!(w.delete(&w.sam.1, m.id).await.status(), 204);
    assert!(w.list("").await.is_empty());
}

#[tokio::test]
async fn facets_count_games_and_respect_archived() {
    let w = world().await;
    let tfd = w.game("The First Descendant").await;
    let wf = w.game("Warframe").await;
    w.build(&w.alex.1, json!({ "title": "A", "game_id": tfd.id }))
        .await;
    let b = w
        .build(&w.alex.1, json!({ "title": "B", "game_id": tfd.id }))
        .await;
    w.build(&w.alex.1, json!({ "title": "C", "game_id": wf.id }))
        .await;
    w.build(&w.alex.1, json!({ "title": "No game" })).await;
    assert_eq!(w.archive(&w.alex.1, b.id, true).await.status(), 200);
    let f: BuildFacets = get_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/builds/facets", w.ch),
    )
    .await;
    assert_eq!(f.total, 3);
    let counts: Vec<_> = f.games.iter().map(|g| (g.game_id, g.count)).collect();
    assert_eq!(counts, vec![(tfd.id, 1), (wf.id, 1)]);
    let f: BuildFacets = get_json(
        &w.app,
        &w.sam.1,
        &format!("/channels/{}/builds/facets?archived=true", w.ch),
    )
    .await;
    assert_eq!(f.total, 4);
    let counts: Vec<_> = f.games.iter().map(|g| (g.game_id, g.count)).collect();
    assert_eq!(counts, vec![(tfd.id, 2), (wf.id, 1)]);
    let r = w
        .app
        .http
        .get(w.app.url(&format!("/channels/{}/builds/facets", w.ch)))
        .bearer_auth(&w.kim)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

/// Your own build is yours to delete after you leave the server, as any message of yours is (#418); archiving it is
/// for members.
#[tokio::test]
async fn an_author_who_left_still_deletes_their_build() {
    let w = world().await;
    let m = w.build(&w.sam.1, json!({ "title": "Sam's" })).await;
    let other = w.build(&w.sam.1, json!({ "title": "Sam's other" })).await;
    let r = post_json(
        &w.app,
        &w.sam.1,
        &format!("/servers/{}/leave", w.server),
        json!({}),
    )
    .await;
    assert!(r.status().is_success(), "leave: {}", r.status());
    assert_eq!(
        w.archive(&w.sam.1, other.id, true).await.status(),
        403,
        "archiving is for members"
    );
    assert_eq!(w.delete(&w.sam.1, m.id).await.status(), 204);
    assert_eq!(w.list("").await, vec![other.id]);
}

async fn batch(r: reqwest::Response) -> BuildsBatchResult {
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

fn sorted(mut v: Vec<MessageId>) -> Vec<MessageId> {
    v.sort();
    v
}

/// Archive-many (#617): each build by the single route's rule (its author, or the owner); the rest are skipped, one
/// `BuildsArchived` names the ones it changed, and a restore sends each its `MessageUpdated`.
#[tokio::test]
async fn archive_many_does_only_the_builds_you_may() {
    let w = world().await;
    let a = w.build(&w.sam.1, json!({ "title": "Sam's A" })).await;
    let b = w.build(&w.alex.1, json!({ "title": "Alex's" })).await;
    let c = w.build(&w.sam.1, json!({ "title": "Sam's C" })).await;
    let mut ws = hello(&w.app, &w.alex.1).await;
    let got = batch(
        w.archive_many(&w.sam.1, w.ch, &[a.id, b.id, c.id], true)
            .await,
    )
    .await;
    assert_eq!(
        got,
        BuildsBatchResult {
            done: 2,
            skipped: 1
        }
    );
    assert_eq!(w.list("").await, vec![b.id], "alex's is untouched");
    let (ids, by) = wait_for(&mut ws, "BuildsArchived", |e| match e {
        Event::BuildsArchived {
            message_ids,
            archived_by,
            ..
        } => Some((message_ids, archived_by)),
        _ => None,
    })
    .await;
    assert_eq!((sorted(ids), by), (sorted(vec![a.id, c.id]), w.sam.0));

    // the owner may archive all of them; the ones already archived count as done, the event names only the change
    let got = batch(
        w.archive_many(&w.alex.1, w.ch, &[a.id, b.id, c.id], true)
            .await,
    )
    .await;
    assert_eq!(
        got,
        BuildsBatchResult {
            done: 3,
            skipped: 0
        }
    );
    let ids = wait_for(&mut ws, "BuildsArchived", |e| match e {
        Event::BuildsArchived { message_ids, .. } => Some(message_ids),
        _ => None,
    })
    .await;
    assert_eq!(ids, vec![b.id]);
    let one: Message = get_json(&w.app, &w.sam.1, &format!("/builds/{}", a.id)).await;
    assert_eq!(
        one.build.unwrap().archived_by,
        Some(w.sam.0),
        "who archived it first is kept"
    );

    // restoring: sam gets theirs back, not alex's
    let got = batch(w.archive_many(&w.sam.1, w.ch, &[a.id, b.id], false).await).await;
    assert_eq!(
        got,
        BuildsBatchResult {
            done: 1,
            skipped: 1
        }
    );
    let back = wait_for(&mut ws, "MessageUpdated", |e| match e {
        Event::MessageUpdated { message } => Some(message),
        _ => None,
    })
    .await;
    assert_eq!(back.id, a.id);
    assert!(back.build.unwrap().archived_at.is_none());
    assert_eq!(w.list("").await, vec![a.id]);
    // an outsider can't
    assert_eq!(
        w.archive_many(&w.kim, w.ch, &[a.id], true).await.status(),
        404
    );
}

#[tokio::test]
async fn delete_many_does_only_the_builds_you_may() {
    let w = world().await;
    let mine = w.build(&w.sam.1, json!({ "title": "Sam's" })).await;
    let theirs = w.build(&w.alex.1, json!({ "title": "Alex's" })).await;
    let c: Message = w
        .comment(&w.alex.1, mine.id, "nice")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(w.delete_many(&w.kim, w.ch, &[mine.id]).await.status(), 404);
    let mut ws = hello(&w.app, &w.alex.1).await;
    let got = batch(w.delete_many(&w.sam.1, w.ch, &[mine.id, theirs.id]).await).await;
    assert_eq!(
        got,
        BuildsBatchResult {
            done: 1,
            skipped: 1
        }
    );
    let mut gone = vec![];
    while gone.len() < 2 {
        gone.push(
            wait_for(&mut ws, "MessageDeleted", |e| match e {
                Event::MessageDeleted { message_id, .. } => Some(message_id),
                _ => None,
            })
            .await,
        );
    }
    assert_eq!(
        sorted(gone),
        sorted(vec![mine.id, c.id]),
        "the build and its comment"
    );
    assert_eq!(w.list("?archived=true").await, vec![theirs.id]);
    // the owner deletes anyone's; one already gone is skipped
    let got = batch(w.delete_many(&w.alex.1, w.ch, &[mine.id, theirs.id]).await).await;
    assert_eq!(
        got,
        BuildsBatchResult {
            done: 1,
            skipped: 1
        }
    );
    assert!(w.list("?archived=true").await.is_empty());
}

#[tokio::test]
async fn batches_are_capped_at_100() {
    let w = world().await;
    let m = w.build(&w.alex.1, json!({ "title": "A" })).await;
    let mut ids: Vec<MessageId> = (0..100).map(|_| MessageId::new()).collect();
    ids[0] = m.id;
    let got = batch(w.archive_many(&w.alex.1, w.ch, &ids, true).await).await;
    assert_eq!(
        got,
        BuildsBatchResult {
            done: 1,
            skipped: 99
        }
    );
    ids.push(MessageId::new());
    assert_eq!(
        w.archive_many(&w.alex.1, w.ch, &ids, true).await.status(),
        400
    );
    assert_eq!(w.delete_many(&w.alex.1, w.ch, &ids).await.status(), 400);
    assert_eq!(
        w.list("?archived=true").await,
        vec![m.id],
        "nothing deleted"
    );
    // a repeated id is one build
    let got = batch(w.delete_many(&w.alex.1, w.ch, &[m.id, m.id]).await).await;
    assert_eq!(
        got,
        BuildsBatchResult {
            done: 1,
            skipped: 0
        }
    );
}

/// A batch on one channel never reaches a build of another: it's skipped.
#[tokio::test]
async fn builds_of_another_channel_are_skipped() {
    let w = world().await;
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/channels", w.server),
        json!({ "kind": "builds", "name": "more-builds" }),
    )
    .await;
    let other: Channel = r.json().await.unwrap();
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/channels/{}/builds", other.id),
        json!({ "title": "Elsewhere" }),
    )
    .await;
    let x: Message = r.json().await.unwrap();
    let got = batch(w.archive_many(&w.alex.1, w.ch, &[x.id], true).await).await;
    assert_eq!(
        got,
        BuildsBatchResult {
            done: 0,
            skipped: 1
        }
    );
    let got = batch(w.delete_many(&w.alex.1, w.ch, &[x.id]).await).await;
    assert_eq!(
        got,
        BuildsBatchResult {
            done: 0,
            skipped: 1
        }
    );
    let one: Message = get_json(&w.app, &w.alex.1, &format!("/builds/{}", x.id)).await;
    assert!(one.build.unwrap().archived_at.is_none());
    // and a plain chat channel isn't a builds channel
    let text = general(&w.app, &w.alex.1, w.server).await;
    assert_eq!(
        w.archive_many(&w.alex.1, text.id, &[x.id], true)
            .await
            .status(),
        400
    );
}

/// A version-4 client hears nothing of a batch (#616): neither its `BuildsArchived` nor its `MessageDeleted`s.
#[tokio::test]
async fn a_version_4_client_hears_nothing_of_a_batch() {
    let w = world().await;
    let text = general(&w.app, &w.alex.1, w.server).await;
    let a = w.build(&w.alex.1, json!({ "title": "A" })).await;
    let b = w.build(&w.alex.1, json!({ "title": "B" })).await;
    let mut old = hello_as(&w.app, &w.sam.1, 4).await;
    batch(w.archive_many(&w.alex.1, w.ch, &[a.id], true).await).await;
    batch(w.archive_many(&w.alex.1, w.ch, &[a.id], false).await).await;
    batch(w.delete_many(&w.alex.1, w.ch, &[a.id, b.id]).await).await;
    send(&w.app, &w.alex.1, text.id, "after").await;
    let id = w.ch.to_string();
    loop {
        match next_frame_beating(&mut old, Duration::from_secs(3)).await {
            Some(ServerFrame::Event(Event::MessageCreated { message, .. }))
                if message.content == "after" =>
            {
                break;
            }
            Some(ServerFrame::Event(e)) => {
                let s = serde_json::to_string(&e).unwrap();
                assert!(!s.contains(&id) && !s.contains(&a.id.to_string()), "{s}");
            }
            Some(_) => continue,
            None => panic!("expected the sentinel"),
        }
    }
}
