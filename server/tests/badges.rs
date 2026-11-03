//! Badges (#49): counters, the engine (tiers, first earners, BadgeEarned) and grant/revoke.
mod common;

use std::time::Duration;

use common::*;
use futures::SinkExt;
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{Badge, BadgeRarity, Profile};
use pulse_server::badges::{catalogue::stat, engine, stats};
use tokio_tungstenite::tungstenite::Message as Ws;

async fn hello(app: &TestApp, token: &str) -> Socket {
    let (mut ws, _) = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap();
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    match next_frame_beating(&mut ws, Duration::from_secs(3)).await {
        Some(ServerFrame::Ready(_)) => ws,
        other => panic!("expected Ready, got {other:?}"),
    }
}

/// The `BadgeEarned` events that arrive before the socket goes quiet.
async fn earned(ws: &mut Socket) -> Vec<(UserId, Badge, String)> {
    let mut out = Vec::new();
    while let Some(f) = next_frame_beating(ws, Duration::from_millis(300)).await {
        if let ServerFrame::Event(Event::BadgeEarned {
            user_id,
            badge,
            name,
            ..
        }) = f
        {
            out.push((user_id, badge, name));
        }
    }
    out
}

async fn add(app: &TestApp, user: UserId, key: &str, delta: i64) -> i64 {
    let mut c = app.db.acquire().await.unwrap();
    stats::add(&mut c, user, key, delta).await.unwrap()
}

async fn badges_of(app: &TestApp, token: &str, user: UserId) -> Vec<Badge> {
    get_json::<Profile>(app, token, &format!("/users/{user}/profile"))
        .await
        .badges
}

fn held<'a>(badges: &'a [Badge], key: &str) -> Option<&'a Badge> {
    badges.iter().find(|b| b.key == key)
}

#[tokio::test]
async fn stats_add_floors_at_zero_and_set_max_only_raises() {
    let app = spawn().await;
    let (alex_id, _) = register(&app, "alex").await;
    assert_eq!(add(&app, alex_id, stat::MESSAGES, 3).await, 3);
    assert_eq!(add(&app, alex_id, stat::MESSAGES, -10).await, 0);
    assert_eq!(add(&app, alex_id, stat::CLIPS, -1).await, 0);
    let mut c = app.db.acquire().await.unwrap();
    assert_eq!(
        stats::set_max(&mut c, alex_id, stat::VENDETTA_MAX, 4)
            .await
            .unwrap(),
        4
    );
    assert_eq!(
        stats::set_max(&mut c, alex_id, stat::VENDETTA_MAX, 2)
            .await
            .unwrap(),
        4
    );
    stats::set(&mut c, alex_id, stat::GHOST, 7).await.unwrap();
    assert_eq!(stats::get(&mut c, alex_id, stat::GHOST).await.unwrap(), 7);
    assert_eq!(stats::get(&mut c, alex_id, stat::DOOS).await.unwrap(), 0);
    let all = stats::all(&mut c, alex_id).await.unwrap();
    assert_eq!(all.get(stat::VENDETTA_MAX), Some(&4));
    assert_eq!(all.get(stat::GHOST), Some(&7));
}

#[tokio::test]
async fn counters_reach_a_tier_and_publish_only_to_the_earner() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let mut alex_ws = hello(&app, &alex).await;
    let mut sam_ws = hello(&app, &sam).await;
    earned(&mut alex_ws).await;
    earned(&mut sam_ws).await;

    add(&app, alex_id, stat::MESSAGES, 100).await;
    let raised = engine::check(&app.state(), alex_id).await.unwrap();
    assert_eq!(raised.len(), 1);
    assert_eq!(raised[0].key, "the_yapper");
    assert_eq!(raised[0].tier, 1);
    assert_eq!(raised[0].rarity, BadgeRarity::Common);
    assert!(raised[0].first_earner);

    let got = earned(&mut alex_ws).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, alex_id);
    assert_eq!(got[0].1.key, "the_yapper");
    assert_eq!(got[0].2, "The Yapper");
    assert!(earned(&mut sam_ws).await.is_empty(), "sam saw alex's badge");
}

#[tokio::test]
async fn each_tier_is_its_own_unlock() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let s = app.state();

    add(&app, alex_id, stat::MESSAGES, 99).await;
    assert!(engine::check(&s, alex_id).await.unwrap().is_empty());
    assert!(badges_of(&app, &alex, alex_id).await.is_empty());

    add(&app, alex_id, stat::MESSAGES, 1).await;
    let raised = engine::check(&s, alex_id).await.unwrap();
    assert_eq!(raised.iter().map(|b| b.tier).collect::<Vec<_>>(), [1]);
    let first = held(&badges_of(&app, &alex, alex_id).await, "the_yapper")
        .unwrap()
        .clone();
    assert_eq!(first.tier, 1);
    assert!(!first.seen);
    // Nothing new: no unlock.
    assert!(engine::check(&s, alex_id).await.unwrap().is_empty());

    sqlx::query("UPDATE user_badges SET seen = 1")
        .execute(&app.db)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    add(&app, alex_id, stat::MESSAGES, 400).await;
    let raised = engine::check(&s, alex_id).await.unwrap();
    assert_eq!(raised.iter().map(|b| b.tier).collect::<Vec<_>>(), [2]);
    assert_eq!(raised[0].rarity, BadgeRarity::Uncommon);
    let second = held(&badges_of(&app, &alex, alex_id).await, "the_yapper")
        .unwrap()
        .clone();
    assert_eq!(second.tier, 2);
    assert!(second.earned_at > first.earned_at);
    assert!(!second.seen, "a new tier is unseen again");
    assert_eq!(second.first_tiers, [1, 2]);
}

#[tokio::test]
async fn first_earner_is_per_tier() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, _) = register(&app, "sam").await;
    let s = app.state();

    add(&app, alex_id, stat::MESSAGES, 100).await;
    engine::check(&s, alex_id).await.unwrap();
    add(&app, sam_id, stat::MESSAGES, 100).await;
    let raised = engine::check(&s, sam_id).await.unwrap();
    assert!(!raised[0].first_earner, "alex was first at tier 1");
    add(&app, sam_id, stat::MESSAGES, 400).await;
    let raised = engine::check(&s, sam_id).await.unwrap();
    assert!(raised[0].first_earner, "sam was first at tier 2");
    add(&app, alex_id, stat::MESSAGES, 400).await;
    engine::check(&s, alex_id).await.unwrap();

    let a = badges_of(&app, &alex, alex_id).await;
    let a = held(&a, "the_yapper").unwrap();
    assert_eq!(
        (a.tier, a.first_tiers.clone(), a.first_earner),
        (2, vec![1], false)
    );
    let sm = badges_of(&app, &alex, sam_id).await;
    let sm = held(&sm, "the_yapper").unwrap();
    assert_eq!(
        (sm.tier, sm.first_tiers.clone(), sm.first_earner),
        (2, vec![2], true)
    );
}

#[tokio::test]
async fn first_earner_race_has_one_winner() {
    let app = spawn().await;
    let (alex_id, _) = register(&app, "alex").await;
    let (sam_id, _) = register(&app, "sam").await;
    add(&app, alex_id, stat::MESSAGES, 100).await;
    add(&app, sam_id, stat::MESSAGES, 100).await;
    let s = app.state();
    let (a, b) = tokio::join!(engine::check(&s, alex_id), engine::check(&s, sam_id));
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!((a.len(), b.len()), (1, 1));
    assert!(a[0].first_earner ^ b[0].first_earner, "exactly one first");
    let rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM badge_firsts WHERE badge_key = 'the_yapper' AND tier = 1",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn counters_going_down_never_take_a_badge() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    add(&app, alex_id, stat::MESSAGES, 100).await;
    engine::check(&app.state(), alex_id).await.unwrap();
    add(&app, alex_id, stat::MESSAGES, -50).await;
    assert!(
        engine::check(&app.state(), alex_id)
            .await
            .unwrap()
            .is_empty()
    );
    let b = badges_of(&app, &alex, alex_id).await;
    assert_eq!(held(&b, "the_yapper").unwrap().tier, 1);
}

#[tokio::test]
async fn stacking_badges_carry_their_count() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    add(&app, alex_id, stat::QOTY_WINS, 2).await;
    engine::check(&app.state(), alex_id).await.unwrap();
    let b = badges_of(&app, &alex, alex_id).await;
    let q = held(&b, "quote_of_the_year").unwrap();
    assert_eq!((q.count, q.rarity), (2, BadgeRarity::Legendary));
}

#[tokio::test]
async fn after_never_fails_and_checks_everyone() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, _) = register(&app, "sam").await;
    add(&app, alex_id, stat::MESSAGES, 100).await;
    add(&app, sam_id, stat::CLIPS, 5).await;
    engine::after(&app.state(), &[alex_id, sam_id]).await;
    assert!(held(&badges_of(&app, &alex, alex_id).await, "the_yapper").is_some());
    assert!(held(&badges_of(&app, &alex, sam_id).await, "clipper").is_some());
}

#[tokio::test]
async fn grant_and_revoke() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;

    engine::grant(&app.db, alex_id, "founder", 1).await.unwrap();
    let b = badges_of(&app, &alex, alex_id).await;
    let f = held(&b, "founder").unwrap();
    assert_eq!(
        (f.tier, f.rarity, f.seen),
        (1, BadgeRarity::Mythical, false)
    );
    assert!(f.first_earner);

    engine::grant(&app.db, alex_id, "the_yapper", 3)
        .await
        .unwrap();
    let b = badges_of(&app, &alex, alex_id).await;
    assert_eq!(held(&b, "the_yapper").unwrap().first_tiers, [1, 2, 3]);
    engine::revoke(&app.db, alex_id, "the_yapper", Some(2))
        .await
        .unwrap();
    let b = badges_of(&app, &alex, alex_id).await;
    let y = held(&b, "the_yapper").unwrap();
    assert_eq!((y.tier, y.first_tiers.clone()), (1, vec![1]));
    engine::revoke(&app.db, alex_id, "the_yapper", None)
        .await
        .unwrap();
    let b = badges_of(&app, &alex, alex_id).await;
    assert!(held(&b, "the_yapper").is_none());
    let firsts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM badge_firsts WHERE badge_key = 'the_yapper'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(firsts, 0, "revoked tiers are open to be first again");

    assert!(engine::grant(&app.db, alex_id, "nope", 1).await.is_err());
    assert!(engine::grant(&app.db, alex_id, "founder", 2).await.is_err());
    assert!(
        engine::grant(&app.db, alex_id, "the_yapper", 0)
            .await
            .is_err()
    );
}
