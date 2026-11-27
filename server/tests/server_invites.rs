//! Server invites and bans (#80, step 1): who may make which invites, previews, accepting (and racing for) one,
//! revoking, direct invites and their DM card, bans, the lookup limits and blocks, the failure log and the sweeps.
mod common;

use std::time::Duration;

use common::*;
use jiff::{SignedDuration, Timestamp};
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{
    InvitePreview, InviteState, Message, NameStyle, ServerBan, ServerInvite, UpdateProfileRequest,
};
use pulse_server::limits::{LimitsConfig, caps, server_invites as rules};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// alex owns "Main"; sam is in it; kim and jo aren't.
struct World {
    app: TestApp,
    alex: (UserId, String),
    sam: (UserId, String),
    kim: (UserId, String),
    jo: (UserId, String),
    server: ServerId,
}

async fn world_with(app: TestApp) -> World {
    let alex = register(&app, "alex").await;
    let sam = register(&app, "sam").await;
    let kim = register(&app, "kim").await;
    let jo = register(&app, "jo").await;
    let server = create_server(&app, &alex.1, "Main").await.id;
    let r = join_server(&app, &sam.1, server).await;
    assert_eq!(r.status(), 200);
    World {
        app,
        alex,
        sam,
        kim,
        jo,
        server,
    }
}

async fn world() -> World {
    world_with(spawn().await).await
}

/// The defaults for the invite limits, everything else unlimited.
async fn limited_world() -> World {
    let mut cfg = test_config();
    let d = LimitsConfig::default();
    cfg.limits.server_invite_user = d.server_invite_user;
    cfg.limits.invite_lookup_ip = d.invite_lookup_ip;
    cfg.limits.invite_lookup_user = d.invite_lookup_user;
    world_with(spawn_with(cfg).await).await
}

async fn make(w: &World, token: &str, body: Value) -> reqwest::Response {
    post_json(
        &w.app,
        token,
        &format!("/servers/{}/invites", w.server),
        body,
    )
    .await
}

async fn make_ok(w: &World, token: &str, body: Value) -> ServerInvite {
    let r = make(w, token, body).await;
    assert_eq!(r.status(), 200, "make an invite");
    r.json().await.unwrap()
}

/// A lookup of `code` from `ip`: `GET /invites/{code}` (`accept` false) or `POST …/accept`.
async fn lookup(
    app: &TestApp,
    token: &str,
    code: &str,
    ip: &str,
    accept: bool,
) -> reqwest::Response {
    let req = if accept {
        app.http.post(app.url(&format!("/invites/{code}/accept")))
    } else {
        app.http.get(app.url(&format!("/invites/{code}")))
    };
    req.bearer_auth(token)
        .header("x-forwarded-for", ip)
        .header("user-agent", "invite-test/1")
        .send()
        .await
        .unwrap()
}

async fn preview(app: &TestApp, token: &str, code: &str) -> reqwest::Response {
    lookup(app, token, code, "10.0.0.1", false).await
}

async fn accept(app: &TestApp, token: &str, code: &str) -> reqwest::Response {
    lookup(app, token, code, "10.0.0.1", true).await
}

async fn preview_ok(app: &TestApp, token: &str, code: &str) -> InvitePreview {
    let r = preview(app, token, code).await;
    assert_eq!(r.status(), 200, "preview {code}");
    r.json().await.unwrap()
}

async fn error_code(r: reqwest::Response) -> String {
    let v: Value = r.json().await.unwrap();
    v["code"].as_str().unwrap_or_default().to_string()
}

async fn is_member(app: &TestApp, server: ServerId, user: UserId) -> bool {
    count(
        app,
        &format!("SELECT COUNT(*) FROM server_members WHERE server_id = '{server}' AND user_id = '{user}'"),
    )
    .await
        == 1
}

async fn failures(app: &TestApp, kind: &str) -> i64 {
    count(
        app,
        &format!("SELECT COUNT(*) FROM invite_failures WHERE kind = '{kind}'"),
    )
    .await
}

async fn sql(app: &TestApp, q: &str) {
    sqlx::query(sqlx::AssertSqlSafe(q.to_string()))
        .execute(&app.db)
        .await
        .unwrap();
}

async fn delete(app: &TestApp, token: &str, path: &str) -> reqwest::Response {
    app.http
        .delete(app.url(path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

async fn put(app: &TestApp, token: &str, path: &str) -> reqwest::Response {
    app.http
        .put(app.url(path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

/// The next event `rx` gets that `want` picks (others skipped); panics after 3 s.
async fn wait_for<T>(
    rx: &mut mpsc::Receiver<ServerFrame>,
    mut want: impl FnMut(Event) -> Option<T>,
) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let f = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("the event arrives")
            .expect("hub open");
        if let ServerFrame::Event(e) = f
            && let Some(t) = want(e)
        {
            return t;
        }
    }
}

#[tokio::test]
async fn a_new_invite_lasts_a_day_and_lets_one_person_in() {
    let w = world().await;
    let inv = make_ok(&w, &w.sam.1, json!({})).await;
    assert_eq!(inv.code.len(), 10);
    assert_eq!(inv.max_uses, Some(1));
    assert_eq!(inv.uses, 0);
    assert_eq!(inv.state, InviteState::Valid);
    let expires = pulse_server::times::parse(inv.expires_at.as_deref().unwrap()).unwrap();
    let left = expires.duration_since(Timestamp::now());
    assert!(
        left > SignedDuration::from_hours(23) && left <= SignedDuration::from_hours(24),
        "{left}"
    );
    // only the owner's links may later double as sign-up codes
    let owners = make_ok(&w, &w.alex.1, json!({})).await;
    let ok = |code: &str| format!("SELECT signup_ok FROM server_invites WHERE code = '{code}'");
    assert_eq!(count(&w.app, &ok(&inv.code)).await, 0);
    assert_eq!(count(&w.app, &ok(&owners.code)).await, 1);
}

#[tokio::test]
async fn members_are_capped_but_the_owner_isnt() {
    let w = world().await;
    let week = rules::MEMBER_MAX_AGE_SECS;
    let most = rules::MEMBER_MAX_USES;
    for body in [
        json!({ "max_age_secs": 0 }),
        json!({ "max_age_secs": week + 1 }),
        json!({ "max_uses": 0 }),
        json!({ "max_uses": most + 1 }),
    ] {
        assert_eq!(
            make(&w, &w.sam.1, body.clone()).await.status(),
            403,
            "{body}"
        );
    }
    let top = make_ok(
        &w,
        &w.sam.1,
        json!({ "max_age_secs": week, "max_uses": most }),
    )
    .await;
    assert_eq!(top.max_uses, Some(most));
    let forever = make_ok(&w, &w.alex.1, json!({ "max_age_secs": 0, "max_uses": 0 })).await;
    assert_eq!(forever.expires_at, None);
    assert_eq!(forever.max_uses, None);
    for body in [json!({ "max_age_secs": -1 }), json!({ "max_uses": -1 })] {
        assert_eq!(make(&w, &w.alex.1, body).await.status(), 400);
    }
    // not a member: no invites
    assert_eq!(make(&w, &w.kim.1, json!({})).await.status(), 403);
}

#[tokio::test]
async fn the_join_text_is_one_trimmed_short_line() {
    let w = world().await;
    let inv = make_ok(&w, &w.sam.1, json!({ "join_text": "  Come hang  " })).await;
    assert_eq!(inv.join_text.as_deref(), Some("Come hang"));
    let blank = make_ok(&w, &w.sam.1, json!({ "join_text": "   " })).await;
    assert_eq!(blank.join_text, None);
    let long = "x".repeat(pulse_protocol::rest::INVITE_JOIN_TEXT_CHARS + 1);
    assert_eq!(
        make(&w, &w.sam.1, json!({ "join_text": long }))
            .await
            .status(),
        400
    );
    assert_eq!(
        make(&w, &w.sam.1, json!({ "join_text": "two\nlines" }))
            .await
            .status(),
        400
    );
    let p = preview_ok(&w.app, &w.kim.1, &inv.code).await;
    assert_eq!(p.join_text.as_deref(), Some("Come hang"));
}

#[tokio::test]
async fn accepting_joins_counts_the_use_and_then_its_used_up() {
    let w = world().await;
    let inv = make_ok(&w, &w.sam.1, json!({})).await;
    let mut alex_rx = w.app.hub.register(w.alex.0, String::new()).rx;
    let p = preview_ok(&w.app, &w.kim.1, &inv.code).await;
    assert_eq!(p.state, InviteState::Valid);
    assert_eq!(p.name, "Main");
    assert_eq!(p.member_count, 2);
    assert_eq!(p.inviter_id, w.sam.0);
    assert_eq!(p.inviter_name, "sam");

    let r = accept(&w.app, &w.kim.1, &inv.code).await;
    assert_eq!(r.status(), 200);
    assert!(is_member(&w.app, w.server, w.kim.0).await);
    let joined = wait_for(&mut alex_rx, |e| match e {
        Event::MemberJoined { server_id, member } if server_id == w.server => Some(member.user.id),
        _ => None,
    })
    .await;
    assert_eq!(joined, w.kim.0);
    // the owner hears their Invites list changed
    wait_for(&mut alex_rx, |e| match e {
        Event::InvitesChanged { server_id, .. } if server_id == w.server => Some(()),
        _ => None,
    })
    .await;
    let code = &inv.code;
    assert_eq!(
        count(
            &w.app,
            &format!("SELECT uses FROM server_invites WHERE code = '{code}'")
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &w.app,
            &format!(
                "SELECT COUNT(*) FROM server_invite_uses WHERE code = '{code}' AND user_id = '{}' AND created_account = 0",
                w.kim.0
            )
        )
        .await,
        1
    );
    // kim starts at "all read" in its chats, as with Join
    assert!(
        count(
            &w.app,
            &format!(
                "SELECT COUNT(*) FROM read_states WHERE user_id = '{}'",
                w.kim.0
            )
        )
        .await
            >= 2
    );
    // already in: Open, and a second accept is refused
    assert_eq!(
        preview_ok(&w.app, &w.kim.1, code).await.state,
        InviteState::AlreadyMember
    );
    let r = accept(&w.app, &w.kim.1, code).await;
    assert_eq!(r.status(), 409);
    assert_eq!(error_code(r).await, "already_member");
    // used up for jo, and logged
    assert_eq!(
        preview_ok(&w.app, &w.jo.1, code).await.state,
        InviteState::UsedUp
    );
    let r = accept(&w.app, &w.jo.1, code).await;
    assert_eq!(r.status(), 409);
    assert_eq!(error_code(r).await, "invite_used_up");
    assert!(!is_member(&w.app, w.server, w.jo.0).await);
    assert_eq!(failures(&w.app, "used_up").await, 2);
}

#[tokio::test]
async fn two_people_racing_a_single_use_invite_give_one_member() {
    for _ in 0..3 {
        let w = world().await;
        let inv = make_ok(&w, &w.sam.1, json!({})).await;
        let (a, b) = tokio::join!(
            accept(&w.app, &w.kim.1, &inv.code),
            accept(&w.app, &w.jo.1, &inv.code)
        );
        let mut codes = [a.status().as_u16(), b.status().as_u16()];
        codes.sort();
        assert_eq!(codes, [200, 409]);
        let members = is_member(&w.app, w.server, w.kim.0).await as u8
            + is_member(&w.app, w.server, w.jo.0).await as u8;
        assert_eq!(members, 1);
        let code = &inv.code;
        assert_eq!(
            count(
                &w.app,
                &format!("SELECT uses FROM server_invites WHERE code = '{code}'")
            )
            .await,
            1
        );
    }
}

#[tokio::test]
async fn an_expired_invite_is_refused_and_logged() {
    let w = world().await;
    let inv = make_ok(&w, &w.sam.1, json!({ "max_uses": 5 })).await;
    let past = pulse_server::db::stamp(Timestamp::now() - SignedDuration::from_mins(1));
    sql(
        &w.app,
        &format!(
            "UPDATE server_invites SET expires_at = '{past}' WHERE code = '{}'",
            inv.code
        ),
    )
    .await;
    assert_eq!(
        preview_ok(&w.app, &w.kim.1, &inv.code).await.state,
        InviteState::Expired
    );
    let r = accept(&w.app, &w.kim.1, &inv.code).await;
    assert_eq!(r.status(), 409);
    assert_eq!(error_code(r).await, "invite_expired");
    assert!(!is_member(&w.app, w.server, w.kim.0).await);
    assert_eq!(failures(&w.app, "expired").await, 2);
}

#[tokio::test]
async fn unknown_and_revoked_codes_answer_the_same_404_and_are_logged() {
    let w = world().await;
    let inv = make_ok(&w, &w.sam.1, json!({ "max_uses": 5 })).await;
    // a member who didn't make it can't revoke it (nor tell it exists)
    let other = make_ok(&w, &w.alex.1, json!({})).await;
    assert_eq!(
        delete(&w.app, &w.sam.1, &format!("/invites/{}", other.code))
            .await
            .status(),
        404
    );
    assert_eq!(
        delete(&w.app, &w.kim.1, &format!("/invites/{}", inv.code))
            .await
            .status(),
        404
    );
    // its maker can
    assert_eq!(
        delete(&w.app, &w.sam.1, &format!("/invites/{}", inv.code))
            .await
            .status(),
        204
    );
    // and the owner can revoke anyone's
    assert_eq!(
        delete(&w.app, &w.alex.1, &format!("/invites/{}", other.code))
            .await
            .status(),
        204
    );

    let revoked = preview(&w.app, &w.kim.1, &inv.code).await;
    let unknown = preview(&w.app, &w.kim.1, "abcdefghij").await;
    assert_eq!(revoked.status(), 404);
    assert_eq!(unknown.status(), 404);
    let (rb, ub) = (revoked.text().await.unwrap(), unknown.text().await.unwrap());
    assert_eq!(rb, ub, "the same body for both");
    let r = accept(&w.app, &w.kim.1, &inv.code).await;
    assert_eq!(r.status(), 404);
    assert_eq!(r.text().await.unwrap(), ub);
    assert!(!is_member(&w.app, w.server, w.kim.0).await);
    assert_eq!(failures(&w.app, "revoked").await, 2);
    assert_eq!(failures(&w.app, "unknown").await, 1);
    // the log has who, from where and with what
    let (ip, user, agent, code): (String, Option<String>, Option<String>, String) = sqlx::query_as(
        "SELECT ip, user_id, user_agent, code FROM invite_failures WHERE kind = 'unknown'",
    )
    .fetch_one(&w.app.db)
    .await
    .unwrap();
    assert_eq!(ip, "10.0.0.1");
    assert_eq!(user, Some(w.kim.0.to_string()));
    assert_eq!(agent.as_deref(), Some("invite-test/1"));
    assert_eq!(code, "abcdefghij");
    // the revoked one still shows in the owner's list, as revoked
    let list: Vec<ServerInvite> =
        get_json(&w.app, &w.alex.1, &format!("/servers/{}/invites", w.server)).await;
    let row = list.iter().find(|i| i.code == inv.code).unwrap();
    assert_eq!(row.state, InviteState::Revoked);
}

#[tokio::test]
async fn members_list_their_own_invites_and_the_owner_all_of_them() {
    let w = world().await;
    let sams = make_ok(&w, &w.sam.1, json!({})).await;
    let alexs = make_ok(&w, &w.alex.1, json!({})).await;
    let path = format!("/servers/{}/invites", w.server);
    let mine: Vec<ServerInvite> = get_json(&w.app, &w.sam.1, &path).await;
    assert_eq!(
        mine.iter().map(|i| &i.code).collect::<Vec<_>>(),
        [&sams.code]
    );
    let all: Vec<ServerInvite> = get_json(&w.app, &w.alex.1, &path).await;
    assert_eq!(all.len(), 2);
    assert!(all.iter().any(|i| i.code == alexs.code));
    let r = w
        .app
        .http
        .get(w.app.url(&path))
        .bearer_auth(&w.kim.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn a_direct_invite_posts_a_card_in_the_dm_and_is_only_for_that_person() {
    let w = world().await;
    let mut kim_rx = w.app.hub.register(w.kim.0, String::new()).rx;
    let inv = make_ok(
        &w,
        &w.sam.1,
        json!({ "for_user": w.kim.0, "max_uses": 10, "join_text": "Hop in" }),
    )
    .await;
    assert_eq!(inv.for_user, Some(w.kim.0));
    assert_eq!(inv.max_uses, Some(1), "a direct invite is single use");
    // the card arrives in sam and kim's (new) DM, mentioning kim
    let card: Message = wait_for(&mut kim_rx, |e| match e {
        Event::MessageCreated { message, .. } if message.server_invite.is_some() => Some(message),
        _ => None,
    })
    .await;
    let attached = card.server_invite.clone().unwrap();
    assert_eq!(attached.code, inv.code);
    assert_eq!(attached.server_id, w.server);
    assert_eq!(card.author_id, Some(w.sam.0));
    assert_eq!(card.mentions, [w.kim.0]);
    assert!(card.content.contains("Main"), "{}", card.content);
    // it comes back with the card in history too, and reaches kim's inbox
    let page: Vec<Message> = get_json(
        &w.app,
        &w.kim.1,
        &format!("/channels/{}/messages", card.channel_id),
    )
    .await;
    assert_eq!(
        page[0].server_invite.as_ref().map(|c| &c.code),
        Some(&inv.code)
    );
    let inbox: Value = get_json(&w.app, &w.kim.1, "/inbox").await;
    assert!(inbox.to_string().contains(&card.id.to_string()), "{inbox}");
    // nobody edits it as text
    let r = w
        .app
        .http
        .patch(w.app.url(&format!("/messages/{}", card.id)))
        .bearer_auth(&w.sam.1)
        .json(&json!({ "content": "something else" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);

    // jo sees it's for kim, and can't use it
    let p = preview_ok(&w.app, &w.jo.1, &inv.code).await;
    assert_eq!(p.state, InviteState::ForSomeoneElse);
    assert_eq!(p.for_user_name.as_deref(), Some("kim"));
    assert_eq!(accept(&w.app, &w.jo.1, &inv.code).await.status(), 403);
    assert!(!is_member(&w.app, w.server, w.jo.0).await);
    // kim can
    assert_eq!(
        preview_ok(&w.app, &w.kim.1, &inv.code).await.state,
        InviteState::Valid
    );
    assert_eq!(accept(&w.app, &w.kim.1, &inv.code).await.status(), 200);
    assert!(is_member(&w.app, w.server, w.kim.0).await);
    // a second direct invite reuses the DM
    let r = make(&w, &w.sam.1, json!({ "for_user": w.jo.0 })).await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM channels WHERE kind = 'dm'").await,
        2,
        "one DM each with kim and jo"
    );
}

#[tokio::test]
async fn direct_invites_skip_yourself_members_the_banned_and_strangers() {
    let w = world().await;
    let me = |u: UserId| json!({ "for_user": u });
    assert_eq!(make(&w, &w.sam.1, me(w.sam.0)).await.status(), 400);
    let r = make(&w, &w.sam.1, me(w.alex.0)).await;
    assert_eq!(r.status(), 409);
    assert_eq!(error_code(r).await, "already_member");
    assert_eq!(make(&w, &w.sam.1, me(UserId::new())).await.status(), 400);
    assert_eq!(
        put(
            &w.app,
            &w.alex.1,
            &format!("/servers/{}/bans/{}", w.server, w.kim.0)
        )
        .await
        .status(),
        204
    );
    assert_eq!(make(&w, &w.sam.1, me(w.kim.0)).await.status(), 403);
    assert_eq!(
        count(&w.app, "SELECT COUNT(*) FROM server_invites").await,
        0,
        "nothing was made"
    );
}

#[tokio::test]
async fn a_ban_removes_them_revokes_their_invite_and_refuses_accepts() {
    let w = world().await;
    let link = make_ok(&w, &w.alex.1, json!({ "max_uses": 5 })).await;
    let direct = make_ok(&w, &w.sam.1, json!({ "for_user": w.kim.0 })).await;
    let ban = |u: UserId| format!("/servers/{}/bans/{u}", w.server);
    // only the owner bans, and not themselves
    assert_eq!(put(&w.app, &w.sam.1, &ban(w.kim.0)).await.status(), 403);
    assert_eq!(put(&w.app, &w.alex.1, &ban(w.alex.0)).await.status(), 400);
    assert_eq!(
        put(&w.app, &w.alex.1, &ban(UserId::new())).await.status(),
        404
    );

    let mut alex_rx = w.app.hub.register(w.alex.0, String::new()).rx;
    assert_eq!(put(&w.app, &w.alex.1, &ban(w.sam.0)).await.status(), 204);
    assert!(
        !is_member(&w.app, w.server, w.sam.0).await,
        "a banned member is taken out"
    );
    wait_for(&mut alex_rx, |e| match e {
        Event::BansChanged { server_id, .. } if server_id == w.server => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(put(&w.app, &w.alex.1, &ban(w.kim.0)).await.status(), 204);
    let d = &direct.code;
    assert_eq!(
        count(
            &w.app,
            &format!(
                "SELECT COUNT(*) FROM server_invites WHERE code = '{d}' AND revoked_at IS NOT NULL"
            )
        )
        .await,
        1,
        "the open direct invite to kim is revoked"
    );
    // banned: their accepts are refused, and the card says so
    assert_eq!(
        preview_ok(&w.app, &w.kim.1, &link.code).await.state,
        InviteState::Banned
    );
    assert_eq!(accept(&w.app, &w.kim.1, &link.code).await.status(), 403);
    assert!(!is_member(&w.app, w.server, w.kim.0).await);
    // the owner lists bans; nobody else does
    let path = format!("/servers/{}/bans", w.server);
    let bans: Vec<ServerBan> = get_json(&w.app, &w.alex.1, &path).await;
    assert_eq!(bans.len(), 2);
    assert!(bans.iter().all(|b| b.banned_by == Some(w.alex.0)));
    let r = w
        .app
        .http
        .get(w.app.url(&path))
        .bearer_auth(&w.jo.1)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    // unban: the link works again
    assert_eq!(delete(&w.app, &w.sam.1, &ban(w.kim.0)).await.status(), 403);
    assert_eq!(delete(&w.app, &w.alex.1, &ban(w.kim.0)).await.status(), 204);
    assert_eq!(delete(&w.app, &w.alex.1, &ban(w.kim.0)).await.status(), 404);
    assert_eq!(accept(&w.app, &w.kim.1, &link.code).await.status(), 200);
    assert!(is_member(&w.app, w.server, w.kim.0).await);
}

#[tokio::test]
async fn a_preview_shows_counts_and_only_faces_the_viewer_knows() {
    let w = world().await;
    // kim shares a DM with sam; jo knows nobody
    create_dm(&w.app, &w.kim.1, &[w.sam.0]).await;
    let inv = make_ok(&w, &w.alex.1, json!({ "max_uses": 5 })).await;
    let p = preview_ok(&w.app, &w.kim.1, &inv.code).await;
    assert_eq!(p.faces.iter().map(|u| u.id).collect::<Vec<_>>(), [w.sam.0]);
    assert_eq!(p.member_count, 2);
    assert_eq!(p.online_count, 0, "nobody is connected");
    let _alex_conn = w.app.hub.register(w.alex.0, String::new());
    let p = preview_ok(&w.app, &w.kim.1, &inv.code).await;
    assert_eq!(p.online_count, 1);
    let p = preview_ok(&w.app, &w.jo.1, &inv.code).await;
    assert!(p.faces.is_empty());
}

/// #708: a face who has blocked the viewer (#474) is the emptied user (no ring, no "is typing" text), as in a members
/// list; someone they haven't blocked gets the whole face.
#[tokio::test]
async fn a_blocked_viewer_gets_the_blockers_face_emptied() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let (_, jo) = register(&app, "jo").await;
    // jo's "Main" has alex in it; sam and kim know alex from another server
    let main = create_server(&app, &jo, "Main").await.id;
    assert_eq!(join_server(&app, &alex, main).await.status(), 200);
    acquaint(&app, &alex, &[&sam, &kim]).await;
    let r = app
        .http
        .put(app.url("/me/profile"))
        .bearer_auth(&alex)
        .json(&UpdateProfileRequest {
            avatar_hash: None,
            banner_hash: None,
            theme_top: "#8a3cff".into(),
            theme_bottom: "#ff8a2a".into(),
            style: NameStyle::default(),
            pronouns: None,
            bio: None,
            display_name: None,
            glow: None,
            typing_text: Some("cooking".into()),
            ring_style: Some("orbit".into()),
            ring_color: Some("gold".into()),
            card_stats: None,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "alex styles their profile");
    let r = app
        .http
        .put(app.url(&format!("/me/blocks/{sam_id}")))
        .bearer_auth(&alex)
        .json(&json!({ "mode": "line" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "alex blocks sam");
    let r = post_json(
        &app,
        &jo,
        &format!("/servers/{main}/invites"),
        json!({ "max_uses": 5 }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let inv: ServerInvite = r.json().await.unwrap();

    let face = |p: &InvitePreview| p.faces.iter().find(|u| u.id == alex_id).cloned();
    let looks = |u: &pulse_protocol::rest::User| {
        (
            u.typing_text.clone(),
            u.ring_style.clone(),
            u.ring_color.clone(),
        )
    };
    let to_sam = face(&preview_ok(&app, &sam, &inv.code).await).expect("alex is a face to sam");
    assert_eq!(looks(&to_sam), (None, None, None), "sam gets alex emptied");
    let to_kim = face(&preview_ok(&app, &kim, &inv.code).await).expect("alex is a face to kim");
    assert_eq!(
        looks(&to_kim),
        (
            Some("cooking".to_owned()),
            Some("orbit".to_owned()),
            Some("gold".to_owned())
        ),
        "kim gets alex's whole face"
    );
}

#[tokio::test]
async fn lookups_are_limited_per_address_and_per_account() {
    let w = limited_world().await;
    let inv = make_ok(&w, &w.alex.1, json!({ "max_uses": 0, "max_age_secs": 0 })).await;
    let per_hour = LimitsConfig::default().invite_lookup_ip.burst;
    // one address, several accounts
    for i in 0..per_hour {
        let t = if i % 2 == 0 { &w.kim.1 } else { &w.jo.1 };
        assert_eq!(
            lookup(&w.app, t, &inv.code, "10.1.0.1", false)
                .await
                .status(),
            200
        );
    }
    assert_eq!(
        lookup(&w.app, &w.sam.1, &inv.code, "10.1.0.1", false)
            .await
            .status(),
        429
    );
    assert_eq!(
        lookup(&w.app, &w.sam.1, &inv.code, "10.1.0.1", true)
            .await
            .status(),
        429
    );
    assert_eq!(
        lookup(&w.app, &w.sam.1, &inv.code, "10.1.0.2", false)
            .await
            .status(),
        200
    );
    // one account, several addresses (kim and jo spent theirs above: half each)
    let mut spent = per_hour / 2;
    while spent < per_hour {
        let ip = format!("10.2.0.{spent}");
        assert_eq!(
            lookup(&w.app, &w.kim.1, &inv.code, &ip, false)
                .await
                .status(),
            200
        );
        spent += 1;
    }
    assert_eq!(
        lookup(&w.app, &w.kim.1, &inv.code, "10.3.0.1", false)
            .await
            .status(),
        429
    );
}

#[tokio::test]
async fn too_many_unknown_codes_block_lookups_for_a_while() {
    let w = world().await;
    let inv = make_ok(&w, &w.alex.1, json!({ "max_uses": 0 })).await;
    let n = rules::UNKNOWN_CODES_TO_BLOCK;
    // from one address (several accounts)
    for i in 0..n {
        let t = if i % 2 == 0 { &w.kim.1 } else { &w.jo.1 };
        let code = format!("zzzzzzzzz{}", (b'a' + i as u8) as char);
        assert_eq!(
            lookup(&w.app, t, &code, "10.9.0.1", false).await.status(),
            404
        );
    }
    // even a good code, from that address, by anyone
    assert_eq!(
        lookup(&w.app, &w.sam.1, &inv.code, "10.9.0.1", false)
            .await
            .status(),
        429
    );
    assert_eq!(
        lookup(&w.app, &w.sam.1, &inv.code, "10.9.0.1", true)
            .await
            .status(),
        429
    );
    assert_eq!(
        lookup(&w.app, &w.sam.1, &inv.code, "10.9.0.2", false)
            .await
            .status(),
        200
    );

    // from one account (several addresses)
    for i in 0..n {
        let ip = format!("10.8.0.{i}");
        assert_eq!(
            lookup(&w.app, &w.sam.1, "qqqqqqqqqq", &ip, false)
                .await
                .status(),
            404
        );
    }
    assert_eq!(
        lookup(&w.app, &w.sam.1, &inv.code, "10.7.0.1", false)
            .await
            .status(),
        429
    );

    // a day later, both are free again
    let day = pulse_server::db::stamp(
        Timestamp::now() - SignedDuration::from_secs(rules::BLOCK_SECS + 60),
    );
    sql(&w.app, &format!("UPDATE invite_failures SET at = '{day}'")).await;
    assert_eq!(
        lookup(&w.app, &w.sam.1, &inv.code, "10.9.0.1", false)
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn expired_and_revoked_codes_never_block() {
    let w = world().await;
    let dead = make_ok(&w, &w.alex.1, json!({})).await;
    let good = make_ok(&w, &w.alex.1, json!({ "max_uses": 0 })).await;
    let past = pulse_server::db::stamp(Timestamp::now() - SignedDuration::from_mins(1));
    sql(
        &w.app,
        &format!(
            "UPDATE server_invites SET expires_at = '{past}' WHERE code = '{}'",
            dead.code
        ),
    )
    .await;
    for _ in 0..rules::UNKNOWN_CODES_TO_BLOCK + 2 {
        assert_eq!(
            lookup(&w.app, &w.kim.1, &dead.code, "10.6.0.1", false)
                .await
                .status(),
            200
        );
    }
    assert_eq!(
        delete(&w.app, &w.alex.1, &format!("/invites/{}", dead.code))
            .await
            .status(),
        204
    );
    for _ in 0..rules::UNKNOWN_CODES_TO_BLOCK + 2 {
        assert_eq!(
            lookup(&w.app, &w.kim.1, &dead.code, "10.6.0.1", false)
                .await
                .status(),
            404
        );
    }
    assert_eq!(
        lookup(&w.app, &w.kim.1, &good.code, "10.6.0.1", false)
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn making_invites_is_rate_limited_per_member() {
    let w = limited_world().await;
    let per_hour = LimitsConfig::default().server_invite_user.burst;
    for _ in 0..per_hour {
        assert_eq!(make(&w, &w.sam.1, json!({})).await.status(), 200);
    }
    assert_eq!(make(&w, &w.sam.1, json!({})).await.status(), 429);
    assert_eq!(
        make(&w, &w.alex.1, json!({})).await.status(),
        200,
        "per member"
    );
}

/// Store `n` invites made by `by` straight into the DB (`extra` sets more columns).
async fn seed(w: &World, by: UserId, n: i64, extra: &str) {
    let at = pulse_server::db::now();
    for i in 0..n {
        sql(
            &w.app,
            &format!(
                "INSERT INTO server_invites (code, server_id, created_by, created_at{}) VALUES ('seed{i:06}', '{}', '{by}', '{at}'{})",
                if extra.is_empty() { "" } else { ", for_user" },
                w.server,
                if extra.is_empty() { String::new() } else { format!(", '{extra}'") },
            ),
        )
        .await;
    }
}

#[tokio::test]
async fn a_server_holds_a_limited_number_of_live_invites() {
    let w = world().await;
    seed(&w, w.alex.0, caps::LIVE_INVITES_PER_SERVER, "").await;
    assert_eq!(make(&w, &w.sam.1, json!({})).await.status(), 400);
    // one stops working: room for another
    sql(&w.app, "UPDATE server_invites SET revoked_at = '2020-01-01T00:00:00.000Z' WHERE code = 'seed000000'").await;
    assert_eq!(make(&w, &w.sam.1, json!({})).await.status(), 200);
}

#[tokio::test]
async fn direct_invites_are_limited_per_day() {
    let w = world().await;
    seed(&w, w.sam.0, rules::DIRECT_PER_DAY, &w.jo.0.to_string()).await;
    assert_eq!(
        make(&w, &w.sam.1, json!({ "for_user": w.kim.0 }))
            .await
            .status(),
        429
    );
    assert_eq!(
        make(&w, &w.sam.1, json!({})).await.status(),
        200,
        "links aren't direct invites"
    );
    // a day on, they may again
    let old = pulse_server::db::stamp(Timestamp::now() - SignedDuration::from_hours(25));
    sql(
        &w.app,
        &format!("UPDATE server_invites SET created_at = '{old}' WHERE for_user IS NOT NULL"),
    )
    .await;
    assert_eq!(
        make(&w, &w.sam.1, json!({ "for_user": w.kim.0 }))
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn the_sweep_drops_long_dead_invites_and_old_failures() {
    let w = world().await;
    let now = Timestamp::now();
    let ago = |days: i64| pulse_server::db::stamp(now - pulse_server::times::days(days));
    let keep = rules::DEAD_RETENTION_DAYS - 1;
    let drop = rules::DEAD_RETENTION_DAYS + 1;
    let (s, a) = (w.server, w.alex.0);
    let at = ago(400);
    for (code, cols, vals) in [
        ("revokedold", "revoked_at", ago(drop)),
        ("revokednew", "revoked_at", ago(keep)),
        ("expiredold", "expires_at", ago(drop)),
        ("expirednew", "expires_at", ago(keep)),
    ] {
        sql(&w.app, &format!("INSERT INTO server_invites (code, server_id, created_by, created_at, {cols}) VALUES ('{code}', '{s}', '{a}', '{at}', '{vals}')")).await;
    }
    // used up: by when its last use was
    for (code, used) in [("usedupold", ago(drop)), ("usedupnew", ago(keep))] {
        sql(&w.app, &format!("INSERT INTO server_invites (code, server_id, created_by, created_at, max_uses, uses) VALUES ('{code}', '{s}', '{a}', '{at}', 1, 1)")).await;
        sql(&w.app, &format!("INSERT INTO server_invite_uses (code, user_id, used_at) VALUES ('{code}', '{}', '{used}')", w.sam.0)).await;
    }
    // still working, however old
    sql(&w.app, &format!("INSERT INTO server_invites (code, server_id, created_by, created_at) VALUES ('liveforever', '{s}', '{a}', '{at}')")).await;
    let fkeep = rules::FAILURE_RETENTION_DAYS - 1;
    let fdrop = rules::FAILURE_RETENTION_DAYS + 1;
    for (code, days) in [("old", fdrop), ("new", fkeep)] {
        sql(&w.app, &format!("INSERT INTO invite_failures (at, ip, code, kind) VALUES ('{}', '10.0.0.1', '{code}', 'unknown')", ago(days))).await;
    }

    let pruned = pulse_server::hygiene::prune(&w.app.db, now).await.unwrap();
    assert_eq!(pruned.server_invites, 3);
    assert_eq!(pruned.invite_failures, 1);
    let left: Vec<String> = sqlx::query_scalar("SELECT code FROM server_invites ORDER BY code")
        .fetch_all(&w.app.db)
        .await
        .unwrap();
    assert_eq!(
        left,
        ["expirednew", "liveforever", "revokednew", "usedupnew"]
    );
    let codes: Vec<String> = sqlx::query_scalar("SELECT code FROM invite_failures")
        .fetch_all(&w.app.db)
        .await
        .unwrap();
    assert_eq!(codes, ["new"]);
}

async fn patch(app: &TestApp, token: &str, code: &str, body: Value) -> reqwest::Response {
    app.http
        .patch(app.url(&format!("/invites/{code}")))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn its_maker_changes_a_link_and_the_code_stays() {
    let w = world().await;
    let inv = make_ok(&w, &w.sam.1, json!({})).await;
    let code = inv.code.clone();
    // the join text, trimmed; then cleared
    let r = patch(
        &w.app,
        &w.sam.1,
        &code,
        json!({ "join_text": "  Come hang " }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let got: ServerInvite = r.json().await.unwrap();
    assert_eq!(got.code, code);
    assert_eq!(got.join_text.as_deref(), Some("Come hang"));
    assert_eq!(got.max_uses, Some(1), "absent fields stay as they were");
    let got: ServerInvite = patch(&w.app, &w.sam.1, &code, json!({ "join_text": "" }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(got.join_text, None);
    // how long it lasts (from when it was made) and its uses
    let got: ServerInvite = patch(
        &w.app,
        &w.sam.1,
        &code,
        json!({ "max_age_secs": 3600, "max_uses": 5 }),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(got.max_uses, Some(5));
    let made = pulse_server::times::parse(&got.created_at).unwrap();
    let ends = pulse_server::times::parse(got.expires_at.as_deref().unwrap()).unwrap();
    assert_eq!(ends.duration_since(made).as_secs(), 3600);
    // a member's limits still apply; the owner's don't
    assert_eq!(
        patch(&w.app, &w.sam.1, &code, json!({ "max_uses": 0 }))
            .await
            .status(),
        403
    );
    let mine = make_ok(&w, &w.alex.1, json!({})).await;
    let got: ServerInvite = patch(
        &w.app,
        &w.alex.1,
        &mine.code,
        json!({ "max_age_secs": 0, "max_uses": 0 }),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!((got.expires_at, got.max_uses), (None, None));
    // one line, not too long
    assert_eq!(
        patch(&w.app, &w.sam.1, &code, json!({ "join_text": "a\nb" }))
            .await
            .status(),
        400
    );
    // still one invite: nothing new was made
    let list: Vec<ServerInvite> =
        get_json(&w.app, &w.alex.1, &format!("/servers/{}/invites", w.server)).await;
    assert_eq!(list.iter().filter(|i| i.code == code).count(), 1);
}

#[tokio::test]
async fn only_its_maker_changes_a_link_and_only_while_it_works() {
    let w = world().await;
    let inv = make_ok(&w, &w.sam.1, json!({})).await;
    // not even the owner, nor an outsider: as for an unknown code
    assert_eq!(
        patch(&w.app, &w.alex.1, &inv.code, json!({ "join_text": "x" }))
            .await
            .status(),
        404
    );
    assert_eq!(
        patch(&w.app, &w.kim.1, &inv.code, json!({ "join_text": "x" }))
            .await
            .status(),
        404
    );
    assert_eq!(
        patch(&w.app, &w.sam.1, "nopenopeno", json!({}))
            .await
            .status(),
        404
    );
    // a direct invite can't be changed
    let direct = make_ok(&w, &w.sam.1, json!({ "for_user": w.kim.0 })).await;
    assert_eq!(
        patch(&w.app, &w.sam.1, &direct.code, json!({ "join_text": "x" }))
            .await
            .status(),
        400
    );
    // a revoked one neither
    assert_eq!(
        delete(&w.app, &w.sam.1, &format!("/invites/{}", inv.code))
            .await
            .status(),
        204
    );
    assert_eq!(
        patch(&w.app, &w.sam.1, &inv.code, json!({ "join_text": "x" }))
            .await
            .status(),
        400
    );
    // a use limit can't drop below the uses it has had
    let five = make_ok(&w, &w.sam.1, json!({ "max_uses": 5 })).await;
    assert_eq!(accept(&w.app, &w.kim.1, &five.code).await.status(), 200);
    assert_eq!(accept(&w.app, &w.jo.1, &five.code).await.status(), 200);
    assert_eq!(
        patch(&w.app, &w.sam.1, &five.code, json!({ "max_uses": 1 }))
            .await
            .status(),
        400
    );
}
