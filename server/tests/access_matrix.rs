//! The access matrix (#80): what someone signed in but not in a server can read from it.
//!
//! [`MATRIX`] lists every route that reads a server's insides (or hands out a way in, like a voice token), and what an
//! outsider gets from it: `Denied` (403/404, or a success that carries none of it) for every row since the lockdown
//! (#80, step 2). The test fails when any answer differs from the table, so no leak can quietly come back.
//! `ExpectedLeak` stays as what the test calls an answer that shows the server's insides.
//!
//! [`every_route_is_classified`] keeps the table whole: it reads every route the server registers (each
//! `.route("…")` under `src/`, with the methods the router answers for it) and fails for one that is in neither the
//! matrix nor [`PUBLIC`] (reads that never show a server's insides: your own things, or nothing of anyone's) nor
//! [`WRITES`] (changes, which check membership as they act). A new route can't skip the question.
mod common;

use std::collections::{BTreeSet, HashMap};

use common::*;
use pulse_protocol::ids::{ChannelId, MessageId};
use pulse_protocol::rest::{AttachmentUpload, Channel, Message};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    /// An outsider reads it. No row expects this since the lockdown (#80); an answer that shows the insides is one.
    ExpectedLeak,
    /// Refused (403 or 404), or a success carrying nothing of the server's.
    Denied,
}
use Expect::*;

/// One read: its route as registered, the URL to call (placeholders filled from the [`World`]), the body for a POST or
/// PUT, and what in the answer shows it leaked (none: any success does). A member (or the owner, for the owner's own
/// routes) makes the same call first and must succeed (with the marker), so a `Denied` means the outsider was refused,
/// not that the fixture is missing something; `no_control` names why that can't be shown for a row.
struct Read {
    route: &'static str,
    url: &'static str,
    body: Option<&'static str>,
    marker: Option<&'static str>,
    expect: Expect,
    no_control: Option<&'static str>,
    owner_only: bool,
}

impl Read {
    const fn no_control(mut self, why: &'static str) -> Self {
        self.no_control = Some(why);
        self
    }
    const fn owner_only(mut self) -> Self {
        self.owner_only = true;
        self
    }
}

const fn get(route: &'static str, url: &'static str, expect: Expect) -> Read {
    Read {
        route,
        url,
        body: None,
        marker: None,
        expect,
        no_control: None,
        owner_only: false,
    }
}

const fn get_marked(
    route: &'static str,
    url: &'static str,
    marker: &'static str,
    expect: Expect,
) -> Read {
    Read {
        route,
        url,
        body: None,
        marker: Some(marker),
        expect,
        no_control: None,
        owner_only: false,
    }
}

const fn write(route: &'static str, url: &'static str, body: &'static str, expect: Expect) -> Read {
    Read {
        route,
        url,
        body: Some(body),
        marker: None,
        expect,
        no_control: None,
        owner_only: false,
    }
}

/// Every read of a server's insides, and what an outsider gets: since the lockdown (#80, step 2), nothing.
const MATRIX: &[Read] = &[
    // the server itself
    get_marked("GET /servers", "/servers", "{server}", Denied),
    get(
        "GET /servers/{id}/channels",
        "/servers/{server}/channels",
        Denied,
    ),
    get(
        "GET /servers/{id}/members",
        "/servers/{server}/members",
        Denied,
    ),
    get(
        "GET /servers/{id}/members/{user}/names",
        "/servers/{server}/members/{sam}/names",
        Denied,
    ),
    // a profile's name archive: only the names given in servers the reader is in (anyone may read the rest)
    get_marked(
        "GET /users/{id}/profile",
        "/users/{sam}/profile",
        "{server}",
        Denied,
    ),
    get(
        "GET /servers/{id}/members/{user}/deadlock",
        "/servers/{server}/members/{sam}/deadlock",
        Denied,
    ),
    get(
        "GET /servers/{id}/sounds",
        "/servers/{server}/sounds",
        Denied,
    ),
    get(
        "GET /servers/{id}/wheels",
        "/servers/{server}/wheels",
        Denied,
    ),
    get("GET /servers/{id}/f1", "/servers/{server}/f1", Denied),
    get(
        "GET /servers/{id}/achievements-channel",
        "/servers/{server}/achievements-channel",
        Denied,
    ),
    get(
        "GET /servers/{id}/deadlock-channel",
        "/servers/{server}/deadlock-channel",
        Denied,
    ),
    get(
        "GET /servers/{id}/deadlock-ranks",
        "/servers/{server}/deadlock-ranks",
        Denied,
    ),
    get(
        "GET /servers/{id}/sports-channel",
        "/servers/{server}/sports-channel",
        Denied,
    ),
    get(
        "GET /servers/{id}/steam-hours",
        "/servers/{server}/steam-hours",
        Denied,
    ),
    get(
        "GET /servers/{id}/clips/storage",
        "/servers/{server}/clips/storage",
        Denied,
    ),
    get(
        "GET /servers/{id}/quotes/leaderboard",
        "/servers/{server}/quotes/leaderboard",
        Denied,
    ),
    get_marked(
        "GET /servers/{id}/quotes/search",
        "/servers/{server}/quotes/search?q=wisdom",
        "{quote}",
        Denied,
    ),
    get_marked(
        "GET /servers/{id}/search",
        "/servers/{server}/search?q=hello",
        "{msg}",
        Denied,
    ),
    get(
        "GET /servers/{id}/deleted-channels",
        "/servers/{server}/deleted-channels",
        Denied,
    )
    .owner_only(),
    get(
        "GET /servers/{id}/invites",
        "/servers/{server}/invites",
        Denied,
    ),
    get("GET /servers/{id}/bans", "/servers/{server}/bans", Denied).owner_only(),
    // its channels
    get_marked(
        "GET /channels/{id}/messages",
        "/channels/{text}/messages",
        "{msg}",
        Denied,
    ),
    get_marked(
        "GET /channels/{id}/pins",
        "/channels/{text}/pins",
        "{msg}",
        Denied,
    ),
    get(
        "GET /channels/{id}/clips",
        "/channels/{clips}/clips",
        Denied,
    ),
    get(
        "GET /channels/{id}/clips/facets",
        "/channels/{clips}/clips/facets",
        Denied,
    ),
    get_marked("GET /search", "/search?q=hello", "{msg}", Denied),
    // its messages
    get("GET /messages/{id}/edits", "/messages/{msg}/edits", Denied),
    get(
        "GET /messages/{id}/reactions/{emoji}",
        "/messages/{msg}/reactions/%F0%9F%91%8D",
        Denied,
    ),
    get(
        "GET /messages/{id}/replies",
        "/messages/{quote}/replies",
        Denied,
    ),
    get("GET /messages/{id}/poll", "/messages/{poll}/poll", Denied),
    get("GET /messages/{id}/plan", "/messages/{plan}/plan", Denied),
    get(
        "GET /messages/{id}/plan/forecast",
        "/messages/{plan}/plan/forecast",
        Denied,
    ),
    get(
        "GET /messages/{id}/plan/ics",
        "/messages/{plan}/plan/ics",
        Denied,
    ),
    get(
        "GET /messages/{id}/attachments/urls",
        "/messages/{files}/attachments/urls",
        Denied,
    ),
    get(
        "GET /messages/{id}/view-once",
        "/messages/{once}/view-once",
        Denied,
    ),
    get("GET /clips/{message_id}/url", "/clips/{clip}/url", Denied),
    // ways in to its calls and streams
    write(
        "POST /voice/{id}/token",
        "/voice/{voice}/token",
        "{}",
        Denied,
    ),
    write(
        "POST /streams/{channel}/ticket",
        "/streams/{voice}/ticket",
        r#"{"role": "v", "ghost": true}"#,
        Denied,
    )
    .no_control("refused before the stream is looked up; a live stream needs LiveKit"),
    write(
        "POST /streams/{channel}/watch",
        "/streams/{voice}/watch",
        r#"{"streamer": "{alex}", "ghost": true}"#,
        Denied,
    )
    .no_control("refused before the stream is looked up; a live stream needs LiveKit"),
    get(
        "GET /streams/{channel}/{user}/preview",
        "/streams/{voice}/{alex}/preview",
        Denied,
    )
    .no_control("refused before the stream is looked up; a live stream needs LiveKit"),
    // copying its messages somewhere the outsider keeps
    write(
        "POST /messages/{id}/forward",
        "/messages/{msg}/forward",
        r#"{"channel_id": "{my_notes}"}"#,
        Denied,
    ),
    write(
        "POST /messages/{id}/bookmark",
        "/messages/{msg}/bookmark",
        "{}",
        Denied,
    ),
    write(
        "POST /messages/{id}/reminders",
        "/messages/{msg}/reminders",
        r#"{"remind_at": "{tomorrow}"}"#,
        Denied,
    ),
    write(
        "POST /messages/{id}/view-once/open",
        "/messages/{once}/view-once/open",
        "{}",
        Denied,
    ),
];

/// Reads that never show a server's insides: no server at all, the caller's own things, or public by design.
const PUBLIC: &[&str] = &[
    "GET /health",
    "GET /gateway",
    "GET /me",
    "GET /auth/2fa",
    "GET /dms",
    "GET /badges",
    "GET /bookmarks",
    "GET /inbox",
    "GET /inbox/threads",
    "GET /inbox/unread",
    "GET /reminders",
    "GET /scheduled",
    "GET /gifs/config",
    "GET /gifs/favourites",
    "GET /me/sports",
    "GET /me/steam",
    "GET /sports/teams",
    "GET /steam/openid/return",
    "GET /servers/deleted",
    "GET /wheels/mine",
    "GET /attachment-uploads/{id}/parts",
    // DMs and groups only: a server channel is refused (400) and searched with the server's search (in the matrix)
    "GET /channels/{id}/search",
    // uploaded files: anyone with the (unguessable) link, as decided for #80
    "GET /media/{hash}",
    "GET /updates/{file}",
    "GET /models/{name}",
    "GET /embed/{provider}/{id}",
    // invites: a preview shows only what a card may (see `servers::invites`)
    "GET /invites/{code}",
    // the public invite page shows only what a card shows a stranger; the installer and robots.txt are public
    "GET /i/{code}",
    "GET /download/windows",
    "GET /robots.txt",
    // a game invite is in a DM or group: members of it only
    "POST /messages/{id}/game-invite/join",
];

/// Changes. Each checks what it changes as it acts; reads of what they make are in the matrix.
const WRITES: &[&str] = &[
    "DELETE /attachment-uploads/{id}",
    "POST /attachment-uploads/{id}/complete",
    "POST /attachment-uploads/{id}/parts",
    "POST /auth/2fa/disable",
    "POST /auth/2fa/enable",
    "POST /auth/2fa/recovery-codes",
    "POST /auth/2fa/setup",
    "POST /auth/login",
    "POST /auth/login/2fa",
    "POST /auth/logout",
    "POST /auth/logout-others",
    "POST /auth/password",
    "POST /auth/register",
    "DELETE /bookmarks/{id}",
    "PATCH /bookmarks/{id}",
    "POST /calls/{id}",
    "POST /calls/{id}/decline",
    "DELETE /categories/{id}",
    "PATCH /categories/{id}",
    "DELETE /channels/{id}",
    "PATCH /channels/{id}",
    "DELETE /channels/{id}/archive",
    "PUT /channels/{id}/archive",
    "POST /channels/{id}/attachments",
    "PUT /channels/{id}/bitrate",
    "PUT /channels/{id}/clip-view",
    "POST /channels/{id}/clips/uploads",
    "POST /channels/{id}/close",
    "POST /channels/{id}/game-invite",
    "PUT /channels/{id}/icon",
    "POST /channels/{id}/members",
    "DELETE /channels/{id}/members/{user_id}",
    "POST /channels/{id}/messages",
    "DELETE /channels/{id}/permanent",
    "POST /channels/{id}/plans",
    "POST /channels/{id}/polls",
    "POST /channels/{id}/quotes",
    "POST /channels/{id}/read",
    "POST /channels/{id}/read-all",
    "POST /channels/{id}/restore",
    "POST /channels/{id}/scheduled",
    "PUT /channels/{id}/stage-share",
    "DELETE /channels/{id}/stage-share/{share_id}",
    "DELETE /channels/{id}/watch-party",
    "POST /channels/{id}/watch-party",
    "PUT /channels/{id}/watch-party",
    "POST /channels/{id}/wheel-spins",
    "POST /channels/{id}/wheel-spins/quiet",
    "DELETE /clip-uploads/{id}",
    "POST /clip-uploads/{id}/confirm",
    "PUT /clips/{message_id}",
    "POST /dms",
    "DELETE /emoji/{id}",
    "PATCH /emoji/{id}",
    "POST /gifs/favourites",
    "DELETE /gifs/favourites/{id}",
    "POST /inbox/read-all",
    "POST /inbox/{message_id}/read",
    "POST /invites",
    "POST /invites/{code}/accept",
    "DELETE /invites/{code}",
    "PATCH /invites/{code}",
    "POST /livekit/webhook",
    "DELETE /me/activity-hidden/{id}",
    "PUT /me/activity-hidden/{id}",
    "DELETE /me/appear-offline/{id}",
    "PUT /me/appear-offline/{id}",
    "POST /me/badges/egg",
    "POST /me/badges/muted-talk",
    "POST /me/badges/seen",
    "PUT /me/badges/showcase",
    "DELETE /me/channel-orders/{server_id}",
    "PUT /me/channel-orders/{server_id}",
    "DELETE /me/custom-status",
    "PUT /me/custom-status",
    "POST /me/dm-categories",
    "DELETE /me/dm-categories/{id}",
    "PATCH /me/dm-categories/{id}",
    "PUT /me/dm-placements/{channel_id}",
    "PUT /me/inbox-filters",
    "DELETE /me/live-notify/{id}",
    "PUT /me/live-notify/{id}",
    "DELETE /me/person-dnd/{id}",
    "PUT /me/person-dnd/{id}",
    "PUT /me/plan-reminders",
    "PUT /me/private-nicknames/{id}",
    "PUT /me/profile",
    "PUT /me/server-order",
    "PUT /me/sports",
    "POST /me/sports/follows",
    "DELETE /me/sports/follows/{league}/{team}",
    "PUT /me/status",
    "DELETE /me/steam",
    "POST /me/steam/link",
    "PUT /me/steam/settings",
    "POST /media/avatar",
    "POST /media/banner",
    "POST /media/clip-thumb",
    "POST /media/emoji",
    "POST /media/quote",
    "POST /media/sound",
    "DELETE /messages/{id}",
    "PATCH /messages/{id}",
    "POST /messages/{id}/embeds/hide",
    "POST /messages/{id}/embeds/show",
    "PUT /messages/{id}/f1/prediction",
    "DELETE /messages/{id}/pin",
    "PUT /messages/{id}/pin",
    "PUT /messages/{id}/plan",
    "POST /messages/{id}/plan/cancel",
    "PUT /messages/{id}/plan/rsvp",
    "PUT /messages/{id}/poll",
    "POST /messages/{id}/poll/close",
    "PUT /messages/{id}/poll/votes",
    "DELETE /messages/{id}/reactions/{emoji}",
    "PUT /messages/{id}/reactions/{emoji}",
    "POST /messages/{id}/replies/seen",
    "POST /messages/{id}/spin-again",
    "PUT /mutes",
    "DELETE /mutes/{kind}/{id}",
    "PUT /notify-levels",
    "POST /qoty/{id}/end",
    "POST /qoty/{id}/options",
    "POST /qoty/{id}/reopen",
    "DELETE /qoty/{id}/vote",
    "PUT /qoty/{id}/vote",
    "DELETE /quote-lines/{line_id}/award",
    "PUT /quote-lines/{line_id}/award",
    "PUT /quotes/{message_id}",
    "DELETE /reminders/{id}",
    "POST /reminders/{id}/alerted",
    "POST /reminders/{id}/seen",
    "POST /reports",
    "DELETE /scheduled/{id}",
    "PATCH /scheduled/{id}",
    "POST /scheduled/{id}/send",
    "POST /servers",
    "DELETE /servers/{id}",
    "PUT /servers/{id}/achievements-channel",
    "PUT /servers/{id}/banner",
    "DELETE /servers/{id}/bans/{user}",
    "PUT /servers/{id}/bans/{user}",
    "POST /servers/{id}/categories",
    "POST /servers/{id}/channels",
    "PUT /servers/{id}/deadlock-channel",
    "POST /servers/{id}/emoji",
    "PUT /servers/{id}/f1-channel",
    "POST /servers/{id}/games",
    "PUT /servers/{id}/icon",
    "POST /servers/{id}/invites",
    // the old open Join: refuses everyone with the same words (#80)
    "POST /servers/{id}/join",
    "DELETE /servers/{id}/join-sound",
    "PUT /servers/{id}/join-sound",
    "PUT /servers/{id}/layout",
    "POST /servers/{id}/leave",
    "DELETE /servers/{id}/members/{user}",
    "PUT /servers/{id}/members/{user}/nickname",
    "PUT /servers/{id}/name",
    "PUT /servers/{id}/owner",
    "POST /servers/{id}/qoty",
    "POST /servers/{id}/restore",
    "POST /servers/{id}/sounds",
    "PUT /servers/{id}/speaker-links",
    "DELETE /servers/{id}/speaker-links/{name_key}",
    "PUT /servers/{id}/sports-channel",
    "POST /servers/{server}/voice/{user}",
    "POST /servers/{server}/voice/{user}/disconnect",
    "DELETE /sounds/{id}",
    "PATCH /sounds/{id}",
    "DELETE /sounds/{id}/favourite",
    "PUT /sounds/{id}/favourite",
    "POST /sounds/{id}/play",
    "DELETE /streams/{channel}/live",
    "POST /streams/{channel}/live",
    "PUT /streams/{channel}/preview",
    "POST /streams/{channel}/unwatch",
    "POST /wheels",
    "DELETE /wheels/{id}",
    "PATCH /wheels/{id}",
];

/// alex owns "Main", with something in every corner; sam is in it; jo (the outsider) isn't.
struct World {
    app: TestApp,
    alex: String,
    sam: String,
    jo: String,
    /// Each caller's notes to self, by token: `{my_notes}`.
    notes: HashMap<String, String>,
    vars: HashMap<&'static str, String>,
}

async fn ok_json<T: serde::de::DeserializeOwned>(r: reqwest::Response, what: &str) -> T {
    let status = r.status();
    let text = r.text().await.unwrap();
    assert!(status.is_success(), "{what}: {status} {text}");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{what}: {e}: {text}"))
}

async fn put_json(app: &TestApp, token: &str, path: &str, body: Value) -> reqwest::Response {
    app.http
        .put(app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn channel(
    app: &TestApp,
    token: &str,
    server: impl std::fmt::Display,
    kind: &str,
) -> Channel {
    ok_json(
        post_json(
            app,
            token,
            &format!("/servers/{server}/channels"),
            json!({ "kind": kind, "name": kind }),
        )
        .await,
        kind,
    )
    .await
}

/// Upload a small WebP to `channel` as the client does, and return its id.
async fn upload(app: &TestApp, token: &str, channel: ChannelId, view_once: bool) -> String {
    let mut bytes = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
    bytes.resize(64, 7);
    let up: AttachmentUpload = ok_json(
        post_json(
            app,
            token,
            &format!("/channels/{channel}/attachments"),
            json!({ "name": "a.webp", "size": bytes.len(), "content_type": "image/webp", "view_once": view_once }),
        )
        .await,
        "start an upload",
    )
    .await;
    let r = reqwest::Client::new()
        .put(up.url.clone().unwrap())
        .header("content-type", &up.content_type)
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    up.id.to_string()
}

async fn world() -> World {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_, jo) = register(&app, "jo").await;
    let server = create_server(&app, &alex, "Main").await.id;
    let r = join_server(&app, &sam, server).await;
    assert_eq!(r.status(), 200);
    let text = general(&app, &alex, server).await;
    let voice: Vec<Channel> = get_json(&app, &alex, &format!("/servers/{server}/channels")).await;
    let voice = voice
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap();

    // a message, edited, reacted to and pinned
    let msg = send(&app, &alex, text.id, "hello world").await.id;
    let r = app
        .http
        .patch(app.url(&format!("/messages/{msg}")))
        .bearer_auth(&alex)
        .json(&json!({ "content": "hello world, again" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "edit");
    let r = put_json(
        &app,
        &sam,
        &format!("/messages/{msg}/reactions/%F0%9F%91%8D"),
        json!({}),
    )
    .await;
    assert!(r.status().is_success(), "react {}", r.status());
    let r = put_json(
        &app,
        &alex,
        &format!("/messages/{msg}/pin"),
        json!({ "duration": "week" }),
    )
    .await;
    assert!(r.status().is_success(), "pin {}", r.status());
    // a nickname, in the server's name archive
    let r = put_json(
        &app,
        &alex,
        &format!("/servers/{server}/members/{sam_id}/nickname"),
        json!({ "nickname": "Sammy" }),
    )
    .await;
    assert!(r.status().is_success(), "nickname {}", r.status());
    // a poll and a plan
    let poll: Message = ok_json(
        post_json(
            &app,
            &alex,
            &format!("/channels/{}/polls", text.id),
            json!({ "question": "Pizza?", "options": ["yes", "no"], "ends_in": "day" }),
        )
        .await,
        "poll",
    )
    .await;
    let starts = (chrono::Utc::now() + chrono::TimeDelta::days(3)).to_rfc3339();
    let plan: Message = ok_json(
        post_json(
            &app,
            &alex,
            &format!("/channels/{}/plans", text.id),
            json!({ "title": "Braai", "starts_at": starts }),
        )
        .await,
        "plan",
    )
    .await;
    // files, one of them view once
    let file = upload(&app, &alex, text.id, false).await;
    let files: Message = ok_json(
        post_json(
            &app,
            &alex,
            &format!("/channels/{}/messages", text.id),
            json!({ "content": "a file", "attachments": [file] }),
        )
        .await,
        "files",
    )
    .await;
    let secret = upload(&app, &alex, text.id, true).await;
    let once: Message = ok_json(
        post_json(
            &app,
            &alex,
            &format!("/channels/{}/messages", text.id),
            json!({ "content": "", "attachments": [secret] }),
        )
        .await,
        "view once",
    )
    .await;
    // a quote with a reply in its thread
    let quotes = channel(&app, &alex, server, "quotes").await;
    let quote: Message = ok_json(
        post_json(
            &app,
            &alex,
            &format!("/channels/{}/quotes", quotes.id),
            json!({ "lines": [{ "speaker_id": sam_id, "text": "wisdom" }] }),
        )
        .await,
        "quote",
    )
    .await;
    let r = post_json(
        &app,
        &sam,
        &format!("/channels/{}/messages", quotes.id),
        json!({ "content": "so true", "thread_root_id": quote.id }),
    )
    .await;
    assert_eq!(r.status(), 200, "reply");
    // a clip, stored as an upload would leave it
    let clips = channel(&app, &alex, server, "clips").await;
    let clip = MessageId::new();
    for q in [
        format!(
            "INSERT INTO media (hash, kind, location, content_type, size, width, height, frames, created_at)
             VALUES ('clipvideo', 'clip', 'r2', 'video/mp4', 1, 640, 360, 1, '{}')",
            pulse_server::db::now()
        ),
        format!(
            "INSERT INTO media (hash, kind, location, content_type, size, width, height, frames, created_at)
             VALUES ('clipthumb', 'clip_thumb', 'disk', 'image/webp', 1, 640, 360, 1, '{}')",
            pulse_server::db::now()
        ),
        format!(
            "INSERT INTO messages (id, channel_id, author_id, kind, content, created_at) VALUES ('{clip}', '{}', '{alex_id}', 'normal', 'clip', '{}')",
            clips.id,
            pulse_server::db::now()
        ),
        format!(
            "INSERT INTO clips (message_id, video_key, thumb_hash, duration_ms, width, height) VALUES ('{clip}', 'clipvideo', 'clipthumb', 1000, 640, 360)"
        ),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(q.clone()))
            .execute(&app.db)
            .await
            .unwrap_or_else(|e| panic!("{q}: {e}"));
    }
    // notes to self, to forward into
    let mut notes = HashMap::new();
    for t in [&sam, &jo] {
        let ch = create_dm(&app, t, &[id_of(&app, t).await]).await;
        notes.insert(t.clone(), ch.id.to_string());
    }

    let tomorrow = (chrono::Utc::now() + chrono::TimeDelta::days(1)).to_rfc3339();
    let vars = HashMap::from([
        ("{server}", server.to_string()),
        ("{text}", text.id.to_string()),
        ("{voice}", voice.id.to_string()),
        ("{quotes}", quotes.id.to_string()),
        ("{clips}", clips.id.to_string()),
        ("{msg}", msg.to_string()),
        ("{poll}", poll.id.to_string()),
        ("{plan}", plan.id.to_string()),
        ("{files}", files.id.to_string()),
        ("{once}", once.id.to_string()),
        ("{quote}", quote.id.to_string()),
        ("{clip}", clip.to_string()),
        ("{alex}", alex_id.to_string()),
        ("{sam}", sam_id.to_string()),
        ("{tomorrow}", tomorrow),
    ]);
    World {
        app,
        alex,
        sam,
        jo,
        notes,
        vars,
    }
}

/// Whose token `token` is.
async fn id_of(app: &TestApp, token: &str) -> pulse_protocol::ids::UserId {
    let me: pulse_protocol::rest::User = get_json(app, token, "/me").await;
    me.id
}

impl World {
    fn fill(&self, s: &str, caller: &str) -> String {
        let mut out = s.replace(
            "{my_notes}",
            self.notes.get(caller).map_or("", String::as_str),
        );
        for (k, v) in &self.vars {
            out = out.replace(k, v);
        }
        out
    }
}

impl World {
    /// `read`'s call as `token`: (status, body).
    async fn call(&self, read: &Read, token: &str) -> (reqwest::StatusCode, String) {
        let (method, _) = read.route.split_once(' ').unwrap();
        let url = self.app.url(&self.fill(read.url, token));
        let req = match method {
            "GET" => self.app.http.get(url),
            "POST" => self.app.http.post(url),
            "PUT" => self.app.http.put(url),
            m => panic!("{m}"),
        };
        let req = match read.body {
            Some(b) => req
                .header("content-type", "application/json")
                .body(self.fill(b, token)),
            None => req,
        };
        let r = req.bearer_auth(token).send().await.unwrap();
        let status = r.status();
        (status, r.text().await.unwrap_or_default())
    }

    fn shows(&self, read: &Read, status: reqwest::StatusCode, body: &str) -> bool {
        status.is_success() && read.marker.is_none_or(|m| body.contains(&self.fill(m, "")))
    }
}

#[tokio::test]
async fn an_outsider_reads_what_the_matrix_says() {
    let w = world().await;
    let mut wrong = Vec::new();
    for read in MATRIX {
        if read.no_control.is_none() {
            let member = if read.owner_only { &w.alex } else { &w.sam };
            let (status, body) = w.call(read, member).await;
            if !w.shows(read, status, &body) {
                wrong.push(format!(
                    "{}: a member got {status}, so the fixture doesn't show this route working: {}",
                    read.route,
                    body.chars().take(200).collect::<String>()
                ));
                continue;
            }
        }
        let (status, body) = w.call(read, &w.jo).await;
        let got = if w.shows(read, status, &body) {
            ExpectedLeak
        } else if matches!(status.as_u16(), 403 | 404) || status.is_success() {
            Denied
        } else {
            wrong.push(format!(
                "{}: an outsider got {status}, neither a read nor a refusal: {body}",
                read.route
            ));
            continue;
        };
        if got != read.expect {
            wrong.push(format!(
                "{}: expected {:?}, got {got:?} ({status}): {}",
                read.route,
                read.expect,
                body.chars().take(200).collect::<String>()
            ));
        }
    }
    assert!(wrong.is_empty(), "\n{}\n", wrong.join("\n"));
}

/// Every route path registered under `src/` (each `.route("…")`), from the source.
fn registered_paths() -> BTreeSet<String> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut dirs = vec![root.clone()];
    let mut out = BTreeSet::new();
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                // the test fakes (S3, Steam, …) are other servers
                if path != root.join("testing") {
                    dirs.push(path);
                }
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (i, _) in text.match_indices(".route(") {
                let rest = text[i + ".route(".len()..].trim_start();
                let lit = rest.strip_prefix('"').unwrap_or_else(|| {
                    panic!(
                        "{}: a route whose path isn't a string literal: the coverage check can't see it",
                        path.display()
                    )
                });
                out.insert(lit[..lit.find('"').unwrap()].to_string());
            }
        }
    }
    out
}

/// `path` with each `{param}` filled with something no static segment is called.
fn concrete(path: &str) -> String {
    path.split('/')
        .map(|seg| if seg.starts_with('{') { "zz9" } else { seg })
        .collect::<Vec<_>>()
        .join("/")
}

#[tokio::test]
async fn every_route_is_classified() {
    let app = spawn().await;
    let mut routes = BTreeSet::new();
    for path in registered_paths() {
        // a method no route answers to: the router says which ones it does (405 with `Allow`)
        let r = app
            .http
            .request(reqwest::Method::TRACE, app.url(&concrete(&path)))
            .send()
            .await
            .unwrap();
        assert_eq!(
            r.status(),
            405,
            "{path}: TRACE should be refused with the methods allowed"
        );
        let allow = r
            .headers()
            .get("allow")
            .unwrap_or_else(|| panic!("{path}: no Allow header"))
            .to_str()
            .unwrap()
            .to_string();
        for m in allow.split(',').map(str::trim).filter(|m| !m.is_empty()) {
            if m == "HEAD" {
                continue; // axum answers HEAD wherever GET is
            }
            routes.insert(format!("{m} {path}"));
        }
    }
    let matrix: BTreeSet<String> = MATRIX.iter().map(|r| r.route.to_string()).collect();
    assert_eq!(matrix.len(), MATRIX.len(), "a route is in the matrix twice");
    let public: BTreeSet<String> = PUBLIC.iter().map(|s| s.to_string()).collect();
    let writes: BTreeSet<String> = WRITES.iter().map(|s| s.to_string()).collect();
    for (a, b, name) in [
        (&matrix, &public, "the matrix and PUBLIC"),
        (&matrix, &writes, "the matrix and WRITES"),
        (&public, &writes, "PUBLIC and WRITES"),
    ] {
        let both: Vec<_> = a.intersection(b).collect();
        assert!(both.is_empty(), "in both {name}: {both:?}");
    }
    let known: BTreeSet<String> = matrix
        .iter()
        .chain(&public)
        .chain(&writes)
        .cloned()
        .collect();
    let unclassified: Vec<_> = routes.difference(&known).collect();
    assert!(
        unclassified.is_empty(),
        "routes in neither the access matrix, PUBLIC nor WRITES (add each where it belongs; a read of a server's \
         insides goes in the matrix): {unclassified:#?}"
    );
    let stale: Vec<_> = known.difference(&routes).collect();
    assert!(stale.is_empty(), "listed but not registered: {stale:#?}");
}
