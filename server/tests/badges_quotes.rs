//! Badges (#49) from quotes, quote awards, Quote of the Year and renames.
mod common;

use common::*;
use pulse_protocol::ids::{ChannelId, LineId, ServerId, UserId};
use pulse_protocol::rest::{Badge, Channel, Message, Profile, QotyEvent};
use pulse_server::badges::{catalogue::stat, stats};
use serde_json::json;

async fn stat_of(app: &TestApp, user: UserId, key: &str) -> i64 {
    let mut c = app.db.acquire().await.unwrap();
    stats::get(&mut c, user, key).await.unwrap()
}

async fn set_stat(app: &TestApp, user: UserId, key: &str, value: i64) {
    let mut c = app.db.acquire().await.unwrap();
    stats::set(&mut c, user, key, value).await.unwrap();
}

async fn badge(app: &TestApp, token: &str, user: UserId, key: &str) -> Option<Badge> {
    get_json::<Profile>(app, token, &format!("/users/{user}/profile"))
        .await
        .badges
        .into_iter()
        .find(|b| b.key == key)
}

fn vendetta(on: UserId) -> String {
    format!("{}{on}", stat::VENDETTA_PREFIX)
}

/// alex and sam in alex's server "Main", with a quotes channel.
struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    server: ServerId,
    quotes: ChannelId,
}

async fn world() -> World {
    let app = spawn().await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let s = create_server(&app, &alex.1, "Main").await;
    let r = post_json(&app, &sam.1, &format!("/servers/{}/join", s.id), json!({})).await;
    assert_eq!(r.status(), 204);
    let r = post_json(
        &app,
        &alex.1,
        &format!("/servers/{}/channels", s.id),
        json!({ "kind": "quotes", "name": "quotes" }),
    )
    .await;
    assert_eq!(r.status(), 200, "create quotes channel");
    let quotes = r.json::<Channel>().await.unwrap().id;
    World {
        app,
        alex,
        sam,
        server: s.id,
        quotes,
    }
}

impl World {
    async fn join(&self, token: &str) {
        let r = post_json(
            &self.app,
            token,
            &format!("/servers/{}/join", self.server),
            json!({}),
        )
        .await;
        assert_eq!(r.status(), 204);
    }
    async fn member(&self, name: &str) -> (UserId, String) {
        let m = register(&self.app, name).await;
        self.join(&m.1).await;
        m
    }
    /// A quote `token` posts with these lines.
    async fn quote(&self, token: &str, lines: serde_json::Value) -> Message {
        let r = post_json(
            &self.app,
            token,
            &format!("/channels/{}/quotes", self.quotes),
            json!({ "lines": lines }),
        )
        .await;
        assert_eq!(r.status(), 200, "post quote");
        r.json().await.unwrap()
    }
    /// alex quotes one line from each of `speakers`.
    async fn alex_quotes(&self, speakers: &[UserId]) -> Message {
        let lines: Vec<_> = speakers
            .iter()
            .map(|s| json!({ "speaker_id": s, "text": "something" }))
            .collect();
        self.quote(&self.alex.1, json!(lines)).await
    }
    async fn award(&self, token: &str, line: LineId, give: bool) -> u16 {
        let url = self.app.url(&format!("/quote-lines/{line}/award"));
        let r = if give {
            self.app.http.put(url)
        } else {
            self.app.http.delete(url)
        };
        r.bearer_auth(token).send().await.unwrap().status().as_u16()
    }
    async fn rename(&self, token: &str, user: UserId, nickname: Option<&str>) {
        let r = self
            .app
            .http
            .put(
                self.app
                    .url(&format!("/servers/{}/members/{user}/nickname", self.server)),
            )
            .bearer_auth(token)
            .json(&json!({ "nickname": nickname }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "rename");
    }
}

#[tokio::test]
async fn friendly_fire_for_quoting_yourself() {
    let w = world().await;
    w.alex_quotes(&[w.alex.0]).await;
    assert!(
        badge(&w.app, &w.alex.1, w.alex.0, "friendly_fire")
            .await
            .is_some()
    );
    assert_eq!(stat_of(&w.app, w.alex.0, stat::QUOTED_OTHERS).await, 0);
    assert_eq!(stat_of(&w.app, w.alex.0, &vendetta(w.alex.0)).await, 0);
}

#[tokio::test]
async fn john_quoter_counts_quotes_not_lines() {
    let w = world().await;
    w.alex_quotes(&[w.sam.0, w.sam.0]).await;
    assert_eq!(stat_of(&w.app, w.alex.0, stat::QUOTED_OTHERS).await, 1);
    assert_eq!(stat_of(&w.app, w.alex.0, &vendetta(w.sam.0)).await, 1);
    // Yourself and sam in one quote: both count, once each.
    w.alex_quotes(&[w.alex.0, w.sam.0]).await;
    assert_eq!(stat_of(&w.app, w.alex.0, stat::QUOTED_OTHERS).await, 2);
    assert_eq!(stat_of(&w.app, w.alex.0, stat::FRIENDLY_FIRE).await, 1);
    // A plain name is nobody's.
    w.quote(
        &w.alex.1,
        json!([{ "speaker_name": "A Friend", "text": "hi" }]),
    )
    .await;
    assert_eq!(stat_of(&w.app, w.alex.0, stat::QUOTED_OTHERS).await, 2);
    // Nothing for the person quoted.
    assert_eq!(stat_of(&w.app, w.sam.0, stat::QUOTED_OTHERS).await, 0);
}

#[tokio::test]
async fn personal_vendetta_at_twenty_of_one_person() {
    let w = world().await;
    let kim = w.member("kim").await;
    set_stat(&w.app, w.alex.0, &vendetta(w.sam.0), 19).await;
    set_stat(&w.app, w.alex.0, stat::VENDETTA_MAX, 19).await;
    w.alex_quotes(&[w.sam.0]).await;
    assert_eq!(stat_of(&w.app, w.alex.0, stat::VENDETTA_MAX).await, 20);
    assert!(
        badge(&w.app, &w.alex.1, w.alex.0, "personal_vendetta")
            .await
            .is_some()
    );
    w.alex_quotes(&[kim.0]).await;
    w.alex_quotes(&[kim.0]).await;
    assert_eq!(stat_of(&w.app, w.alex.0, &vendetta(kim.0)).await, 2);
    assert_eq!(stat_of(&w.app, w.alex.0, stat::VENDETTA_MAX).await, 20);
}

#[tokio::test]
async fn quote_awards_follow_give_and_take_back() {
    let w = world().await;
    let kim = w.member("kim").await;
    let q = w.alex_quotes(&[w.sam.0]).await;
    let line = q.quote.unwrap().lines[0].id;
    assert_eq!(w.award(&kim.1, line, true).await, 204);
    assert_eq!(w.award(&kim.1, line, true).await, 204);
    assert_eq!(w.award(&w.alex.1, line, true).await, 204);
    assert_eq!(stat_of(&w.app, w.sam.0, stat::QUOTE_AWARDS).await, 2);
    assert_eq!(w.award(&kim.1, line, false).await, 204);
    assert_eq!(w.award(&kim.1, line, false).await, 204);
    assert_eq!(stat_of(&w.app, w.sam.0, stat::QUOTE_AWARDS).await, 1);
    // Your own line can't be awarded.
    assert_eq!(w.award(&w.sam.1, line, true).await, 403);
    assert_eq!(stat_of(&w.app, w.sam.0, stat::QUOTE_AWARDS).await, 1);

    // The tenth award unlocks Quotable.
    set_stat(&w.app, w.sam.0, stat::QUOTE_AWARDS, 9).await;
    assert_eq!(w.award(&kim.1, line, true).await, 204);
    assert!(badge(&w.app, &w.sam.1, w.sam.0, "quotable").await.is_some());
}

#[tokio::test]
async fn qoty_win_counts_and_reopen_takes_it_back() {
    let w = world().await;
    let (fan, _) = register(&w.app, "kim").await;
    let q = w.alex_quotes(&[w.sam.0]).await;
    // A reaction score, so the quote is an option.
    sqlx::query(
        "INSERT INTO reactions (message_id, user_id, emoji, created_at)
         VALUES (?, ?, '😂', '2026-01-01T00:00:00.000Z')",
    )
    .bind(q.id.to_string())
    .bind(fan.to_string())
    .execute(&w.app.db)
    .await
    .unwrap();
    let year = chrono::Datelike::year(&chrono::Utc::now());
    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/servers/{}/qoty", w.server),
        json!({ "year": year, "minutes": null }),
    )
    .await;
    assert_eq!(r.status(), 200, "start");
    let e: QotyEvent = r.json().await.unwrap();
    let r = w
        .app
        .http
        .put(w.app.url(&format!("/qoty/{}/vote", e.id)))
        .bearer_auth(&w.alex.1)
        .json(&json!({ "message_id": q.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "vote");
    let r = post_json(&w.app, &w.alex.1, &format!("/qoty/{}/end", e.id), json!({})).await;
    assert_eq!(r.status(), 200, "end");

    assert_eq!(stat_of(&w.app, w.sam.0, stat::QOTY_WINS).await, 1);
    let won = badge(&w.app, &w.sam.1, w.sam.0, "quote_of_the_year")
        .await
        .expect("won");
    assert_eq!(won.count, 1);
    // The poster didn't win anything.
    assert_eq!(stat_of(&w.app, w.alex.0, stat::QOTY_WINS).await, 0);

    let r = post_json(
        &w.app,
        &w.alex.1,
        &format!("/qoty/{}/reopen", e.id),
        json!({ "minutes": null }),
    )
    .await;
    assert_eq!(r.status(), 200, "reopen");
    assert_eq!(stat_of(&w.app, w.sam.0, stat::QOTY_WINS).await, 0);
    assert!(
        badge(&w.app, &w.sam.1, w.sam.0, "quote_of_the_year")
            .await
            .is_some(),
        "badges are never taken back"
    );

    // Ending again counts the win again.
    let r = post_json(&w.app, &w.alex.1, &format!("/qoty/{}/end", e.id), json!({})).await;
    assert_eq!(r.status(), 200, "end again");
    assert_eq!(stat_of(&w.app, w.sam.0, stat::QOTY_WINS).await, 1);
}

#[tokio::test]
async fn renamed_by_others_only() {
    let w = world().await;
    w.rename(&w.sam.1, w.alex.0, Some("Captain")).await;
    w.rename(&w.sam.1, w.alex.0, Some("Major")).await;
    // The same name again changes nothing.
    w.rename(&w.sam.1, w.alex.0, Some("Major")).await;
    w.rename(&w.alex.1, w.alex.0, Some("Me")).await;
    w.rename(&w.sam.1, w.alex.0, None).await;
    assert_eq!(stat_of(&w.app, w.alex.0, stat::RENAMED_BY_OTHERS).await, 2);
    assert_eq!(stat_of(&w.app, w.sam.0, stat::RENAMED_BY_OTHERS).await, 0);

    set_stat(&w.app, w.alex.0, stat::RENAMED_BY_OTHERS, 9).await;
    w.rename(&w.sam.1, w.alex.0, Some("Tenth")).await;
    assert!(
        badge(&w.app, &w.alex.1, w.alex.0, "identity_crisis")
            .await
            .is_some()
    );
}
