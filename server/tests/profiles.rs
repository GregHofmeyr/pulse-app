//! Profiles: migration 0010 and the name style carried inside `User`.
mod common;

use std::borrow::Cow;
use std::time::Duration;

use common::*;
use futures::SinkExt;
use image::{ImageFormat, Rgba, RgbaImage};
use pulse_protocol::gateway::{ClientFrame, Event, Ready, ServerFrame};
use pulse_protocol::rest::{
    ApiError, Badge, MediaInfo, NameStyle, Profile, UpdateProfileRequest, User,
};
use pulse_server::limits::{LimitsConfig, Rate};
use pulse_server::media;
use sqlx::sqlite::SqliteConnectOptions;
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

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

async fn ready_of(app: &TestApp, token: &str) -> Ready {
    hello(app, token).await.1
}

fn styled(u: &User, id: pulse_protocol::ids::UserId) {
    assert_eq!(u.id, id);
    assert_eq!(
        u.style,
        NameStyle {
            font: Some("pacifico".into()),
            colors: vec!["#ffc4a3".into(), "#ff7fae".into()],
        }
    );
    assert_eq!(u.avatar_hash.as_deref(), Some("avatar-full"));
    assert_eq!(u.avatar_still.as_deref(), Some("avatar-still"));
}

#[tokio::test]
async fn ready_carries_name_styles() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    let r = post_json(
        &app,
        &sam,
        &format!("/servers/{}/join", s.id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(r.status(), 204);

    for q in [
        "INSERT INTO media (hash, kind, content_type, size, width, height, frames, still_hash, created_at) \
         VALUES ('avatar-still', 'avatar', 'image/webp', 1, 256, 256, 1, NULL, '2026-10-01T00:00:00.000Z')",
        "INSERT INTO media (hash, kind, content_type, size, width, height, frames, still_hash, created_at) \
         VALUES ('avatar-full', 'avatar', 'image/webp', 1, 256, 256, 4, 'avatar-still', '2026-10-01T00:00:00.000Z')",
    ] {
        sqlx::query(q).execute(&app.db).await.unwrap();
    }
    sqlx::query(
        "UPDATE users SET name_font = 'pacifico', name_color = '#ffc4a3,#ff7fae', avatar_hash = 'avatar-full' WHERE id = ?",
    )
    .bind(alex_id.to_string())
    .execute(&app.db)
    .await
    .unwrap();

    let ready = ready_of(&app, &sam).await;
    styled(
        &ready
            .people
            .iter()
            .find(|p| p.user.id == alex_id)
            .unwrap()
            .user,
        alex_id,
    );
    let m = ready
        .members
        .iter()
        .flat_map(|sm| sm.members.iter())
        .find(|m| m.user.id == alex_id)
        .unwrap();
    styled(&m.user, alex_id);

    let me: User = get_json(&app, &alex, "/me").await;
    styled(&me, alex_id);
}

#[tokio::test]
async fn migration_0010_adds_defaults_and_drops_images() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(false);
    let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();

    // A database as the previous release left it: migrations up to 0009 only.
    let mut migrator = sqlx::migrate::Migrator::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations"),
    )
    .await
    .unwrap();
    assert!(
        migrator.migrations.iter().any(|m| m.version == 10),
        "0010 exists"
    );
    migrator.migrations = Cow::Owned(
        migrator
            .migrations
            .iter()
            .filter(|m| m.version <= 9)
            .cloned()
            .collect(),
    );
    migrator.run(&pool).await.unwrap();

    let ts = "2026-10-01T00:00:00.000Z";
    let user = "01J0000000000000000000USER";
    sqlx::query(&format!(
        "INSERT INTO users (id, username, password_hash, created_at) VALUES ('{user}', 'alex', 'x', '{ts}')"
    ))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(&format!(
        "INSERT INTO images (hash, bytes, created_at) VALUES ('h', x'00', '{ts}')"
    ))
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    // Upgrade the way the server does at startup.
    let pool = pulse_server::db::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    type Row = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let row: Row = sqlx::query_as(
        "SELECT theme_top, theme_bottom, banner_hash, name_font, name_color, pronouns, bio FROM users WHERE id = ?",
    )
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row,
        (
            "#3a3d46".into(),
            "#16171b".into(),
            None,
            None,
            None,
            None,
            None
        )
    );
    let images: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = 'images'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(images, 0);
    let broken: Vec<(String, i64, String, i64)> = sqlx::query_as("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(broken.is_empty(), "{broken:?}");
}

fn ago(secs: i64) -> String {
    pulse_server::db::stamp(chrono::Utc::now() - chrono::Duration::seconds(secs))
}

async fn profile_of(app: &TestApp, token: &str, id: pulse_protocol::ids::UserId) -> Profile {
    get_json(app, token, &format!("/users/{id}/profile")).await
}

async fn exec(app: &TestApp, q: &str, binds: &[&str]) {
    let mut query = sqlx::query(q);
    for b in binds {
        query = query.bind(*b);
    }
    query.execute(&app.db).await.unwrap();
}

#[tokio::test]
async fn stats_total_across_servers() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let sa = create_server(&app, &alex, "A").await;
    let sb = create_server(&app, &alex, "B").await;
    let ca = general(&app, &alex, sa.id).await;
    let cb = general(&app, &alex, sb.id).await;
    let u = alex_id.to_string();
    let (a, b) = (ca.id.to_string(), cb.id.to_string());
    let q = "INSERT INTO voice_sessions (id, user_id, channel_id, joined_at, left_at) VALUES (?, ?, ?, ?, ?)";
    exec(&app, q, &["s1", &u, &a, &ago(7200), &ago(3600)]).await;
    exec(&app, q, &["s2", &u, &b, &ago(5400), &ago(3600)]).await;
    exec(
        &app,
        "INSERT INTO voice_sessions (id, user_id, channel_id, joined_at) VALUES ('s3', ?, ?, ?)",
        &[&u, &a, &ago(60)],
    )
    .await;

    let p = profile_of(&app, &alex, alex_id).await;
    assert!(
        (5460..=5470).contains(&p.stats.voice_seconds),
        "{}",
        p.stats.voice_seconds
    );
    let created: String = sqlx::query_scalar("SELECT created_at FROM users WHERE id = ?")
        .bind(&u)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(p.stats.member_since, created);
    assert_eq!(p.user.username, "alex");
}

#[tokio::test]
async fn top_reaction_counts_live_messages_only() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let s = create_server(&app, &alex, "Main").await;
    let c = general(&app, &alex, s.id).await;
    let u = alex_id.to_string();
    let mut live = Vec::new();
    for i in 0..5 {
        live.push(
            send(&app, &alex, c.id, &format!("m{i}"))
                .await
                .id
                .to_string(),
        );
    }
    let gone = send(&app, &alex, c.id, "gone").await.id.to_string();
    exec(
        &app,
        "UPDATE messages SET deleted_at = ? WHERE id = ?",
        &[&ago(1), &gone],
    )
    .await;
    let q = "INSERT INTO reactions (message_id, user_id, emoji, created_at) VALUES (?, ?, ?, ?)";
    for m in &live[..3] {
        exec(&app, q, &[m, &u, "🔥", &ago(100)]).await;
    }
    for m in &live[3..] {
        exec(&app, q, &[m, &u, "😂", &ago(50)]).await;
    }
    // Three more 😂 on a deleted message must not count.
    exec(&app, q, &[&gone, &u, "😂", &ago(10)]).await;
    exec(&app, q, &[&gone, &u, "🎉", &ago(10)]).await;
    assert_eq!(
        profile_of(&app, &alex, alex_id)
            .await
            .stats
            .top_reaction
            .as_deref(),
        Some("🔥")
    );

    // A tie goes to the most recent: 😂 now has 3 live too, and the newest reaction.
    exec(&app, q, &[&live[0], &u, "😂", &ago(5)]).await;
    assert_eq!(
        profile_of(&app, &alex, alex_id)
            .await
            .stats
            .top_reaction
            .as_deref(),
        Some("😂")
    );
}

#[tokio::test]
async fn name_archive_and_renames() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, _) = register(&app, "sam").await;
    let sa = create_server(&app, &alex, "A").await;
    let sb = create_server(&app, &alex, "B").await;

    let none = profile_of(&app, &alex, alex_id).await;
    assert_eq!(none.stats.times_renamed, 0);
    assert!(none.name_archive.is_empty());

    let q = "INSERT INTO nickname_history (id, server_id, target_user_id, changed_by, old_nick, new_nick, changed_at) VALUES (?, ?, ?, ?, NULL, ?, ?)";
    let (u, by) = (alex_id.to_string(), sam_id.to_string());
    let t_old = ago(200);
    let t_new = ago(100);
    exec(&app, q, &["h1", &sa.id.to_string(), &u, &u, "Old", &t_old]).await;
    exec(&app, q, &["h2", &sb.id.to_string(), &u, &by, "New", &t_new]).await;
    // A cleared nickname (new_nick NULL) is a rename, but has no name to archive.
    let cleared = "INSERT INTO nickname_history (id, server_id, target_user_id, changed_by, old_nick, new_nick, changed_at) VALUES (?, ?, ?, ?, 'Old', NULL, ?)";
    exec(
        &app,
        cleared,
        &["h3", &sa.id.to_string(), &u, &u, &ago(150)],
    )
    .await;

    let p = profile_of(&app, &alex, alex_id).await;
    assert_eq!(p.stats.times_renamed, 3);
    assert_eq!(p.name_archive.len(), 2);
    assert_eq!(p.name_archive[0].name, "New");
    assert_eq!(p.name_archive[0].server_id, sb.id);
    assert_eq!(p.name_archive[0].changed_by, sam_id);
    assert_eq!(p.name_archive[0].changed_at, t_new);
    assert_eq!(p.name_archive[1].name, "Old");
    assert_eq!(p.name_archive[1].server_id, sa.id);
    assert_eq!(p.name_archive[1].changed_by, alex_id);
}

#[tokio::test]
async fn badges_come_from_user_badges() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    assert!(profile_of(&app, &alex, alex_id).await.badges.is_empty());
    let t = ago(30);
    exec(
        &app,
        "INSERT INTO user_badges (user_id, badge_key, earned_at, first_earner) VALUES (?, 'founder', ?, 1)",
        &[&alex_id.to_string(), &t],
    )
    .await;
    assert_eq!(
        profile_of(&app, &alex, alex_id).await.badges,
        vec![Badge {
            key: "founder".into(),
            earned_at: t,
            first_earner: true
        }]
    );
}

#[tokio::test]
async fn card_carries_theme_and_banner_still() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    exec(&app,
        "INSERT INTO media (hash, kind, content_type, size, width, height, frames, still_hash, created_at) VALUES ('b-still', 'banner', 'image/webp', 1, 960, 384, 1, NULL, '2026-10-01T00:00:00.000Z')", &[]).await;
    exec(&app,
        "INSERT INTO media (hash, kind, content_type, size, width, height, frames, still_hash, created_at) VALUES ('b-full', 'banner', 'image/webp', 1, 600, 240, 4, 'b-still', '2026-10-01T00:00:00.000Z')", &[]).await;
    exec(
        &app,
        "UPDATE users SET banner_hash = 'b-full', pronouns = 'they/them', bio = 'hi' WHERE id = ?",
        &[&alex_id.to_string()],
    )
    .await;
    let c = profile_of(&app, &alex, alex_id).await.card;
    assert_eq!(c.banner_hash.as_deref(), Some("b-full"));
    assert_eq!(c.banner_still.as_deref(), Some("b-still"));
    assert_eq!(
        (c.theme_top.as_str(), c.theme_bottom.as_str()),
        ("#3a3d46", "#16171b")
    );
    assert_eq!(c.pronouns.as_deref(), Some("they/them"));
    assert_eq!(c.bio.as_deref(), Some("hi"));
}

#[tokio::test]
async fn unknown_user_is_404() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    for id in [
        pulse_protocol::ids::UserId::new().to_string(),
        "not-an-id".into(),
    ] {
        let r = app
            .http
            .get(app.url(&format!("/users/{id}/profile")))
            .bearer_auth(&alex)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "{id}");
    }
    let r = app
        .http
        .get(app.url("/users/x/profile"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

// ---- PUT /me/profile ----

fn png() -> Vec<u8> {
    let img = RgbaImage::from_pixel(64, 32, Rgba([10, 120, 240, 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

async fn upload_kind(app: &TestApp, token: &str, kind: &str) -> String {
    let r = app
        .http
        .post(app.url(&format!("/media/{kind}")))
        .bearer_auth(token)
        .header("content-type", "application/octet-stream")
        .body(png())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json::<MediaInfo>().await.unwrap().hash
}

fn request() -> UpdateProfileRequest {
    UpdateProfileRequest {
        avatar_hash: None,
        banner_hash: None,
        theme_top: "#8a3cff".into(),
        theme_bottom: "#ff8a2a".into(),
        style: NameStyle::default(),
        pronouns: None,
        bio: None,
    }
}

async fn put_profile(app: &TestApp, token: &str, body: &UpdateProfileRequest) -> reqwest::Response {
    app.http
        .put(app.url("/me/profile"))
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .unwrap()
}

async fn refused(app: &TestApp, token: &str, body: &UpdateProfileRequest, msg: &str) {
    let r = put_profile(app, token, body).await;
    assert_eq!(r.status(), 400, "{msg}");
    assert_eq!(r.json::<ApiError>().await.unwrap().message, msg);
}

async fn connected(app: &TestApp, token: &str) -> Socket {
    hello(app, token).await.0
}

async fn profile_event(ws: &mut Socket) -> Event {
    loop {
        match next_frame_beating(ws, Duration::from_secs(3)).await {
            Some(ServerFrame::Event(e @ Event::ProfileUpdated { .. })) => return e,
            Some(_) => continue,
            None => panic!("expected ProfileUpdated"),
        }
    }
}

#[tokio::test]
async fn saves_a_whole_profile_and_tells_everyone() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let mut kim_ws = connected(&app, &kim).await;

    let req = UpdateProfileRequest {
        avatar_hash: Some(upload_kind(&app, &alex, "avatar").await),
        banner_hash: Some(upload_kind(&app, &alex, "banner").await),
        theme_top: "#8a3cff".into(),
        theme_bottom: "#ff8a2a".into(),
        style: NameStyle {
            font: Some("pacifico".into()),
            colors: vec!["#ffbe7a".into(), "#ff7fae".into()],
        },
        pronouns: Some("they/them".into()),
        bio: Some("one\ntwo\nthree".into()),
    };
    let r = put_profile(&app, &alex, &req).await;
    assert_eq!(r.status(), 200);
    let p: Profile = r.json().await.unwrap();
    assert_eq!(p.user.id, alex_id);
    assert_eq!(p.user.style, req.style);
    assert_eq!(p.user.avatar_hash, req.avatar_hash);
    assert_eq!(p.card.banner_hash, req.banner_hash);
    assert_eq!(p.card.pronouns.as_deref(), Some("they/them"));
    assert_eq!(p.card.bio.as_deref(), Some("one\ntwo\nthree"));

    match profile_event(&mut kim_ws).await {
        Event::ProfileUpdated { user, card } => {
            assert_eq!(user, p.user);
            assert_eq!(card, p.card);
        }
        _ => unreachable!(),
    }
    let ready = ready_of(&app, &sam).await;
    let seen = &ready
        .people
        .iter()
        .find(|p| p.user.id == alex_id)
        .unwrap()
        .user;
    assert_eq!(seen.style, req.style);
}

#[tokio::test]
async fn custom_colours_are_lightened() {
    let app = spawn().await;
    let (id, alex) = register(&app, "alex").await;
    let mut req = request();
    req.style.colors = vec!["#0000FF".into()];
    let p: Profile = put_profile(&app, &alex, &req).await.json().await.unwrap();
    assert_eq!(p.user.style.colors, vec!["#a1a1ff".to_string()]);
    assert_eq!(
        profile_of(&app, &alex, id).await.user.style.colors,
        vec!["#a1a1ff".to_string()]
    );
}

#[tokio::test]
async fn bad_input_is_refused() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let colours = "Colours must look like #a1b2c3.";

    let mut r = request();
    r.style.font = Some("comic-sans".into());
    refused(&app, &alex, &r, "Unknown name font.").await;

    let mut r = request();
    r.style.colors = vec!["blue".into()];
    refused(&app, &alex, &r, colours).await;

    let mut r = request();
    r.style.colors = vec!["#aaaaaa".into(), "#bbbbbb".into(), "#cccccc".into()];
    refused(&app, &alex, &r, colours).await;

    let mut r = request();
    r.style.colors = vec!["#0000ff".into(), "blue".into()];
    refused(&app, &alex, &r, colours).await;

    let mut r = request();
    r.theme_top = "#12345".into();
    refused(&app, &alex, &r, colours).await;

    let mut r = request();
    r.theme_bottom = "red".into();
    refused(&app, &alex, &r, colours).await;

    let mut r = request();
    r.pronouns = Some("p".repeat(41));
    refused(&app, &alex, &r, "Pronouns can be up to 40 characters.").await;

    let mut r = request();
    r.pronouns = Some("a\u{0007}b".into());
    refused(
        &app,
        &alex,
        &r,
        "Pronouns can't contain control characters.",
    )
    .await;

    let mut r = request();
    r.bio = Some("b".repeat(191));
    refused(&app, &alex, &r, "Your bio can be up to 190 characters.").await;

    let mut r = request();
    r.bio = Some("a\nb\nc\nd\ne\nf".into());
    refused(&app, &alex, &r, "Your bio can have at most 4 line breaks.").await;

    let mut r = request();
    r.bio = Some("a\u{0007}b".into());
    refused(
        &app,
        &alex,
        &r,
        "Your bio can't contain control characters.",
    )
    .await;

    let expired = "That picture has expired. Upload it again.";
    exec(&app,
        "INSERT INTO media (hash, kind, content_type, size, width, height, frames, still_hash, created_at) VALUES ('an-emoji', 'emoji', 'image/webp', 1, 128, 128, 1, NULL, '2026-10-01T00:00:00.000Z')", &[]).await;
    let mut r = request();
    r.avatar_hash = Some("an-emoji".into());
    refused(&app, &alex, &r, expired).await;

    let av = upload_kind(&app, &alex, "avatar").await;
    let mut r = request();
    r.banner_hash = Some(av);
    refused(&app, &alex, &r, expired).await;
}

#[tokio::test]
async fn limits_count_characters_not_bytes() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let mut r = request();
    r.pronouns = Some("\u{1F600}".repeat(40));
    r.bio = Some("\u{e9}".repeat(190));
    assert_eq!(put_profile(&app, &alex, &r).await.status(), 200);
}

#[tokio::test]
async fn clearing_works() {
    let app = spawn().await;
    let (id, alex) = register(&app, "alex").await;
    let mut r = request();
    r.avatar_hash = Some(upload_kind(&app, &alex, "avatar").await);
    r.pronouns = Some("she/her".into());
    r.bio = Some("hello".into());
    assert_eq!(put_profile(&app, &alex, &r).await.status(), 200);

    let mut r = request();
    r.pronouns = Some(String::new());
    r.bio = Some("  ".into());
    let p: Profile = put_profile(&app, &alex, &r).await.json().await.unwrap();
    assert_eq!(p.user.avatar_hash, None);
    assert_eq!((p.card.pronouns, p.card.bio), (None, None));
    let row: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as("SELECT avatar_hash, pronouns, bio, name_color FROM users WHERE id = ?")
        .bind(id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(row, (None, None, None, None));
}

#[tokio::test]
async fn a_swept_upload_is_refused_on_save() {
    let app = spawn().await;
    let (id, alex) = register(&app, "alex").await;
    let hash = upload_kind(&app, &alex, "avatar").await;
    exec(
        &app,
        "UPDATE media SET created_at = '2020-01-01T00:00:00.000Z'",
        &[],
    )
    .await;
    media::cleanup(&app.db, &app.cfg.media_dir, Duration::from_secs(24 * 3600))
        .await
        .unwrap();
    let mut r = request();
    r.avatar_hash = Some(hash);
    r.pronouns = Some("changed".into());
    refused(
        &app,
        &alex,
        &r,
        "That picture has expired. Upload it again.",
    )
    .await;
    assert_eq!(profile_of(&app, &alex, id).await.card.pronouns, None);
}

#[tokio::test]
async fn the_current_avatar_is_always_accepted() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let hash = upload_kind(&app, &alex, "avatar").await;
    let mut r = request();
    r.avatar_hash = Some(hash);
    assert_eq!(put_profile(&app, &alex, &r).await.status(), 200);
    exec(
        &app,
        "UPDATE media SET created_at = '2020-01-01T00:00:00.000Z'",
        &[],
    )
    .await;
    media::cleanup(&app.db, &app.cfg.media_dir, Duration::from_secs(24 * 3600))
        .await
        .unwrap();
    assert_eq!(put_profile(&app, &alex, &r).await.status(), 200);
    assert_eq!(put_profile(&app, &alex, &r).await.status(), 200);
}

#[tokio::test]
async fn saving_is_rate_limited() {
    let mut cfg = pulse_server::testing::test_config();
    cfg.limits = LimitsConfig::unlimited();
    cfg.limits.mutate_user = Rate::new(2, 0.0);
    let app = spawn_with(cfg).await;
    let (_, alex) = register(&app, "alex").await;
    assert_eq!(put_profile(&app, &alex, &request()).await.status(), 200);
    assert_eq!(put_profile(&app, &alex, &request()).await.status(), 200);
    assert_eq!(put_profile(&app, &alex, &request()).await.status(), 429);
}

#[tokio::test]
async fn bio_line_endings_are_normalised_and_blank_lines_trimmed_at_both_ends() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let mut r = request();
    r.bio = Some("a\r\nb\rc".into());
    let p: Profile = put_profile(&app, &alex, &r).await.json().await.unwrap();
    assert_eq!(p.card.bio.as_deref(), Some("a\nb\nc"));

    let mut r = request();
    r.bio = Some("a\r\nb\r\nc\r\nd\r\ne\r\nf".into());
    refused(&app, &alex, &r, "Your bio can have at most 4 line breaks.").await;

    let mut r = request();
    r.bio = Some("\n  indented  \n\n".into());
    let p: Profile = put_profile(&app, &alex, &r).await.json().await.unwrap();
    assert_eq!(p.card.bio.as_deref(), Some("  indented"));

    // Blank leading lines are dropped, so they don't count towards the 4 line breaks.
    let mut r = request();
    r.bio = Some("\n \n\r\n\n\n  a\nb\nc\nd\ne".into());
    let p: Profile = put_profile(&app, &alex, &r).await.json().await.unwrap();
    assert_eq!(p.card.bio.as_deref(), Some("  a\nb\nc\nd\ne"));
}
