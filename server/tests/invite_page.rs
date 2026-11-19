//! The public side of invites (#80, step 3): the invite page, the Windows download, robots.txt, signing up through a
//! server owner's invite, and the traps.
mod common;

use common::*;
use pulse_protocol::rest::{ServerInvite, SessionResponse};
use serde_json::{Value, json};

async fn invite(app: &TestApp, token: &str, server: impl std::fmt::Display, body: Value) -> String {
    let r = post_json(app, token, &format!("/servers/{server}/invites"), body).await;
    assert_eq!(r.status(), 200);
    r.json::<ServerInvite>().await.unwrap().code
}

async fn get(app: &TestApp, path: &str) -> reqwest::Response {
    app.http.get(app.url(path)).send().await.unwrap()
}

async fn sign_up(
    app: &TestApp,
    code: &str,
    username: &str,
    website: Option<&str>,
) -> reqwest::Response {
    let mut body =
        json!({ "invite_code": code, "username": username, "password": "hunter2hunter2" });
    if let Some(w) = website {
        body["website"] = json!(w);
    }
    app.http
        .post(app.url("/auth/register"))
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// alex owns "Main" (with a secret channel) and sam is in it.
async fn world() -> (TestApp, String, String, pulse_protocol::ids::ServerId) {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let server = create_server(&app, &alex, "Main <Hangout>").await.id;
    assert_eq!(join_server(&app, &sam, server).await.status(), 200);
    let r = post_json(
        &app,
        &alex,
        &format!("/servers/{server}/channels"),
        json!({ "kind": "text", "name": "secret-plans" }),
    )
    .await;
    assert_eq!(r.status(), 200);
    (app, alex, sam, server)
}

#[tokio::test]
async fn the_page_shows_what_a_card_shows_a_stranger_and_nothing_inside() {
    let (app, alex, _, server) = world().await;
    let code = invite(
        &app,
        &alex,
        server,
        json!({ "max_uses": 5, "join_text": "Come hang" }),
    )
    .await;
    let r = get(&app, &format!("/i/{code}")).await;
    assert_eq!(r.status(), 200);
    let csp = r.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        csp.contains("default-src 'none'") && !csp.contains("script-src"),
        "{csp}"
    );
    assert_eq!(r.headers()["x-robots-tag"], "noindex, nofollow");
    assert_eq!(r.headers()["cache-control"], "no-store");
    let body = r.text().await.unwrap();
    assert!(
        body.contains("Main &lt;Hangout&gt;"),
        "escaped name: {body}"
    );
    assert!(body.contains("alex invited you to"));
    assert!(body.contains("2 members"));
    assert!(body.contains(&format!("href=\"pulse://invite/{code}\"")));
    assert!(body.contains("Come hang"));
    assert!(body.contains("/download/windows"));
    assert!(body.contains("noindex"));
    assert!(
        body.contains("sign up with this invite link"),
        "an owner's link signs up"
    );
    assert!(!body.contains("<script"));
    assert!(!body.contains("secret-plans") && !body.contains("general"));
}

#[tokio::test]
async fn unknown_revoked_expired_and_used_up_codes_get_the_same_page() {
    let (app, alex, sam, server) = world().await;
    let revoked = invite(&app, &alex, server, json!({})).await;
    let r = app
        .http
        .delete(app.url(&format!("/invites/{revoked}")))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let used = invite(&app, &alex, server, json!({ "max_uses": 1 })).await;
    let (_, kim) = register(&app, "kim").await;
    let r = post_json(&app, &kim, &format!("/invites/{used}/accept"), json!({})).await;
    assert_eq!(r.status(), 200);
    let expired = invite(&app, &sam, server, json!({})).await;
    sqlx::query("UPDATE server_invites SET expires_at = '2000-01-01T00:00:00.000Z' WHERE code = ?")
        .bind(&expired)
        .execute(&app.db)
        .await
        .unwrap();
    let unknown = get(&app, "/i/abcdefghjk").await;
    assert_eq!(unknown.status(), 404);
    let want = unknown.text().await.unwrap();
    assert!(!want.contains("Main"));
    for code in [&revoked, &used, &expired] {
        let r = get(&app, &format!("/i/{code}")).await;
        assert_eq!(r.status(), 404, "{code}");
        assert_eq!(r.text().await.unwrap(), want, "{code}");
    }
    // a member's link works on the page but doesn't sign anyone up
    let members = invite(&app, &sam, server, json!({})).await;
    let body = get(&app, &format!("/i/{members}"))
        .await
        .text()
        .await
        .unwrap();
    assert!(body.contains("Open it to join") && !body.contains("sign up"));
}

#[tokio::test]
async fn robots_keep_out() {
    let app = spawn().await;
    let r = get(&app, "/robots.txt").await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.unwrap(), "User-agent: *\nDisallow: /\n");
}

#[tokio::test]
async fn the_download_is_the_latest_published_installer() {
    let app = spawn().await;
    assert_eq!(
        get(&app, "/download/windows").await.status(),
        404,
        "nothing published"
    );
    let dir = &app.cfg.updates_dir;
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("Pulse_0.2.14_x64-setup.exe"), b"old").unwrap();
    std::fs::write(dir.join("Pulse_0.2.15_x64-setup.exe"), b"new installer").unwrap();
    std::fs::write(
        dir.join("latest.json"),
        br#"{"version":"0.2.15","platforms":{}}"#,
    )
    .unwrap();
    let r = get(&app, "/download/windows").await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers()["content-disposition"],
        "attachment; filename=\"Pulse_0.2.15_x64-setup.exe\""
    );
    assert_eq!(&r.bytes().await.unwrap()[..], b"new installer");
    // a manifest naming something that isn't an installer serves nothing
    std::fs::write(
        dir.join("latest.json"),
        br#"{"version":"../../etc/passwd"}"#,
    )
    .unwrap();
    assert_eq!(get(&app, "/download/windows").await.status(), 404);
}

#[tokio::test]
async fn downloads_are_rate_limited_per_address() {
    let mut cfg = test_config();
    cfg.limits.download_ip = pulse_server::limits::Rate::new(1, 0.0001);
    let app = spawn_with(cfg).await;
    assert_eq!(get(&app, "/download/windows").await.status(), 404);
    assert_eq!(get(&app, "/download/windows").await.status(), 429);
}

#[tokio::test]
async fn a_server_owners_link_signs_someone_up_into_its_server() {
    let (app, alex, sam, server) = world().await;
    let code = invite(&app, &alex, server, json!({ "max_uses": 2 })).await;
    // pasted as a whole link, as the app's sign-up accepts it
    let r = sign_up(
        &app,
        &format!("https://pulse.example/i/{code}"),
        "kim",
        None,
    )
    .await;
    assert_eq!(r.status(), 200);
    let kim: SessionResponse = r.json().await.unwrap();
    let servers: Vec<pulse_protocol::rest::Server> = get_json(&app, &kim.token, "/servers").await;
    assert_eq!(
        servers.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![server]
    );
    assert_eq!(
        count(
            &app,
            &format!(
                "SELECT COUNT(*) FROM server_invite_uses WHERE code = '{code}' AND user_id = '{}' AND created_account = 1",
                kim.user.id
            )
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &app,
            &format!("SELECT uses FROM server_invites WHERE code = '{code}'")
        )
        .await,
        1
    );
    // used up after the second
    assert_eq!(sign_up(&app, &code, "jo", None).await.status(), 200);
    assert_eq!(sign_up(&app, &code, "riley", None).await.status(), 404);
    // a member's link never signs anyone up
    let members = invite(&app, &sam, server, json!({})).await;
    assert_eq!(sign_up(&app, &members, "riley", None).await.status(), 404);
    // nor does a direct invite, even the owner's
    let (riley_id, _) = register(&app, "riley").await;
    let direct = invite(&app, &alex, server, json!({ "for_user": riley_id })).await;
    assert_eq!(sign_up(&app, &direct, "casey", None).await.status(), 404);
}

#[tokio::test]
async fn the_hidden_sign_up_field_blocks_that_address() {
    let (app, alex, _, server) = world().await;
    let code = invite(&app, &alex, server, json!({ "max_uses": 5 })).await;
    assert_eq!(
        sign_up(&app, &code, "kim", Some("spam.example"))
            .await
            .status(),
        404
    );
    assert_eq!(
        count(&app, "SELECT COUNT(*) FROM users WHERE username = 'kim'").await,
        0
    );
    // from then on that address can't sign up or look codes up
    assert_eq!(sign_up(&app, &code, "kim", None).await.status(), 429);
    assert_eq!(get(&app, &format!("/i/{code}")).await.status(), 429);
}

#[tokio::test]
async fn the_hidden_download_field_blocks_that_address() {
    let (app, alex, _, server) = world().await;
    let code = invite(&app, &alex, server, json!({})).await;
    assert_eq!(get(&app, "/download/windows?website=x").await.status(), 404);
    assert_eq!(get(&app, "/download/windows").await.status(), 429);
    assert_eq!(get(&app, &format!("/i/{code}")).await.status(), 429);
}

#[tokio::test]
async fn the_decoy_blocks_that_address_and_account() {
    let (app, alex, sam, server) = world().await;
    let code = invite(&app, &alex, server, json!({})).await;
    assert_eq!(get(&app, TEST_DECOY).await.status(), 404);
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM invite_failures WHERE kind = 'trap'"
        )
        .await,
        1
    );
    assert_eq!(get(&app, &format!("/i/{code}")).await.status(), 429);
    // the app's own lookups from that address too
    let r = app
        .http
        .get(app.url(&format!("/invites/{code}")))
        .bearer_auth(&sam)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 429);
}

#[tokio::test]
async fn a_decoy_outside_the_invite_pages_is_caught_too() {
    let mut cfg = test_config();
    cfg.invite_decoy = "/somewhere/else".into();
    let app = spawn_with(cfg).await;
    assert_eq!(get(&app, "/somewhere/else").await.status(), 404);
    assert_eq!(get(&app, "/somewhere/other").await.status(), 404);
    assert_eq!(
        count(
            &app,
            "SELECT COUNT(*) FROM invite_failures WHERE kind = 'trap'"
        )
        .await,
        1
    );
}
