//! Per-person Do Not Disturb (#310): your list follows you to every device (Ready and `PersonDndChanged`), never
//! anyone else's; a timed one past its end reads as gone; only real people, never yourself.
mod common;

use std::time::Duration;

use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, Event, ServerFrame};
use pulse_protocol::rest::PersonDnd;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as Ws;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> (Socket, Vec<PersonDnd>) {
    let mut ws = tokio_tungstenite::connect_async(app.ws_url("/gateway"))
        .await
        .unwrap()
        .0;
    let f = serde_json::to_string(&ClientFrame::Hello {
        token: token.into(),
        client_version: pulse_protocol::PROTOCOL_VERSION,
    })
    .unwrap();
    ws.send(Ws::text(f)).await.unwrap();
    loop {
        if let Some(ServerFrame::Ready(r)) = next_frame(&mut ws).await {
            return (ws, r.person_dnd);
        }
    }
}

async fn next_frame(ws: &mut Socket) -> Option<ServerFrame> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        match m {
            Ws::Text(t) => return Some(serde_json::from_str(&t).unwrap()),
            Ws::Close(_) => return None,
            _ => continue,
        }
    }
}

async fn next_list(ws: &mut Socket) -> Option<Vec<PersonDnd>> {
    loop {
        if let ServerFrame::Event(Event::PersonDndChanged { list, .. }) = next_frame(ws).await? {
            return Some(list);
        }
    }
}

async fn call(
    app: &TestApp,
    method: reqwest::Method,
    token: &str,
    path: &str,
    body: Value,
) -> reqwest::Response {
    app.http
        .request(method, app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn the_list_follows_you_to_every_device_and_no_one_else() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (sam_id, sam) = register(&app, "sam").await;
    let (kim_id, _) = register(&app, "kim").await;
    let (mut other_device, empty) = hello(&app, &alex).await;
    assert!(empty.is_empty());
    let (mut sams, _) = hello(&app, &sam).await;

    let r = call(
        &app,
        reqwest::Method::PUT,
        &alex,
        &format!("/me/person-dnd/{sam_id}"),
        json!({}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let list: Vec<PersonDnd> = r.json().await.unwrap();
    assert_eq!(
        list,
        vec![PersonDnd {
            user_id: sam_id,
            dms_only: false,
            until: None
        }]
    );
    assert_eq!(next_list(&mut other_device).await.unwrap(), list);

    // kim, only in DMs, for an hour
    let until =
        pulse_server::times::rfc3339(jiff::Timestamp::now() + jiff::SignedDuration::from_hours(1));
    let r = call(
        &app,
        reqwest::Method::PUT,
        &alex,
        &format!("/me/person-dnd/{kim_id}"),
        json!({ "dms_only": true, "until": until }),
    )
    .await;
    let list: Vec<PersonDnd> = r.json().await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[1].user_id, kim_id);
    assert!(list[1].dms_only);
    assert!(list[1].until.is_some());

    // a fresh connection gets it in Ready; sam is never told
    let (_, ready) = hello(&app, &alex).await;
    assert_eq!(ready, list);
    loop {
        match next_frame(&mut sams).await {
            None => break,
            Some(ServerFrame::Event(Event::PersonDndChanged { .. })) => {
                panic!("sam was told alex's list")
            }
            Some(_) => {}
        }
    }

    // off again
    let r = call(
        &app,
        reqwest::Method::DELETE,
        &alex,
        &format!("/me/person-dnd/{sam_id}"),
        json!({}),
    )
    .await;
    let list: Vec<PersonDnd> = r.json().await.unwrap();
    assert_eq!(
        list.iter().map(|p| p.user_id).collect::<Vec<_>>(),
        vec![kim_id]
    );
}

#[tokio::test]
async fn run_out_ones_read_as_gone_and_bad_asks_are_refused() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, _) = register(&app, "sam").await;
    let put = |path: String, body: Value| {
        let app = &app;
        let alex = alex.clone();
        async move { call(app, reqwest::Method::PUT, &alex, &path, body).await }
    };
    // yourself, nobody, a time gone by, not a time
    assert_eq!(
        put(format!("/me/person-dnd/{alex_id}"), json!({}))
            .await
            .status(),
        400
    );
    assert_eq!(
        put(
            format!("/me/person-dnd/{}", pulse_protocol::ids::UserId::new()),
            json!({})
        )
        .await
        .status(),
        404
    );
    assert_eq!(
        put("/me/person-dnd/nope".into(), json!({})).await.status(),
        404
    );
    assert_eq!(
        put(
            format!("/me/person-dnd/{sam_id}"),
            json!({ "until": "2001-01-01T00:00:00Z" })
        )
        .await
        .status(),
        400
    );
    assert_eq!(
        put(
            format!("/me/person-dnd/{sam_id}"),
            json!({ "until": "soon" })
        )
        .await
        .status(),
        400
    );
    // one that ran out (as if its time passed) isn't in the list
    let r = put(
        format!("/me/person-dnd/{sam_id}"),
        json!({ "until": pulse_server::times::rfc3339(jiff::Timestamp::now() + jiff::SignedDuration::from_mins(5)) }),
    )
    .await;
    assert_eq!(r.status(), 200);
    sqlx::query("UPDATE person_dnd SET until = '2001-01-01T00:00:00.000Z'")
        .execute(&app.db)
        .await
        .unwrap();
    let (_, ready) = hello(&app, &alex).await;
    assert!(ready.is_empty());
}
