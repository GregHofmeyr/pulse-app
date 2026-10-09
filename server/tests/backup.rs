//! Online backups: a consistent, readable copy of the live database.
mod common;

use common::*;

#[tokio::test]
async fn backup_is_a_readable_copy_and_never_overwrites() {
    let app = spawn().await;
    register(&app, "alex").await;
    register(&app, "sam").await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("snap.db");
    pulse_server::db::backup(&app.db, &path).await.unwrap();

    let copy = pulse_server::db::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&copy)
        .await
        .unwrap();
    assert_eq!(users, 2);
    assert!(
        pulse_server::db::backup(&app.db, &path).await.is_err(),
        "an existing file is never overwritten"
    );
}
