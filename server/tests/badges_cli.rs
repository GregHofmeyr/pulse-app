//! The `badge` admin/test command (#49): award, reset, stats and set-stat against the real binary.
mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::rest::{Badge, Profile, SessionResponse};
use serde_json::json;

/// Register `username` on the running binary; (id, token).
async fn register_on(p: &ServerProcess, username: &str) -> SessionResponse {
    let r = reqwest::Client::new()
        .post(p.url("/auth/register"))
        .json(&json!({"invite_code": binary_invite(p), "username": username, "password": "hunter2hunter2"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "register {username}");
    r.json().await.unwrap()
}

async fn badges(p: &ServerProcess, s: &SessionResponse) -> Vec<Badge> {
    let r = reqwest::Client::new()
        .get(p.url(&format!("/users/{}/profile", s.user.id)))
        .bearer_auth(&s.token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json::<Profile>().await.unwrap().badges
}

fn badge(p: &ServerProcess, args: &[&str]) -> (bool, String) {
    let args: Vec<&str> = ["badge"].iter().chain(args).copied().collect();
    run_binary(&p.db_url, &[], &args, Duration::from_secs(20))
}

#[tokio::test]
async fn award_founder_then_profile_shows_it_unseen() {
    let p = spawn_binary().await;
    let alex = register_on(&p, "alex").await;

    let (ok, out) = badge(&p, &["award", "alex", "founder"]);
    assert!(ok, "{out}");
    assert!(out.contains("awarded founder tier 1 to alex"), "{out}");

    // The running server wasn't told, but the next profile load shows it, unseen ([NEW]).
    let b = badges(&p, &alex).await;
    let f = b.iter().find(|b| b.key == "founder").expect("founder held");
    assert_eq!(f.tier, 1);
    assert!(!f.seen);
}

#[tokio::test]
async fn reset_tier_and_stats() {
    let p = spawn_binary().await;
    let alex = register_on(&p, "alex").await;

    let (ok, out) = badge(&p, &["award", "alex", "the_yapper", "--tier", "3"]);
    assert!(ok, "{out}");
    assert!(out.contains("awarded the_yapper tier 3 to alex"), "{out}");
    let (ok, out) = badge(&p, &["reset", "alex", "the_yapper", "--tier", "2"]);
    assert!(ok, "{out}");
    assert!(out.contains("reset the_yapper for alex"), "{out}");
    let b = badges(&p, &alex).await;
    assert_eq!(b.iter().find(|b| b.key == "the_yapper").unwrap().tier, 1);

    let (ok, out) = badge(&p, &["set-stat", "alex", "messages", "99"]);
    assert!(ok, "{out}");
    let (ok, out) = badge(&p, &["stats", "alex"]);
    assert!(ok, "{out}");
    assert!(out.lines().any(|l| l.trim() == "messages 99"), "{out}");
    assert!(
        out.lines().any(|l| l.trim() == "the_yapper tier 1"),
        "{out}"
    );

    let (ok, out) = badge(&p, &["reset", "alex", "the_yapper"]);
    assert!(ok, "{out}");
    assert!(badges(&p, &alex).await.is_empty());
}

#[tokio::test]
async fn unknown_key_fails() {
    let p = spawn_binary().await;
    register_on(&p, "alex").await;

    let (ok, out) = badge(&p, &["award", "alex", "nope"]);
    assert!(!ok, "{out}");
    assert!(
        out.contains("founder") && out.contains("the_yapper"),
        "{out}"
    );
    let (ok, out) = badge(&p, &["award", "alex", "founder", "--tier", "2"]);
    assert!(!ok, "{out}");
    let (ok, out) = badge(&p, &["award", "kim", "founder"]);
    assert!(!ok, "{out}");
    assert!(out.contains("no user named kim"), "{out}");
    let (ok, out) = badge(&p, &["set-stat", "alex", "mesages", "1"]);
    assert!(!ok, "{out}");
}
