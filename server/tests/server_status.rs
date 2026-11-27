//! Your status in one server (#154): "My status here". Members of that server see it there (`in_servers` on Ready's
//! people and `PresenceChanged`), everyone sees your main status elsewhere, and only you get your list.
mod common;

use std::time::Duration;

use common::*;
use pulse_protocol::gateway::{Event, ServerFrame};
use pulse_protocol::ids::{ServerId, UserId};
use pulse_protocol::rest::{PresenceStatus, ServerPresence, ServerStatus, Status};
use serde_json::json;

async fn drain(ws: &mut Socket, ms: u64) -> Vec<Event> {
    let mut got = vec![];
    while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(ms), next_frame(ws)).await {
        if let ServerFrame::Event(e) = f {
            got.push(e);
        }
    }
    got
}

async fn my_status(
    app: &TestApp,
    token: &str,
    server: ServerId,
    status: Option<&str>,
) -> reqwest::Response {
    app.http
        .put(app.url(&format!("/servers/{server}/my-status")))
        .bearer_auth(token)
        .json(&json!({ "status": status }))
        .send()
        .await
        .unwrap()
}

async fn set_status(app: &TestApp, token: &str, status: &str) {
    let r = app
        .http
        .put(app.url("/me/status"))
        .bearer_auth(token)
        .json(&json!({ "status": status }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
}

/// The newest `in_servers` told about `who` (None: no PresenceChanged about them).
fn last_in_servers(events: &[Event], who: UserId) -> Option<(PresenceStatus, Vec<ServerPresence>)> {
    events.iter().rev().find_map(|e| match e {
        Event::PresenceChanged {
            user_id,
            status,
            in_servers,
            ..
        } if *user_id == who => Some((*status, in_servers.clone())),
        _ => None,
    })
}

#[tokio::test]
async fn members_see_it_in_that_server_and_the_main_status_elsewhere() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    // sam shares Main and Side with alex; kim only Side
    let main = create_server(&app, &alex, "Main").await.id;
    let side = create_server(&app, &alex, "Side").await.id;
    assert_eq!(join_server(&app, &sam, main).await.status(), 200);
    assert_eq!(join_server(&app, &sam, side).await.status(), 200);
    assert_eq!(join_server(&app, &kim, side).await.status(), 200);
    let (mut alex_ws, _) = hello(&app, &alex).await;
    let (mut sam_ws, _) = hello(&app, &sam).await;
    let (mut kim_ws, _) = hello(&app, &kim).await;
    drain(&mut alex_ws, 300).await;
    drain(&mut sam_ws, 300).await;
    drain(&mut kim_ws, 300).await;

    // Invisible in Main
    let r = my_status(&app, &alex, main, Some("invisible")).await;
    assert_eq!(r.status(), 200);
    let list: Vec<ServerStatus> = r.json().await.unwrap();
    assert_eq!(
        list,
        vec![ServerStatus {
            server_id: main,
            status: Status::Invisible
        }]
    );
    assert!(
        drain(&mut alex_ws, 300)
            .await
            .iter()
            .any(|e| matches!(e, Event::MyServerStatusesChanged { list, .. } if list.len() == 1))
    );
    // sam: online everywhere, offline in Main
    let (status, in_servers) =
        last_in_servers(&drain(&mut sam_ws, 400).await, alex_id).expect("told");
    assert_eq!(status, PresenceStatus::Online);
    assert_eq!(in_servers.len(), 1);
    assert_eq!(in_servers[0].server_id, main);
    assert_eq!(in_servers[0].status, PresenceStatus::Offline);
    assert!(
        in_servers[0].last_seen_at.is_some(),
        "frozen at when it was set"
    );
    // kim isn't in Main: nothing changes for them
    assert!(last_in_servers(&drain(&mut kim_ws, 300).await, alex_id).is_none());
    // a new Ready says the same, and alex's own carries the list
    let (_ws, ready) = hello(&app, &sam).await;
    let a = ready.people.iter().find(|p| p.user.id == alex_id).unwrap();
    assert_eq!(a.status, PresenceStatus::Online);
    assert_eq!(
        a.in_servers
            .iter()
            .map(|p| (p.server_id, p.status))
            .collect::<Vec<_>>(),
        vec![(main, PresenceStatus::Offline)]
    );
    let (_ws, ready) = hello(&app, &kim).await;
    assert!(
        ready
            .people
            .iter()
            .find(|p| p.user.id == alex_id)
            .unwrap()
            .in_servers
            .is_empty()
    );
    let (_ws, ready) = hello(&app, &alex).await;
    assert_eq!(
        ready.server_statuses,
        vec![ServerStatus {
            server_id: main,
            status: Status::Invisible
        }]
    );

    // Do Not Disturb in Side: both see DND there
    assert_eq!(
        my_status(&app, &alex, side, Some("dnd")).await.status(),
        200
    );
    let (_, in_servers) = last_in_servers(&drain(&mut kim_ws, 400).await, alex_id).expect("told");
    assert_eq!(
        in_servers
            .iter()
            .map(|p| (p.server_id, p.status))
            .collect::<Vec<_>>(),
        vec![(side, PresenceStatus::Dnd)]
    );

    // Same as everywhere again
    assert_eq!(my_status(&app, &alex, main, None).await.status(), 200);
    assert_eq!(my_status(&app, &alex, side, None).await.status(), 200);
    let (_, in_servers) = last_in_servers(&drain(&mut sam_ws, 400).await, alex_id).expect("told");
    assert!(in_servers.is_empty());
}

#[tokio::test]
async fn online_in_a_server_shows_there_while_invisible_elsewhere() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let main = create_server(&app, &alex, "Main").await.id;
    assert_eq!(join_server(&app, &sam, main).await.status(), 200);
    let (_alex_ws, _) = hello(&app, &alex).await;
    set_status(&app, &alex, "invisible").await;
    assert_eq!(
        my_status(&app, &alex, main, Some("online")).await.status(),
        200
    );
    let (_ws, ready) = hello(&app, &sam).await;
    let a = ready.people.iter().find(|p| p.user.id == alex_id).unwrap();
    assert_eq!(a.status, PresenceStatus::Offline);
    assert_eq!(
        a.in_servers
            .iter()
            .map(|p| (p.server_id, p.status))
            .collect::<Vec<_>>(),
        vec![(main, PresenceStatus::Online)]
    );
}

#[tokio::test]
async fn only_in_a_server_you_are_in() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, kim) = register(&app, "kim").await;
    let main = create_server(&app, &alex, "Main").await.id;
    assert_eq!(my_status(&app, &kim, main, Some("dnd")).await.status(), 404);
    assert_eq!(
        my_status(&app, &kim, ServerId::new(), Some("dnd"))
            .await
            .status(),
        404
    );
    let r = app
        .http
        .put(app.url(&format!("/servers/{main}/my-status")))
        .bearer_auth(&alex)
        .json(&json!({ "status": "away" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
}
