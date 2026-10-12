//! The live server's voice is `wss://`: the LiveKit client must be built with a TLS backend.

use livekit::{Room, RoomOptions};

#[tokio::test]
async fn voice_can_dial_wss() {
    // A listener that accepts and hangs up: TCP succeeds, so the client gets as far as TLS.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            drop(sock);
        }
    });
    let err = Room::connect(
        &format!("wss://127.0.0.1:{port}"),
        "not-a-token",
        RoomOptions::default(),
    )
    .await
    .expect_err("nothing speaks TLS there");
    let msg = err.to_string();
    assert!(!msg.contains("TLS support not compiled in"), "{msg}");
}
