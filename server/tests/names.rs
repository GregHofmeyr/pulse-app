//! Name layers (#47, #48): display names, server nicknames and the Name Archive, private
//! nicknames, and the names the server writes itself.
mod common;

use std::borrow::Cow;
use std::time::Duration;

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{
    ApiError, Member, Message, NameArchivePage, NameStyle, Profile, UpdateProfileRequest,
};
use pulse_server::limits::{LimitsConfig, Rate};
use serde_json::json;
use sqlx::sqlite::SqliteConnectOptions;
use tokio_tungstenite::tungstenite::Message as Ws;

/// A gateway connection that has said hello, and the `Ready` it got.
async fn hello(app: &TestApp, token: &str) -> (Socket, Ready) {
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
        match next_frame_beating(&mut ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Ready(r)) => return (ws, r),
            Some(_) => continue,
            None => panic!("expected Ready"),
        }
    }
}

/// Every event `ws` gets until one matches `stop` (which is returned last), as JSON text too.
async fn events_until(ws: &mut Socket, stop: impl Fn(&Event) -> bool) -> Vec<(Event, String)> {
    let mut out = vec![];
    loop {
        match next_frame_beating(ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Event(e)) => {
                let text = serde_json::to_string(&e).unwrap();
                let done = stop(&e);
                out.push((e, text));
                if done {
                    return out;
                }
            }
            Some(_) => continue,
            None => panic!("socket went quiet before the expected event"),
        }
    }
}

async fn put_json(
    app: &TestApp,
    token: &str,
    path: &str,
    body: serde_json::Value,
) -> reqwest::Response {
    app.http
        .put(app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn join(app: &TestApp, token: &str, server: ServerId) {
    let r = post_json(app, token, &format!("/servers/{server}/join"), json!({})).await;
    assert_eq!(r.status(), 204);
}

async fn rename(
    app: &TestApp,
    token: &str,
    server: ServerId,
    user: UserId,
    nickname: Option<&str>,
) -> reqwest::Response {
    put_json(
        app,
        token,
        &format!("/servers/{server}/members/{user}/nickname"),
        json!({ "nickname": nickname }),
    )
    .await
}

async fn private_nick(
    app: &TestApp,
    token: &str,
    user: UserId,
    nickname: Option<&str>,
) -> reqwest::Response {
    put_json(
        app,
        token,
        &format!("/me/private-nicknames/{user}"),
        json!({ "nickname": nickname }),
    )
    .await
}

fn profile_request(display_name: Option<&str>) -> UpdateProfileRequest {
    UpdateProfileRequest {
        avatar_hash: None,
        banner_hash: None,
        theme_top: "#8a3cff".into(),
        theme_bottom: "#ff8a2a".into(),
        style: NameStyle::default(),
        pronouns: None,
        bio: None,
        display_name: display_name.map(str::to_owned),
    }
}

async fn put_profile(app: &TestApp, token: &str, req: &UpdateProfileRequest) -> reqwest::Response {
    app.http
        .put(app.url("/me/profile"))
        .bearer_auth(token)
        .json(req)
        .send()
        .await
        .unwrap()
}

async fn error_text(r: reqwest::Response) -> String {
    r.json::<ApiError>().await.unwrap().message
}

// ---------------------------------------------------------------------------------------------
// Display names

#[tokio::test]
async fn migration_0013_copies_usernames_into_display_names() {
    let dir = tempfile::tempdir().unwrap();
    let opts = SqliteConnectOptions::new()
        .filename(dir.path().join("old.db"))
        .create_if_missing(true)
        .foreign_keys(false);
    let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    let all = migrator.migrations.clone();
    let names = all
        .iter()
        .find(|m| m.description.contains("name layers"))
        .expect("the name layers migration exists")
        .version;
    // A database as the previous release left it.
    migrator.migrations = Cow::Owned(all.iter().filter(|m| m.version < names).cloned().collect());
    migrator.run(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, created_at) \
         VALUES ('01J0000000000000000000USER', 'alex', 'x', '2026-10-01T00:00:00.000Z')",
    )
    .execute(&pool)
    .await
    .unwrap();

    migrator.migrations = all;
    migrator.run(&pool).await.unwrap();
    let dn: String = sqlx::query_scalar("SELECT display_name FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(dn, "alex");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM private_nicknames")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn new_users_start_with_their_username_as_display_name() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let me: pulse_protocol::rest::User = get_json(&app, &alex, "/me").await;
    assert_eq!(me.display_name, "alex");
    let (_, ready) = hello(&app, &alex).await;
    assert_eq!(ready.me.display_name, "alex");
    let p: Profile = get_json(&app, &alex, &format!("/users/{alex_id}/profile")).await;
    assert_eq!(p.user.display_name, "alex");
}

#[tokio::test]
async fn a_display_name_saves_and_reaches_everyone_live() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let (mut sam_ws, _) = hello(&app, &sam).await;

    let r = put_profile(&app, &alex, &profile_request(Some("  Alex 🎸 the Great  "))).await;
    assert_eq!(r.status(), 200);
    let p: Profile = r.json().await.unwrap();
    assert_eq!(p.user.display_name, "Alex 🎸 the Great");
    assert_eq!(p.user.username, "alex");

    let got = events_until(&mut sam_ws, |e| matches!(e, Event::ProfileUpdated { .. })).await;
    let Event::ProfileUpdated { user, .. } = &got.last().unwrap().0 else {
        unreachable!()
    };
    assert_eq!(user.id, alex_id);
    assert_eq!(user.display_name, "Alex 🎸 the Great");

    // Saving without the field (an older client) keeps it.
    let r = put_profile(&app, &alex, &profile_request(None)).await;
    assert_eq!(r.status(), 200);
    let p: Profile = r.json().await.unwrap();
    assert_eq!(p.user.display_name, "Alex 🎸 the Great");
}

#[tokio::test]
async fn bad_display_names_are_refused_with_words() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    for bad in ["", "   ", "line\nbreak", &"x".repeat(33)] {
        let r = put_profile(&app, &alex, &profile_request(Some(bad))).await;
        assert_eq!(r.status(), 400, "{bad:?}");
        assert!(error_text(r).await.contains("Names"), "{bad:?}");
    }
    let r = put_profile(&app, &alex, &profile_request(Some(&"x".repeat(32)))).await;
    assert_eq!(r.status(), 200);
}

// ---------------------------------------------------------------------------------------------
// Server nicknames and the Name Archive

struct Crew {
    app: TestApp,
    server: ServerId,
    alex: (UserId, String),
    sam: (UserId, String),
    kim: (UserId, String),
    jo: (UserId, String),
}

async fn crew_with(cfg: pulse_server::config::Config) -> Crew {
    let app = spawn_with(cfg).await;
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let kim = register(&app, "kim").await;
    let jo = register(&app, "jo").await;
    let server = create_server(&app, &alex.1, "Main").await.id;
    for t in [&sam.1, &kim.1, &jo.1] {
        join(&app, t, server).await;
    }
    Crew {
        app,
        server,
        alex,
        sam,
        kim,
        jo,
    }
}

async fn crew() -> Crew {
    crew_with(pulse_server::testing::test_config()).await
}

#[tokio::test]
async fn anyone_can_rename_anyone_quietly_and_it_is_archived() {
    let c = crew().await;
    let (mut jo_ws, _) = hello(&c.app, &c.jo.1).await;

    // sam names alex; alex renames themself; kim resets it.
    let r = rename(&c.app, &c.sam.1, c.server, c.alex.0, Some("  Captain  ")).await;
    assert_eq!(r.status(), 200);
    let m: Member = r.json().await.unwrap();
    assert_eq!(m.nickname.as_deref(), Some("Captain"));
    assert_eq!(m.user.id, c.alex.0);
    let got = events_until(&mut jo_ws, |e| matches!(e, Event::MemberUpdated { .. })).await;
    let Event::MemberUpdated { server_id, member } = &got.last().unwrap().0 else {
        unreachable!()
    };
    assert_eq!(*server_id, c.server);
    assert_eq!(member.nickname.as_deref(), Some("Captain"));

    assert_eq!(
        rename(&c.app, &c.alex.1, c.server, c.alex.0, Some("Al"))
            .await
            .status(),
        200
    );
    assert_eq!(
        rename(&c.app, &c.kim.1, c.server, c.alex.0, None)
            .await
            .status(),
        200
    );
    let got = events_until(
        &mut jo_ws,
        |e| matches!(e, Event::MemberUpdated { member, .. } if member.nickname.is_none()),
    )
    .await;
    // Quiet: no system line anywhere.
    assert!(
        !got.iter()
            .any(|(e, _)| matches!(e, Event::MessageCreated { .. })),
        "a rename posted a message"
    );
    let general = general(&c.app, &c.alex.1, c.server).await;
    let msgs: Vec<Message> = get_json(
        &c.app,
        &c.alex.1,
        &format!("/channels/{}/messages", general.id),
    )
    .await;
    assert!(msgs.is_empty(), "{msgs:?}");

    let rows: Vec<(Option<String>, Option<String>, String)> = sqlx::query_as(
        "SELECT old_nick, new_nick, changed_by FROM nickname_history \
         WHERE server_id = ? AND target_user_id = ? ORDER BY changed_at, id",
    )
    .bind(c.server.to_string())
    .bind(c.alex.0.to_string())
    .fetch_all(&c.app.db)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            (None, Some("Captain".into()), c.sam.0.to_string()),
            (
                Some("Captain".into()),
                Some("Al".into()),
                c.alex.0.to_string()
            ),
            (Some("Al".into()), None, c.kim.0.to_string()),
        ]
    );

    let page: NameArchivePage = get_json(
        &c.app,
        &c.jo.1,
        &format!("/servers/{}/members/{}/names", c.server, c.alex.0),
    )
    .await;
    assert_eq!(page.total, 3);
    let names: Vec<_> = page.entries.iter().map(|e| e.name.as_deref()).collect();
    assert_eq!(names, [None, Some("Al"), Some("Captain")]);
    assert_eq!(page.entries[2].changed_by, c.sam.0);
    assert!(page.next_cursor.is_none());

    let p: Profile = get_json(&c.app, &c.jo.1, &format!("/users/{}/profile", c.alex.0)).await;
    assert_eq!(p.stats.times_renamed, 3);
}

#[tokio::test]
async fn a_rename_to_the_same_name_changes_nothing() {
    let c = crew().await;
    assert_eq!(
        rename(&c.app, &c.sam.1, c.server, c.kim.0, Some("Kimmy"))
            .await
            .status(),
        200
    );
    assert_eq!(
        rename(&c.app, &c.sam.1, c.server, c.kim.0, Some(" Kimmy "))
            .await
            .status(),
        200
    );
    assert_eq!(
        rename(&c.app, &c.sam.1, c.server, c.jo.0, None)
            .await
            .status(),
        200
    );
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nickname_history")
        .fetch_one(&c.app.db)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn renaming_needs_both_people_in_the_server() {
    let c = crew().await;
    let (outsider_id, outsider) = register(&c.app, "outsider").await;
    // Not a member: refused.
    assert_eq!(
        rename(&c.app, &outsider, c.server, c.alex.0, Some("x"))
            .await
            .status(),
        403
    );
    // Target not in the server.
    assert_eq!(
        rename(&c.app, &c.alex.1, c.server, outsider_id, Some("x"))
            .await
            .status(),
        404
    );
    // No such server.
    assert_eq!(
        rename(&c.app, &c.alex.1, ServerId::new(), c.sam.0, Some("x"))
            .await
            .status(),
        404
    );
    // The archive is for members only.
    let r = c
        .app
        .http
        .get(
            c.app
                .url(&format!("/servers/{}/members/{}/names", c.server, c.alex.0)),
        )
        .bearer_auth(&outsider)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    // Same length rules as display names.
    let r = rename(&c.app, &c.alex.1, c.server, c.sam.0, Some(&"y".repeat(33))).await;
    assert_eq!(r.status(), 400);
    let r = rename(&c.app, &c.alex.1, c.server, c.sam.0, Some("a\tb")).await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn renames_have_a_generous_rate_limit_that_says_so() {
    let mut cfg = pulse_server::testing::test_config();
    cfg.limits = LimitsConfig {
        rename_user: Rate::per_minute(2),
        ..LimitsConfig::unlimited()
    };
    let c = crew_with(cfg).await;
    assert_eq!(
        rename(&c.app, &c.sam.1, c.server, c.kim.0, Some("a"))
            .await
            .status(),
        200
    );
    assert_eq!(
        rename(&c.app, &c.sam.1, c.server, c.kim.0, Some("b"))
            .await
            .status(),
        200
    );
    let r = rename(&c.app, &c.sam.1, c.server, c.kim.0, Some("c")).await;
    assert_eq!(r.status(), 429);
    assert!(error_text(r).await.contains("renaming too fast"));
    // Per renamer: kim still can.
    assert_eq!(
        rename(&c.app, &c.kim.1, c.server, c.kim.0, Some("d"))
            .await
            .status(),
        200
    );
    // The default stops only real abuse.
    assert_eq!(LimitsConfig::default().rename_user, Rate::per_minute(30));
}

#[tokio::test]
async fn the_archive_pages_newest_first() {
    let c = crew().await;
    for i in 0..55 {
        let r = rename(&c.app, &c.sam.1, c.server, c.kim.0, Some(&format!("n{i}"))).await;
        assert_eq!(r.status(), 200);
    }
    let path = format!("/servers/{}/members/{}/names", c.server, c.kim.0);
    let first: NameArchivePage = get_json(&c.app, &c.jo.1, &path).await;
    assert_eq!(first.total, 55);
    assert_eq!(first.entries.len(), 50);
    assert_eq!(first.entries[0].name.as_deref(), Some("n54"));
    assert_eq!(first.entries[49].name.as_deref(), Some("n5"));
    let cursor = first.next_cursor.expect("more to come");
    let url = reqwest::Url::parse_with_params(&c.app.url(&path), &[("cursor", &cursor)]).unwrap();
    let second: NameArchivePage = c
        .app
        .http
        .get(url)
        .bearer_auth(&c.jo.1)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let names: Vec<_> = second
        .entries
        .iter()
        .map(|e| e.name.clone().unwrap())
        .collect();
    assert_eq!(names, ["n4", "n3", "n2", "n1", "n0"]);
    assert!(second.next_cursor.is_none());
}

// ---------------------------------------------------------------------------------------------
// Private nicknames

#[tokio::test]
async fn a_private_nickname_reaches_only_its_owner() {
    let c = crew().await;
    let (mut alex_ws, _) = hello(&c.app, &c.alex.1).await;
    let (mut sam_ws, _) = hello(&c.app, &c.sam.1).await;
    let (mut kim_ws, _) = hello(&c.app, &c.kim.1).await;

    assert_eq!(
        private_nick(&c.app, &c.alex.1, c.sam.0, Some(" Sammy "))
            .await
            .status(),
        204
    );
    let got = events_until(&mut alex_ws, |e| {
        matches!(e, Event::PrivateNicknameChanged { .. })
    })
    .await;
    let Event::PrivateNicknameChanged {
        user_id,
        target_id,
        nickname,
    } = &got.last().unwrap().0
    else {
        unreachable!()
    };
    assert_eq!((*user_id, *target_id), (c.alex.0, c.sam.0));
    assert_eq!(nickname.as_deref(), Some("Sammy"));

    // A public event after it, as a marker: nothing before it on the others' sockets names it.
    assert_eq!(
        rename(&c.app, &c.jo.1, c.server, c.jo.0, Some("marker"))
            .await
            .status(),
        200
    );
    for ws in [&mut sam_ws, &mut kim_ws] {
        let got = events_until(ws, |e| matches!(e, Event::MemberUpdated { .. })).await;
        for (e, text) in &got {
            assert!(!matches!(e, Event::PrivateNicknameChanged { .. }), "{e:?}");
            assert!(!text.contains("Sammy"), "{text}");
        }
    }

    // Owner's Ready carries it; nobody else's does, in any field.
    let (_, ready) = hello(&c.app, &c.alex.1).await;
    assert_eq!(ready.private_nicknames.len(), 1);
    assert_eq!(ready.private_nicknames[0].user_id, c.sam.0);
    assert_eq!(ready.private_nicknames[0].nickname, "Sammy");
    for t in [&c.sam.1, &c.kim.1] {
        let (_, ready) = hello(&c.app, t).await;
        assert!(ready.private_nicknames.is_empty());
        assert!(!serde_json::to_string(&ready).unwrap().contains("Sammy"));
    }
    // Nor any REST payload sam can read about themself.
    let paths = [
        format!("/users/{}/profile", c.sam.0),
        format!("/servers/{}/members", c.server),
        format!("/servers/{}/members/{}/names", c.server, c.sam.0),
        "/me".to_string(),
    ];
    for p in &paths {
        let body = c
            .app
            .http
            .get(c.app.url(p))
            .bearer_auth(&c.sam.1)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(!body.contains("Sammy"), "{p}: {body}");
    }
}

#[tokio::test]
async fn private_nicknames_clear_and_follow_the_name_rules() {
    let c = crew().await;
    assert_eq!(
        private_nick(&c.app, &c.alex.1, c.sam.0, Some("Sammy"))
            .await
            .status(),
        204
    );
    assert_eq!(
        private_nick(&c.app, &c.alex.1, c.sam.0, None)
            .await
            .status(),
        204
    );
    let (_, ready) = hello(&c.app, &c.alex.1).await;
    assert!(ready.private_nicknames.is_empty());

    assert_eq!(
        private_nick(&c.app, &c.alex.1, c.alex.0, Some("me"))
            .await
            .status(),
        400
    );
    assert_eq!(
        private_nick(&c.app, &c.alex.1, UserId::new(), Some("x"))
            .await
            .status(),
        404
    );
    let r = private_nick(&c.app, &c.alex.1, c.sam.0, Some(&"z".repeat(33))).await;
    assert_eq!(r.status(), 400);
    // Blank clears, like None.
    assert_eq!(
        private_nick(&c.app, &c.alex.1, c.sam.0, Some("   "))
            .await
            .status(),
        204
    );
}

// ---------------------------------------------------------------------------------------------
// Names the server writes

#[tokio::test]
async fn group_system_lines_use_display_names() {
    let c = crew().await;
    let r = put_profile(&c.app, &c.alex.1, &profile_request(Some("Alex A"))).await;
    assert_eq!(r.status(), 200);
    let r = put_profile(&c.app, &c.sam.1, &profile_request(Some("Sam S"))).await;
    assert_eq!(r.status(), 200);
    let group = create_dm(&c.app, &c.alex.1, &[c.sam.0, c.kim.0]).await;
    let msgs: Vec<Message> = get_json(
        &c.app,
        &c.alex.1,
        &format!("/channels/{}/messages", group.id),
    )
    .await;
    let line = &msgs.last().unwrap().content;
    assert!(line.starts_with("Alex A created the group with"), "{line}");
    assert!(line.contains("Sam S"), "{line}");
}

#[tokio::test]
async fn quote_search_finds_display_names_and_nicknames() {
    let c = crew().await;
    let r = post_json(
        &c.app,
        &c.alex.1,
        &format!("/servers/{}/channels", c.server),
        json!({ "kind": "quotes", "name": "quotes" }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let ch: pulse_protocol::rest::Channel = r.json().await.unwrap();
    let r = post_json(
        &c.app,
        &c.alex.1,
        &format!("/channels/{}/quotes", ch.id),
        json!({ "lines": [{ "speaker_id": c.sam.0, "text": "hello there" }] }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let r = put_profile(&c.app, &c.sam.1, &profile_request(Some("Samwise"))).await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        rename(&c.app, &c.kim.1, c.server, c.sam.0, Some("Gardener"))
            .await
            .status(),
        200
    );
    for q in ["samwise", "gardener", "sam"] {
        let found: Vec<Message> = get_json(
            &c.app,
            &c.alex.1,
            &format!("/servers/{}/quotes/search?q={q}", c.server),
        )
        .await;
        assert_eq!(found.len(), 1, "{q}");
    }
}
