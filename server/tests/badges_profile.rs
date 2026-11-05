//! Badges (#49): the catalogue, the showcase, [NEW] state and hidden badges masked per viewer.
mod common;

use common::*;
use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{
    BadgeCatalogue, BadgeDef, BadgeProgressPart, BadgeRarity, BadgeUnit, Profile,
};
use pulse_server::badges::{catalogue::stat, engine, stats};

/// Everything that would give a hidden badge away, as it could appear in a payload.
const LEAKS: &[&str] = &[
    "friendly_fire",
    "Friendly Fire",
    "Quoted yourself",
    "skill_issue",
    "Skill Issue",
    "spectacularly embarrassing",
    "logo_egg",
    "Egg Hunter",
    "highlight_reel",
    "Highlight Reel",
    "good at games",
];

async fn raw(app: &TestApp, token: &str, path: &str) -> String {
    let r = app
        .http
        .get(app.url(path))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "GET {path}");
    r.text().await.unwrap()
}

fn assert_no_leaks(body: &str) {
    for leak in LEAKS {
        assert!(!body.contains(leak), "{leak:?} leaked: {body}");
    }
}

async fn profile(app: &TestApp, token: &str, user: UserId) -> (String, Profile) {
    let body = raw(app, token, &format!("/users/{user}/profile")).await;
    let p = serde_json::from_str(&body).unwrap();
    (body, p)
}

async fn catalogue(app: &TestApp, token: &str) -> (String, BadgeCatalogue) {
    let body = raw(app, token, "/badges").await;
    let c = serde_json::from_str(&body).unwrap();
    (body, c)
}

fn def<'a>(c: &'a BadgeCatalogue, key: &str) -> &'a BadgeDef {
    c.badges.iter().find(|d| d.key == key).unwrap()
}

async fn put_showcase(app: &TestApp, token: &str, keys: &[&str]) -> reqwest::StatusCode {
    app.http
        .put(app.url("/me/badges/showcase"))
        .bearer_auth(token)
        .json(&serde_json::json!({ "keys": keys }))
        .send()
        .await
        .unwrap()
        .status()
}

async fn grant(app: &TestApp, user: UserId, key: &str, tier: u8) {
    engine::grant(&app.db, user, key, tier).await.unwrap();
}

#[tokio::test]
async fn hidden_badges_never_leak_in_the_catalogue() {
    let app = spawn().await;
    let (alex_id, _) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    // Someone holding one must not unmask it for anyone else.
    grant(&app, alex_id, "friendly_fire", 1).await;

    let (body, c) = catalogue(&app, &sam).await;
    assert_no_leaks(&body);
    let masked: Vec<_> = c.badges.iter().filter(|d| d.name == "???").collect();
    assert!(masked.len() >= 4, "{masked:?}");
    for d in &masked {
        assert!(d.hidden && d.key.starts_with("hidden:"), "{d:?}");
        assert!(d.description.is_empty() && d.progress.is_none() && d.parts.is_empty());
        assert!(!d.tiers.is_empty() && d.tiers.iter().all(|t| t.threshold == 0));
    }
    // Rarity and share stay visible: Friendly Fire is the first hidden badge.
    let ff = def(&c, "hidden:0");
    assert_eq!(ff.tiers[0].rarity, BadgeRarity::Uncommon);
    assert_eq!(ff.tiers[0].holders, 1);
}

#[tokio::test]
async fn hidden_badges_never_leak_through_someone_elses_profile() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    grant(&app, alex_id, "friendly_fire", 1).await;
    grant(&app, alex_id, "night_owl", 1).await;
    assert_eq!(put_showcase(&app, &alex, &["friendly_fire"]).await, 204);

    let (body, p) = profile(&app, &sam, alex_id).await;
    assert_no_leaks(&body);
    let hidden: Vec<_> = p.badges.iter().filter(|b| b.hidden).collect();
    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0].key, "hidden:0");
    assert_eq!(hidden[0].rarity, BadgeRarity::Uncommon);
    assert_eq!(hidden[0].count, 0);
    // alex was first to it, and when: neither shows through the mask.
    assert_eq!(hidden[0].earned_at, "");
    assert_eq!(hidden[0].tier, 1);
    assert!(!hidden[0].first_earner);
    assert!(hidden[0].first_tiers.is_empty());
    assert!(p.badges.iter().any(|b| b.key == "night_owl"));
    assert_eq!(p.showcase, ["hidden:0"]);

    // The owner always sees their own.
    let (_, own) = profile(&app, &alex, alex_id).await;
    assert!(
        own.badges
            .iter()
            .any(|b| b.key == "friendly_fire" && !b.hidden)
    );
    assert_eq!(own.showcase, ["friendly_fire"]);
    let (own_cat, _) = catalogue(&app, &alex).await;
    assert!(own_cat.contains("Friendly Fire"));

    // Once sam earns it too, sam sees alex's unmasked.
    grant(&app, sam_id, "friendly_fire", 1).await;
    let (body, p) = profile(&app, &sam, alex_id).await;
    assert!(body.contains("friendly_fire"));
    assert!(p.badges.iter().all(|b| !b.hidden));
    assert_eq!(p.showcase, ["friendly_fire"]);
}

#[tokio::test]
async fn catalogue_share_and_progress() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    register(&app, "kim").await;
    {
        let mut c = app.db.acquire().await.unwrap();
        stats::add(&mut c, alex_id, stat::MESSAGES, 150)
            .await
            .unwrap();
        stats::add(&mut c, sam_id, stat::VOICE_SECS, 3 * 3600 + 1800)
            .await
            .unwrap();
    }
    engine::check(&app.state(), alex_id).await.unwrap();

    let (_, c) = catalogue(&app, &alex).await;
    assert_eq!(c.people, 3);
    let yapper = def(&c, "the_yapper");
    assert_eq!(yapper.tiers[0].threshold, 100);
    assert_eq!(yapper.tiers[0].holders, 1);
    assert_eq!(yapper.tiers[1].holders, 0);
    assert_eq!(yapper.progress, Some(150));
    assert_eq!(yapper.unit, BadgeUnit::Count);

    let regular = def(&c, "the_regular");
    assert_eq!(regular.unit, BadgeUnit::Hours);
    assert_eq!(regular.tiers[0].threshold, 10);
    assert_eq!(regular.tiers[5].threshold, 1000);
    assert_eq!(regular.progress, Some(0));
    // Progress is the viewer's own, in hours.
    let (_, c) = catalogue(&app, &sam).await;
    assert_eq!(def(&c, "the_regular").progress, Some(3));
    assert_eq!(def(&c, "the_yapper").progress, Some(0));
    // Combo's threshold is its message count.
    let combo = def(&c, "combo");
    assert_eq!(
        combo.tiers.iter().map(|t| t.threshold).collect::<Vec<_>>(),
        [100, 500, 1000]
    );
    assert_eq!(combo.progress, None);
    // ...its progress is per stat, toward the next tier (#191), the viewer's own, hours whole.
    let part = |label: &str, value: u64, target: u64| BadgeProgressPart {
        label: label.into(),
        value,
        target,
    };
    assert_eq!(
        combo.parts,
        [
            part("Messages", 0, 100),
            part("Hours in voice", 3, 100),
            part("Clips", 0, 10)
        ]
    );
    let (_, c) = catalogue(&app, &alex).await;
    assert_eq!(def(&c, "combo").parts[0], part("Messages", 150, 100));
    assert!(def(&c, "the_yapper").parts.is_empty());
}

#[tokio::test]
async fn showcase_order_persists_and_defaults_to_rarest() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    // One of each rarity, plus a second Common.
    for (key, tier) in [
        ("the_yapper", 1),        // Common
        ("identity_crisis", 1),   // Common
        ("early_bird", 1),        // Uncommon
        ("night_owl", 1),         // Rare
        ("the_doos", 1),          // Epic
        ("quote_of_the_year", 1), // Legendary
    ] {
        grant(&app, alex_id, key, tier).await;
    }
    grant(&app, alex_id, "founder", 1).await; // Mythical
    let (_, p) = profile(&app, &sam, alex_id).await;
    assert_eq!(
        p.showcase,
        [
            "founder",
            "quote_of_the_year",
            "the_doos",
            "night_owl",
            "early_bird"
        ]
    );
    assert!(!p.showcase_chosen, "the default isn't a choice");

    assert_eq!(
        put_showcase(&app, &alex, &["the_yapper", "founder"]).await,
        204
    );
    for token in [&alex, &sam] {
        let (_, p) = profile(&app, token, alex_id).await;
        assert_eq!(p.showcase, ["the_yapper", "founder"]);
        assert!(p.showcase_chosen);
    }

    assert_eq!(put_showcase(&app, &alex, &["ghost"]).await, 400);
    assert_eq!(put_showcase(&app, &alex, &["nope"]).await, 400);
    assert_eq!(
        put_showcase(&app, &alex, &["founder", "founder"]).await,
        400
    );
    let six = [
        "the_yapper",
        "identity_crisis",
        "early_bird",
        "night_owl",
        "the_doos",
        "founder",
    ];
    assert_eq!(put_showcase(&app, &alex, &six).await, 400);
    // A refused PUT changes nothing.
    let (_, p) = profile(&app, &alex, alex_id).await;
    assert_eq!(p.showcase, ["the_yapper", "founder"]);

    // Empty clears the choice: back to the default.
    assert_eq!(put_showcase(&app, &alex, &[]).await, 204);
    let (_, p) = profile(&app, &alex, alex_id).await;
    assert_eq!(p.showcase[0], "founder");
    assert_eq!(p.showcase.len(), 5);
    assert!(!p.showcase_chosen);
}

#[tokio::test]
async fn seen_marks_all() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    grant(&app, alex_id, "night_owl", 1).await;
    grant(&app, alex_id, "ghost", 1).await;

    let (_, p) = profile(&app, &alex, alex_id).await;
    assert!(p.badges.iter().all(|b| !b.seen));
    let (_, p) = profile(&app, &sam, alex_id).await;
    assert!(p.badges.iter().all(|b| b.seen), "sam sees alex's [NEW]");

    // An older client posts no body: everything is seen, as before (#129).
    let r = app
        .http
        .post(app.url("/me/badges/seen"))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let (_, p) = profile(&app, &alex, alex_id).await;
    assert_eq!(p.badges.len(), 2);
    assert!(p.badges.iter().all(|b| b.seen));
}

/// #129: the case says which badges (at which tier) it showed; one earned after it loaded, or a
/// tier reached since, stays [NEW].
#[tokio::test]
async fn seen_marks_only_what_the_case_showed() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    grant(&app, alex_id, "night_owl", 1).await;
    grant(&app, alex_id, "the_yapper", 1).await;
    let (_, shown) = profile(&app, &alex, alex_id).await;

    // While the case is open: a new badge, and a tier up of one it shows.
    grant(&app, alex_id, "ghost", 1).await;
    grant(&app, alex_id, "the_yapper", 2).await;

    let badges: Vec<_> = shown
        .badges
        .iter()
        .map(|b| serde_json::json!({ "key": b.key, "tier": b.tier }))
        .collect();
    let r = post_json(
        &app,
        &alex,
        "/me/badges/seen",
        serde_json::json!({ "badges": badges }),
    )
    .await;
    assert_eq!(r.status(), 204);

    let (_, p) = profile(&app, &alex, alex_id).await;
    let seen = |key: &str| p.badges.iter().find(|b| b.key == key).unwrap().seen;
    assert!(seen("night_owl"));
    assert!(!seen("ghost"), "earned after the case loaded");
    assert!(!seen("the_yapper"), "tier II was never shown");

    // Only your own badges: naming one you don't hold changes nothing and isn't an error.
    let r = post_json(
        &app,
        &alex,
        "/me/badges/seen",
        serde_json::json!({ "badges": [{ "key": "founder", "tier": 1 }] }),
    )
    .await;
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn hand_given_defs_are_listed_but_never_hidden() {
    let app = spawn().await;
    let (_, sam) = register(&app, "sam").await;
    let (_, c) = catalogue(&app, &sam).await;
    for key in ["founder", "partner"] {
        let d = def(&c, key);
        assert!(d.hand_given && !d.hidden, "{d:?}");
        assert_ne!(d.name, "???");
        assert_eq!(d.progress, None);
        assert_eq!(d.tiers.len(), 1);
        assert_eq!(d.tiers[0].rarity, BadgeRarity::Mythical);
    }
    // Skill Issue has no trigger yet but isn't hand-given.
    assert!(c.badges.iter().filter(|d| d.hand_given).count() == 2);
}
