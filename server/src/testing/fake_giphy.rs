//! A stand-in for GIPHY's API (#58) on 127.0.0.1, so no test calls the real one. The app's core searches GIPHY
//! itself (the server only hands out the key), so the core's tests point it here.
//!
//! - `GET /v1/gifs/search?api_key&q&limit&offset&rating` and `GET /v1/gifs/trending?api_key&limit&offset&rating`
//!   answer a page shaped like GIPHY's (sizes as strings, as GIPHY sends them): two GIFs, `Fake<n>a` and `Fake<n>b`
//!   for offset `n`, titled after the query (or "Trending"), out of [`TOTAL`].
//! - [`FakeGiphy::set_status`] makes every answer that status instead (429: over the key's limit).
//! - An `api_key` other than [`KEY`] gets a 401, as GIPHY answers a bad key.

use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};

/// The key the fake accepts.
pub const KEY: &str = "fake-giphy-key";
/// How many results every search and trending claim to have.
pub const TOTAL: u32 = 30;

#[derive(Clone)]
struct Shared {
    log: Arc<Mutex<Vec<String>>>,
    status: Arc<AtomicU16>,
}

pub struct FakeGiphy {
    pub port: u16,
    log: Arc<Mutex<Vec<String>>>,
    status: Arc<AtomicU16>,
}

impl FakeGiphy {
    pub async fn start() -> FakeGiphy {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let shared = Shared {
            log: Arc::new(Mutex::new(Vec::new())),
            status: Arc::new(AtomicU16::new(200)),
        };
        let app = Router::new()
            .route("/v1/gifs/{kind}", get(page))
            .with_state(shared.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeGiphy {
            port,
            log: shared.log,
            status: shared.status,
        }
    }

    /// What the app's GIPHY client is pointed at instead of `https://api.giphy.com/v1/gifs`.
    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}/v1/gifs", self.port)
    }

    /// Every request's path and query, in order.
    pub fn requests(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    /// Answer every request with `status` (200 to answer normally again).
    pub fn set_status(&self, status: u16) {
        self.status.store(status, Ordering::SeqCst);
    }
}

fn rendition(id: &str, file: &str, w: u32, h: u32) -> Value {
    json!({
        "width": w.to_string(),
        "height": h.to_string(),
        "url": format!("https://media1.giphy.com/media/v1.fake/{id}/{file}.gif?cid=fake&rid={file}.gif&ct=g"),
        "webp": format!("https://media1.giphy.com/media/v1.fake/{id}/{file}.webp?cid=fake&rid={file}.webp&ct=g"),
    })
}

fn gif(id: &str, title: &str) -> Value {
    json!({
        "type": "gif",
        "id": id,
        "url": format!("https://giphy.com/gifs/fake-{id}"),
        "title": title,
        "rating": "g",
        "images": {
            "original": rendition(id, "giphy", 480, 270),
            "fixed_width": rendition(id, "200w", 200, 113),
            "fixed_width_still": { "width": "200", "height": "113", "url": format!("https://media1.giphy.com/media/v1.fake/{id}/200w_s.gif?cid=fake&rid=200w_s.gif&ct=g") },
        },
    })
}

async fn page(
    State(s): State<Shared>,
    Path(kind): Path<String>,
    Query(q): Query<std::collections::HashMap<String, String>>,
    uri: Uri,
) -> Response {
    s.log.lock().unwrap().push(uri.to_string());
    let status = s.status.load(Ordering::SeqCst);
    if status != 200 {
        let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        return (
            code,
            axum::Json(json!({ "meta": { "status": status, "msg": "fake" } })),
        )
            .into_response();
    }
    if q.get("api_key").map(String::as_str) != Some(KEY) {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({ "message": "Invalid authentication credentials" })),
        )
            .into_response();
    }
    let title = match kind.as_str() {
        "search" => q.get("q").cloned().unwrap_or_default(),
        "trending" => "Trending".to_string(),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let offset: u32 = q.get("offset").and_then(|o| o.parse().ok()).unwrap_or(0);
    let data = vec![
        gif(&format!("Fake{offset}a"), &format!("{title} GIF")),
        gif(&format!("Fake{offset}b"), &format!("{title} GIF too")),
    ];
    axum::Json(json!({
        "data": data,
        "pagination": { "total_count": TOTAL, "count": 2, "offset": offset },
        "meta": { "status": 200, "msg": "OK", "response_id": "fake" },
    }))
    .into_response()
}
