//! Badge art (#225): a picture for a badge, stored only on the server (never in the repo) by the
//! `badge art` admin command, carried to clients as a media hash on the badge's definition.
mod common;

use std::io::Cursor;
use std::time::Duration;

use common::*;
use pulse_protocol::rest::BadgeCatalogue;
use pulse_server::badges::art;
use pulse_server::media;

/// A `w`×`h` PNG: wide pictures must keep their shape (the client fits them whole).
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([40, 200, 120, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

async fn catalogue(app: &TestApp, token: &str) -> BadgeCatalogue {
    let r = app
        .http
        .get(app.url("/badges"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

fn art_of(c: &BadgeCatalogue, key: &str) -> Option<String> {
    c.badges.iter().find(|d| d.key == key).unwrap().art.clone()
}

#[tokio::test]
async fn art_round_trips_to_the_catalogue_and_the_media_route() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let dir = &app.cfg.media_dir;

    // Nothing has art until it's given some.
    let c = catalogue(&app, &alex).await;
    assert!(c.badges.iter().all(|d| d.art.is_none()));

    let hash = art::set(&app.db, dir, "patient_zero", &png(600, 200))
        .await
        .unwrap();
    assert!(media::valid_hash(&hash));
    assert_eq!(
        art_of(&catalogue(&app, &alex).await, "patient_zero").as_deref(),
        Some(hash.as_str())
    );
    // Other badges are unchanged.
    assert_eq!(art_of(&catalogue(&app, &alex).await, "founder"), None);

    // Fitted within 256 px on its long side, its shape kept, and served like any other media.
    let (kind, w, h): (String, i64, i64) =
        sqlx::query_as("SELECT kind, width, height FROM media WHERE hash = ?")
            .bind(&hash)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!((kind.as_str(), w, h), ("badge_art", 256, 85));
    let r = app
        .http
        .get(app.url(&format!("/media/{hash}")))
        .bearer_auth(&alex)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "image/webp");

    // Replacing it points the badge at the new picture.
    let again = art::set(&app.db, dir, "patient_zero", &png(100, 100))
        .await
        .unwrap();
    assert_ne!(again, hash);
    assert_eq!(
        art_of(&catalogue(&app, &alex).await, "patient_zero").as_deref(),
        Some(again.as_str())
    );

    // Cleared, it shows its emoji again.
    assert!(art::clear(&app.db, "patient_zero").await.unwrap());
    assert!(!art::clear(&app.db, "patient_zero").await.unwrap());
    assert_eq!(art_of(&catalogue(&app, &alex).await, "patient_zero"), None);
}

#[tokio::test]
async fn unknown_keys_and_bad_images_are_refused() {
    let app = spawn().await;
    let dir = &app.cfg.media_dir;
    let e = art::set(&app.db, dir, "no_such_badge", &png(10, 10))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("unknown badge no_such_badge"), "{e}");
    let e = art::set(&app.db, dir, "founder", b"not a picture")
        .await
        .unwrap_err();
    assert!(e.to_string().contains("supported image"), "{e}");
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM badge_art")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn cleanup_never_deletes_art_in_use() {
    let app = spawn().await;
    let dir = &app.cfg.media_dir;
    let hash = art::set(&app.db, dir, "founder", &png(64, 64))
        .await
        .unwrap();
    let file = media::path_for(dir, &hash).unwrap();

    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        0
    );
    assert!(file.exists());

    // Replaced: the old picture is no longer in use, so it goes.
    art::set(&app.db, dir, "founder", &png(32, 32))
        .await
        .unwrap();
    assert_eq!(
        media::cleanup(&app.db, dir, Duration::ZERO).await.unwrap(),
        1
    );
    assert!(!file.exists());
}

#[tokio::test]
async fn a_masked_hidden_badge_never_carries_art() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    art::set(&app.db, &app.cfg.media_dir, "skill_issue", &png(64, 64))
        .await
        .unwrap();
    let c = catalogue(&app, &alex).await;
    for d in c.badges.iter().filter(|d| d.key.starts_with("hidden:")) {
        assert_eq!(d.art, None, "{}", d.key);
    }
}

// --- the `badge art` command, against the real binary ---

fn badge(p: &ServerProcess, media_dir: &std::path::Path, args: &[&str]) -> (bool, String) {
    let args: Vec<&str> = ["badge"].iter().chain(args).copied().collect();
    let dir = media_dir.to_str().unwrap();
    run_binary(
        &p.db_url,
        &[("PULSE_MEDIA_DIR", dir)],
        &args,
        Duration::from_secs(20),
    )
}

#[tokio::test]
async fn the_command_stores_art_and_refuses_unknown_keys_and_bad_files() {
    let p = spawn_binary().await;
    let tmp = tempfile::tempdir().unwrap();
    let media_dir = tmp.path().join("media");
    let good = tmp.path().join("art.png");
    std::fs::write(&good, png(300, 150)).unwrap();
    let bad = tmp.path().join("art.txt");
    std::fs::write(&bad, "not a picture").unwrap();
    let good = good.to_str().unwrap();

    let (ok, out) = badge(&p, &media_dir, &["art", "no_such_badge", good]);
    assert!(!ok, "{out}");
    assert!(out.contains("unknown badge no_such_badge"), "{out}");

    let (ok, out) = badge(&p, &media_dir, &["art", "partner", bad.to_str().unwrap()]);
    assert!(!ok, "{out}");
    assert!(out.contains("supported image"), "{out}");

    let missing = tmp.path().join("missing.png");
    let (ok, out) = badge(
        &p,
        &media_dir,
        &["art", "partner", missing.to_str().unwrap()],
    );
    assert!(!ok, "{out}");

    let (ok, out) = badge(&p, &media_dir, &["art", "partner", good]);
    assert!(ok, "{out}");
    assert!(out.contains("art for partner set"), "{out}");
    let db = pulse_server::db::connect(&p.db_url).await.unwrap();
    let hash: String = sqlx::query_scalar("SELECT media_hash FROM badge_art WHERE badge_key = ?")
        .bind("partner")
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(media::path_for(&media_dir, &hash).unwrap().exists());

    let (ok, out) = badge(&p, &media_dir, &["clear-art", "partner"]);
    assert!(ok, "{out}");
    assert!(out.contains("art for partner cleared"), "{out}");
    let (ok, out) = badge(&p, &media_dir, &["clear-art", "nope"]);
    assert!(!ok, "{out}");
}
