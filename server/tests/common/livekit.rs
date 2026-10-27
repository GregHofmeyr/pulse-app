//! A fake LiveKit RoomService for tests, speaking LiveKit's Twirp protocol: POST
//! /twirp/livekit.RoomService/<Method>, protobuf bodies.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use livekit_protocol as lkp;
use prost::Message as _;

pub type Rooms = Vec<(String, Vec<lkp::ParticipantInfo>)>;

/// A fake RoomService on 127.0.0.1:<free port>; returns its base URL.
pub async fn fake_livekit(rooms: Rooms) -> String {
    fake_livekit_logged(rooms).await.0
}

/// [`fake_livekit`], also returning the log of participants it was asked to remove.
pub async fn fake_livekit_logged(rooms: Rooms) -> (String, Removed) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let removed = serve_fake(listener, rooms);
    (format!("http://{addr}"), removed)
}

/// A RoomService that accepts the connection and never answers (overloaded or mid-restart).
pub async fn fake_livekit_hanging() -> String {
    let hang = || async {
        tokio::time::sleep(Duration::from_secs(60)).await;
        Vec::<u8>::new()
    };
    let mut app = axum::Router::new();
    for method in [
        "ListRooms",
        "ListParticipants",
        "GetParticipant",
        "RemoveParticipant",
    ] {
        app = app.route(&format!("/twirp/livekit.RoomService/{method}"), post(hang));
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// LiveKit removals the fake saw, as (room, identity).
pub type Removed = Arc<Mutex<Vec<(String, String)>>>;

/// Serve the fake RoomService on an already-bound listener (lets a test bring LiveKit up late).
/// Also answers GetParticipant / RemoveParticipant (restore removes participants whose grant
/// isn't a live session's); returns the removal log.
pub fn serve_fake(listener: tokio::net::TcpListener, rooms: Rooms) -> Removed {
    serve_fake_shared(listener, Arc::new(Mutex::new(rooms)))
}

/// Rooms a test can change while the fake serves them.
pub type SharedRooms = Arc<Mutex<Rooms>>;

/// A fake RoomService serving `rooms` as they are at each request; returns its base URL.
pub async fn fake_livekit_shared(rooms: SharedRooms) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    serve_fake_shared(listener, rooms);
    format!("http://{addr}")
}

/// [`serve_fake`] on rooms the caller keeps a handle to.
pub fn serve_fake_shared(listener: tokio::net::TcpListener, rooms: SharedRooms) -> Removed {
    let removed = Removed::default();
    let (r1, r2, r3, r4, log) = (
        rooms.clone(),
        rooms.clone(),
        rooms.clone(),
        rooms,
        removed.clone(),
    );
    let app = axum::Router::new()
        .route(
            "/twirp/livekit.RoomService/ListRooms",
            post(move || {
                let rooms = r1.clone();
                async move {
                    lkp::ListRoomsResponse {
                        rooms: rooms
                            .lock()
                            .unwrap()
                            .iter()
                            .map(|(name, _)| lkp::Room {
                                name: name.clone(),
                                ..Default::default()
                            })
                            .collect(),
                    }
                    .encode_to_vec()
                }
            }),
        )
        .route(
            "/twirp/livekit.RoomService/ListParticipants",
            post(move |body: Bytes| {
                let rooms = r2.clone();
                async move {
                    let req = lkp::ListParticipantsRequest::decode(body).unwrap();
                    // as LiveKit v1.13.10: a room it doesn't have lists nobody (no error)
                    let participants = rooms
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|(name, _)| *name == req.room)
                        .map(|(_, p)| p.clone())
                        .unwrap_or_default();
                    lkp::ListParticipantsResponse { participants }.encode_to_vec()
                }
            }),
        )
        .route(
            "/twirp/livekit.RoomService/GetParticipant",
            post(move |body: Bytes| {
                let rooms = r3.clone();
                async move {
                    let req = lkp::RoomParticipantIdentity::decode(body).unwrap();
                    let found = rooms
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|(name, _)| *name == req.room)
                        .flat_map(|(_, ps)| ps.iter())
                        .find(|p| p.identity == req.identity)
                        .cloned();
                    match found {
                        Some(p) => p.encode_to_vec().into_response(),
                        None => twirp_not_found(),
                    }
                }
            }),
        )
        .route(
            "/twirp/livekit.RoomService/RemoveParticipant",
            post(move |body: Bytes| {
                let (rooms, log) = (r4.clone(), log.clone());
                async move {
                    let req = lkp::RoomParticipantIdentity::decode(body).unwrap();
                    let mut hit = false;
                    for (name, ps) in rooms.lock().unwrap().iter_mut() {
                        if *name == req.room {
                            let before = ps.len();
                            ps.retain(|p| p.identity != req.identity);
                            hit |= ps.len() != before;
                        }
                    }
                    if !hit {
                        return twirp_not_found();
                    }
                    log.lock().unwrap().push((req.room, req.identity));
                    lkp::RemoveParticipantResponse {}
                        .encode_to_vec()
                        .into_response()
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    removed
}

pub fn twirp_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"code":"not_found","msg":"participant not found"}"#,
    )
        .into_response()
}
