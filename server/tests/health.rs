mod common;

#[tokio::test]
async fn health_ok_and_schema_migrated() {
    let app = common::spawn().await;
    let r = app.http.get(app.url("/health")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='messages'",
    )
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn unknown_route_is_json_404() {
    let app = common::spawn().await;
    let r = app.http.get(app.url("/nope")).send().await.unwrap();
    assert_eq!(r.status(), 404);
}
