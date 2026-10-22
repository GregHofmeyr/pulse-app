use livekit_api::access_token::TokenVerifier;
use pulse_server::testing;
use serde_json::json;

async fn lounge(app: &testing::TestApp, token: &str) -> String {
    let s = testing::create_server(app, token, "Main").await;
    let chans: Vec<pulse_protocol::rest::Channel> =
        testing::get_json(app, token, &format!("/servers/{}/channels", s.id)).await;
    chans
        .into_iter()
        .find(|c| c.name.as_deref() == Some("Lounge"))
        .unwrap()
        .id
        .to_string()
}

#[tokio::test]
async fn spike_tickets_are_404_when_disabled() {
    let app = testing::spawn().await; // spike: false
    let (_, tok) = testing::register(&app, "alex").await;
    let ch = lounge(&app, &tok).await;
    let res = testing::post_json(
        &app,
        &tok,
        &format!("/spike/stream/{ch}/ticket"),
        json!({"role":"v"}),
    )
    .await;
    assert_eq!(res.status(), 404);
}

#[tokio::test]
async fn spike_tickets_only_name_the_stream_room() {
    let mut cfg = testing::test_config();
    cfg.spike = true;
    let app = testing::spawn_with(cfg.clone()).await;
    let (uid, tok) = testing::register(&app, "alex").await;
    let ch = lounge(&app, &tok).await;
    app.voice.join(ch.parse().unwrap(), uid, "PA_test"); // what the LiveKit join webhook does
    for (role, sources) in [
        ("v", vec!["screen_share"]),
        ("a", vec!["screen_share_audio"]),
        ("v2", vec![]),
    ] {
        let res = testing::post_json(
            &app,
            &tok,
            &format!("/spike/stream/{ch}/ticket"),
            json!({"role":role}),
        )
        .await;
        assert_eq!(res.status(), 200, "{role}");
        let body: pulse_protocol::rest::VoiceTokenResponse = res.json().await.unwrap();
        let claims = TokenVerifier::with_api_key(&cfg.livekit_key, &cfg.livekit_secret)
            .verify(&body.token)
            .unwrap();
        assert_eq!(claims.video.room, format!("{ch}~s"), "never the voice room");
        assert_eq!(claims.sub, format!("{uid}#{role}"));
        assert_eq!(claims.video.can_publish_sources, sources, "{role}");
    }
    // the LiveKit webhook ignores the stream room: its name isn't a ChannelId
    assert!(
        format!("{ch}~s")
            .parse::<pulse_protocol::ids::ChannelId>()
            .is_err()
    );
}

#[tokio::test]
async fn spike_tickets_need_you_in_voice() {
    let mut cfg = testing::test_config();
    cfg.spike = true;
    let app = testing::spawn_with(cfg).await;
    let (_, tok) = testing::register(&app, "alex").await;
    let ch = lounge(&app, &tok).await;
    let res = testing::post_json(
        &app,
        &tok,
        &format!("/spike/stream/{ch}/ticket"),
        json!({"role":"v"}),
    )
    .await;
    assert_eq!(res.status(), 403);
}
