//! F1 race weekends (#386): the owner's F1 channel (none by default, and then nothing is fetched), the weekend card
//! with podium predictions (live over the gateway, closed at lights out), the "lights out in 1 hour" line, and the
//! results card with the scores and the season's table, once each, against a fake Jolpica-F1 whose drivers are made
//! up.
mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use chrono::{DateTime, TimeDelta, Utc};
use common::*;
use futures::{SinkExt, StreamExt};
use pulse_protocol::gateway::{ClientFrame, ServerFrame};
use pulse_protocol::ids::{ChannelId, MessageId, ServerId};
use pulse_protocol::rest::{Channel, F1Card, F1Prediction, F1Status, Message, MessageKind};
use pulse_server::f1::worker::tick;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as Ws;

/// A stand-in for Jolpica: answers what it was given by path (`current.json`, `2030/2/results.json`, ...), an empty
/// table for anything else, and remembers every path asked.
#[derive(Clone, Default)]
struct FakeJolpica {
    bodies: Arc<Mutex<HashMap<String, Value>>>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakeJolpica {
    fn set(&self, path: &str, body: Value) {
        self.bodies.lock().unwrap().insert(path.into(), body);
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn asked(&self, part: &str) -> usize {
        self.calls().iter().filter(|c| c.contains(part)).count()
    }
}

async fn fake_jolpica() -> (FakeJolpica, String) {
    let fake = FakeJolpica::default();
    async fn answer(
        State(f): State<FakeJolpica>,
        Path(path): Path<String>,
    ) -> (StatusCode, axum::Json<Value>) {
        f.calls.lock().unwrap().push(path.clone());
        let body = f.bodies.lock().unwrap().get(&path).cloned();
        (
            StatusCode::OK,
            axum::Json(body.unwrap_or_else(|| {
                json!({"MRData": {"total": "0", "RaceTable": {"Races": []},
                                  "StandingsTable": {"StandingsLists": []}, "DriverTable": {"Drivers": []}}})
            })),
        )
    }
    let router = Router::new()
        .route("/ergast/f1/{*path}", get(answer))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/ergast/f1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (fake, base)
}

fn date_time(t: DateTime<Utc>) -> Value {
    json!({"date": t.format("%Y-%m-%d").to_string(), "time": t.format("%H:%M:%SZ").to_string()})
}

/// A race on the calendar with lights out at `race`: practice two days before, qualifying the day before.
fn race(round: u16, name: &str, race: DateTime<Utc>) -> Value {
    let mut r = json!({
        "season": "2030", "round": round.to_string(), "raceName": name,
        "Circuit": {"circuitId": "c", "circuitName": format!("{name} Circuit"),
                    "Location": {"lat": "0", "long": "0", "locality": "Port Town", "country": "UK"}},
        "FirstPractice": date_time(race - TimeDelta::days(2)),
        "Qualifying": date_time(race - TimeDelta::days(1)),
    });
    let d = date_time(race);
    r["date"] = d["date"].clone();
    r["time"] = d["time"].clone();
    r
}

fn schedule(races: Vec<Value>) -> Value {
    json!({"MRData": {"total": races.len().to_string(), "RaceTable": {"season": "2030", "Races": races}}})
}

const POINTS: [&str; 5] = ["25", "18", "15", "12", "10"];

/// Made-up drivers, finishing in this order.
const FIELD: [(&str, &str, &str, &str); 5] = [
    ("racer_a", "RAA", "Ace", "Racer"),
    ("rider_b", "RIB", "Bo", "Rider"),
    ("speed_c", "SPC", "Cy", "Speed"),
    ("dash_d", "DAD", "Di", "Dash"),
    ("zoom_e", "ZOE", "Ed", "Zoom"),
];

fn results(round: u16, order: &[&str]) -> Value {
    let rows: Vec<Value> = order
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let (_, code, given, family) = FIELD.iter().find(|d| d.0 == *id).unwrap();
            json!({"position": (i + 1).to_string(), "positionText": (i + 1).to_string(),
                   "points": POINTS[i],
                   "Driver": {"driverId": id, "code": code, "givenName": given, "familyName": family},
                   "Constructor": {"name": "Rocket"}, "status": "Finished",
                   "Time": {"time": if i == 0 { "1:30:00.000".to_string() } else { format!("+{i}.000") }},
                   "FastestLap": {"rank": if i == 1 { "1" } else { "2" }, "lap": "40", "Time": {"time": "1:20.001"}}})
        })
        .collect();
    json!({"MRData": {"total": rows.len().to_string(), "RaceTable": {"Races": [{
        "season": "2030", "round": round.to_string(), "raceName": "x", "date": "2030-01-01",
        "Circuit": {"circuitName": "x", "Location": {"locality": "x", "country": "x"}},
        "Results": rows}]}}})
}

fn standings(round: u16) -> Value {
    let rows: Vec<Value> = FIELD
        .iter()
        .enumerate()
        .map(|(i, (id, code, given, family))| {
            json!({"position": (i + 1).to_string(), "points": (100 - i * 10).to_string(), "wins": "1",
                   "Driver": {"driverId": id, "code": code, "givenName": given, "familyName": family},
                   "Constructors": [{"name": "Rocket"}]})
        })
        .collect();
    json!({"MRData": {"StandingsTable": {"season": "2030", "round": round.to_string(),
        "StandingsLists": [{"season": "2030", "round": round.to_string(), "DriverStandings": rows}]}}})
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn hello(app: &TestApp, token: &str) -> Socket {
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
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Ws::Text(t) = m
            && let Ok(ServerFrame::Ready(_)) = serde_json::from_str(&t)
        {
            return ws;
        }
    }
}

async fn next_raw(ws: &mut Socket, pick: impl Fn(&Value) -> bool) -> Option<Value> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .ok()??
            .ok()?;
        if let Ws::Text(t) = m {
            let v: Value = serde_json::from_str(&t).unwrap();
            if v["op"] == "Event" && pick(&v["d"]) {
                return Some(v["d"].clone());
            }
        }
    }
}

async fn put_json(app: &TestApp, token: &str, path: &str, body: Value) -> reqwest::Response {
    app.http
        .put(app.url(path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// alex owns a server sam and kim are in; jo isn't in it. Jolpica is the fake. Returns (app, fake, tokens [alex,
/// sam, kim, jo], server, #general).
async fn setup() -> (TestApp, FakeJolpica, [String; 4], ServerId, Channel) {
    let (fake, base) = fake_jolpica().await;
    let mut cfg = test_config();
    cfg.f1.api_base = base;
    let app = spawn_with(cfg).await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    let (_, kim) = register(&app, "kim").await;
    let (_, jo) = register(&app, "jo").await;
    let s = create_server(&app, &alex, "Main").await;
    for t in [&sam, &kim] {
        let r = post_json(&app, t, &format!("/servers/{}/join", s.id), json!({})).await;
        assert!(r.status().is_success());
    }
    let g = general(&app, &alex, s.id).await;
    (app, fake, [alex, sam, kim, jo], s.id, g)
}

async fn pick_channel(
    app: &TestApp,
    token: &str,
    server: ServerId,
    ch: Option<ChannelId>,
) -> reqwest::Response {
    put_json(
        app,
        token,
        &format!("/servers/{server}/f1-channel"),
        json!({ "channel_id": ch }),
    )
    .await
}

async fn f1_lines(app: &TestApp, token: &str, ch: ChannelId) -> Vec<Message> {
    let list: Vec<Message> = get_json(app, token, &format!("/channels/{ch}/messages")).await;
    let mut lines: Vec<Message> = list.into_iter().filter(|m| m.f1.is_some()).collect();
    lines.sort_by_key(|m| m.id.to_string());
    lines
}

async fn predict(app: &TestApp, token: &str, card: MessageId, picks: Value) -> reqwest::Response {
    put_json(
        app,
        token,
        &format!("/messages/{card}/f1/prediction"),
        json!({ "picks": picks }),
    )
    .await
}

#[tokio::test]
async fn nothing_is_fetched_or_posted_without_an_f1_channel() {
    let (app, fake, [alex, ..], server, g) = setup().await;
    fake.set(
        "current.json",
        schedule(vec![race(
            1,
            "Harbour Grand Prix",
            Utc::now() + TimeDelta::hours(3),
        )]),
    );
    assert_eq!(tick(&app.state(), Utc::now()).await.unwrap(), 0);
    let st: F1Status = get_json(&app, &alex, &format!("/servers/{server}/f1")).await;
    assert_eq!(
        st,
        F1Status {
            channel_id: None,
            next: None,
            season_table: vec![]
        }
    );
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());
    assert!(f1_lines(&app, &alex, g.id).await.is_empty());
}

#[tokio::test]
async fn only_the_owner_picks_one_of_the_server_s_text_channels() {
    let (app, fake, [alex, sam, _, jo], server, g) = setup().await;
    let lights_out = Utc::now() + TimeDelta::days(5);
    fake.set(
        "current.json",
        schedule(vec![race(1, "Harbour Grand Prix", lights_out)]),
    );
    assert_eq!(
        pick_channel(&app, &sam, server, Some(g.id)).await.status(),
        403
    );
    let r = post_json(
        &app,
        &alex,
        &format!("/servers/{server}/channels"),
        json!({"kind": "voice", "name": "Pit"}),
    )
    .await;
    let voice: Channel = r.json().await.unwrap();
    assert_eq!(
        pick_channel(&app, &alex, server, Some(voice.id))
            .await
            .status(),
        400
    );
    let r = pick_channel(&app, &alex, server, Some(g.id)).await;
    assert_eq!(r.status(), 200);
    let st: F1Status = r.json().await.unwrap();
    assert_eq!(st.channel_id, Some(g.id));
    let next = st.next.unwrap();
    assert_eq!(
        (next.round, next.race_name.as_str(), next.country.as_str()),
        (1, "Harbour Grand Prix", "UK")
    );
    assert_eq!(next.sessions.len(), 3, "practice, qualifying, the race");
    // everyone in the server sees it; the calendar was asked once (cached since)
    let st: F1Status = get_json(&app, &sam, &format!("/servers/{server}/f1")).await;
    assert_eq!(st.channel_id, Some(g.id));
    assert_eq!(fake.asked("current.json"), 1);
    let r = app
        .http
        .get(app.url(&format!("/servers/{server}/f1")))
        .bearer_auth(&jo)
        .send()
        .await
        .unwrap();
    assert!(r.status() == 403 || r.status() == 404, "{}", r.status());
    // and back to none
    let st: F1Status = pick_channel(&app, &alex, server, None)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(st.channel_id, None);
}

#[tokio::test]
async fn a_race_weekend_from_the_card_to_the_scores() {
    let (app, fake, [alex, sam, kim, jo], server, g) = setup().await;
    let s = app.state();
    let lights_out = Utc::now() + TimeDelta::hours(3);
    fake.set(
        "current.json",
        schedule(vec![
            race(1, "Desert Grand Prix", Utc::now() - TimeDelta::days(7)),
            race(2, "Harbour Grand Prix", lights_out),
        ]),
    );
    // the field to pick from: round 1's finishers
    fake.set(
        "2030/1/results.json",
        results(1, &["zoom_e", "dash_d", "speed_c", "rider_b", "racer_a"]),
    );
    assert_eq!(
        pick_channel(&app, &alex, server, Some(g.id)).await.status(),
        200
    );

    // the weekend card, once
    assert_eq!(tick(&s, Utc::now()).await.unwrap(), 1);
    assert_eq!(tick(&s, Utc::now()).await.unwrap(), 0);
    let lines = f1_lines(&app, &sam, g.id).await;
    assert_eq!(lines.len(), 1);
    let card = &lines[0];
    assert_eq!(card.kind, MessageKind::System);
    assert!(
        card.content
            .starts_with("🏁 Race weekend: Harbour Grand Prix (round 2)"),
        "{}",
        card.content
    );
    let Some(F1Card::Weekend(w)) = card.f1.as_deref() else {
        panic!("{:?}", card.f1)
    };
    assert_eq!(w.weekend.round, 2);
    let names: Vec<&str> = w.drivers.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        names,
        ["Ace Racer", "Bo Rider", "Cy Speed", "Di Dash", "Ed Zoom"],
        "by name"
    );
    assert_eq!(w.drivers[0].team.as_deref(), Some("Rocket"));
    assert!(w.predictions.is_empty());
    // round 1 is long over: its results were never asked for
    assert_eq!(fake.asked("2030/1/results.json"), 1, "only as the field");

    // predictions, live
    let mut ws = hello(&app, &alex).await;
    let r = predict(
        &app,
        &sam,
        card.id,
        json!(["racer_a", "rider_b", "speed_c"]),
    )
    .await;
    assert_eq!(r.status(), 200);
    let ev = next_raw(&mut ws, |d| d["t"] == "F1PredictionsChanged")
        .await
        .unwrap();
    assert_eq!(ev["d"]["message_id"], card.id.to_string());
    assert_eq!(
        ev["d"]["predictions"][0]["picks"],
        json!(["racer_a", "rider_b", "speed_c"])
    );
    let all: Vec<F1Prediction> =
        predict(&app, &kim, card.id, json!(["rider_b", "racer_a", "zoom_e"]))
            .await
            .json()
            .await
            .unwrap();
    assert_eq!(all.len(), 2);
    for bad in [
        json!(["racer_a", "racer_a", "speed_c"]),
        json!(["racer_a", "rider_b"]),
        json!(["racer_a", "rider_b", "nobody"]),
    ] {
        assert_eq!(
            predict(&app, &kim, card.id, bad.clone()).await.status(),
            400,
            "{bad}"
        );
    }
    assert_eq!(
        predict(&app, &jo, card.id, json!(["racer_a", "rider_b", "speed_c"]))
            .await
            .status(),
        403,
        "jo isn't in the server"
    );
    // alex picks, then takes it back
    predict(&app, &alex, card.id, json!(["dash_d", "zoom_e", "racer_a"])).await;
    let all: Vec<F1Prediction> = predict(&app, &alex, card.id, Value::Null)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    let lines = f1_lines(&app, &kim, g.id).await;
    let Some(F1Card::Weekend(w)) = lines[0].f1.as_deref() else {
        panic!()
    };
    assert_eq!(w.predictions.len(), 2, "the history carries them");
    assert!(w.predictions.iter().all(|p| p.points.is_none()));

    // lights out in an hour, naming the card
    assert_eq!(
        tick(&s, lights_out - TimeDelta::minutes(30)).await.unwrap(),
        1
    );
    assert_eq!(
        tick(&s, lights_out - TimeDelta::minutes(29)).await.unwrap(),
        0
    );
    let lines = f1_lines(&app, &sam, g.id).await;
    let Some(F1Card::LightsOut(lo)) = lines[1].f1.as_deref() else {
        panic!("{:?}", lines[1].f1)
    };
    assert_eq!(lo.weekend_message_id, Some(card.id));
    assert_eq!(
        lines[1].content,
        "🚦 Lights out in 1 hour: Harbour Grand Prix"
    );

    // sam has read the chat (the inbox lists mentions in chats you have a read point in)
    send(&app, &sam, g.id, "come on racer_a").await;
    // during the race nothing is asked; after it, the results are polled until they're in
    tick(&s, lights_out + TimeDelta::minutes(30)).await.unwrap();
    assert_eq!(fake.asked("2030/2/"), 0);
    let after = lights_out + TimeDelta::hours(2);
    assert_eq!(tick(&s, after).await.unwrap(), 0);
    assert_eq!(fake.asked("2030/2/results.json"), 1, "not in yet");
    fake.set(
        "2030/2/results.json",
        results(2, &["racer_a", "speed_c", "rider_b", "dash_d", "zoom_e"]),
    );
    fake.set("2030/2/driverStandings.json", standings(2));
    assert_eq!(
        tick(&s, after + TimeDelta::minutes(1)).await.unwrap(),
        0,
        "cached for a few minutes"
    );
    assert_eq!(fake.asked("2030/2/results.json"), 1);
    assert_eq!(tick(&s, after + TimeDelta::minutes(6)).await.unwrap(), 1);
    assert_eq!(tick(&s, after + TimeDelta::minutes(20)).await.unwrap(), 0);
    assert_eq!(
        fake.asked("2030/2/results.json"),
        2,
        "never again once they're in"
    );
    assert_eq!(fake.asked("2030/2/driverStandings.json"), 1);

    let lines = f1_lines(&app, &sam, g.id).await;
    assert_eq!(lines.len(), 4, "card, lights out, results, scores");
    let Some(F1Card::Results(res)) = lines[2].f1.as_deref() else {
        panic!("{:?}", lines[2].f1)
    };
    assert_eq!(
        lines[2].content,
        "🏆 Harbour Grand Prix: 1. Ace Racer, 2. Cy Speed, 3. Bo Rider"
    );
    assert_eq!(res.results.len(), 5);
    assert_eq!(res.fastest_lap.as_ref().unwrap().driver.id, "speed_c");
    assert_eq!(res.standings.len(), 5);
    let Some(F1Card::Scores(sc)) = lines[3].f1.as_deref() else {
        panic!("{:?}", lines[3].f1)
    };
    // sam: P1 exact (3), P2 and P3 swapped (1 + 1) = 5; kim: rider_b and racer_a on the podium elsewhere = 2
    let pts: Vec<u32> = sc.scores.iter().map(|x| x.points).collect();
    assert_eq!(pts, [5, 2]);
    assert_eq!(
        lines[3].content,
        "🔮 Podium predictions, Harbour Grand Prix: sam 5, kim 2"
    );
    assert_eq!(sc.weekend_message_id, Some(card.id));
    // the predictors are pinged (#403): a mention each, so it's in their inbox
    let predictors: Vec<_> = sc.scores.iter().map(|x| x.user_id).collect();
    assert_eq!(predictors.len(), 2);
    assert_eq!(lines[3].mentions, predictors);
    assert!(
        lines[2].mentions.is_empty(),
        "the results card pings nobody"
    );
    let inbox: Value = get_json(&app, &sam, "/inbox").await;
    assert!(
        inbox["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["message"]["id"] == json!(lines[3].id)),
        "{inbox}"
    );
    assert_eq!(
        sc.drivers.len(),
        5,
        "the field, to name picks off the podium"
    );
    assert_eq!(
        sc.season_table.iter().map(|r| r.points).collect::<Vec<_>>(),
        [5, 2]
    );
    // the card shows each prediction's points now, and the season's table is the banner's too
    let Some(F1Card::Weekend(w)) = lines[0].f1.as_deref() else {
        panic!()
    };
    assert_eq!(
        w.predictions.iter().map(|p| p.points).collect::<Vec<_>>(),
        [Some(5), Some(2)]
    );
    let st: F1Status = get_json(&app, &kim, &format!("/servers/{server}/f1")).await;
    assert_eq!(
        st.season_table.iter().map(|r| r.points).collect::<Vec<_>>(),
        [5, 2]
    );
    assert_eq!(
        count(&app, "SELECT value FROM badge_stats WHERE key = 'f1_season_points' ORDER BY value DESC LIMIT 1").await,
        5
    );
}

#[tokio::test]
async fn predictions_close_at_lights_out() {
    let (app, fake, [alex, sam, ..], server, g) = setup().await;
    let lights_out = Utc::now() - TimeDelta::minutes(1);
    fake.set(
        "current.json",
        schedule(vec![race(1, "Desert Grand Prix", lights_out)]),
    );
    fake.set(
        "2030/drivers.json",
        json!({"MRData": {"DriverTable": {"Drivers": [
            {"driverId": "racer_a", "givenName": "Ace", "familyName": "Racer"},
            {"driverId": "rider_b", "givenName": "Bo", "familyName": "Rider"},
            {"driverId": "speed_c", "givenName": "Cy", "familyName": "Speed"}]}}}),
    );
    pick_channel(&app, &alex, server, Some(g.id)).await;
    // the card went out earlier in the weekend (the first race: the season's entry list to pick from)
    assert_eq!(
        tick(&app.state(), lights_out - TimeDelta::hours(5))
            .await
            .unwrap(),
        1
    );
    let lines = f1_lines(&app, &sam, g.id).await;
    let Some(F1Card::Weekend(w)) = lines[0].f1.as_deref() else {
        panic!()
    };
    assert_eq!(w.drivers.len(), 3);
    assert_eq!(w.drivers[0].team, None);
    let r = predict(
        &app,
        &sam,
        lines[0].id,
        json!(["racer_a", "rider_b", "speed_c"]),
    )
    .await;
    assert_eq!(r.status(), 409);
}

#[tokio::test]
async fn the_results_wait_a_while_for_the_standings_then_go_without() {
    let (app, fake, [alex, sam, ..], server, g) = setup().await;
    let s = app.state();
    let lights_out = Utc::now() - TimeDelta::days(1);
    fake.set(
        "current.json",
        schedule(vec![race(1, "Desert Grand Prix", lights_out)]),
    );
    fake.set(
        "2030/1/results.json",
        results(1, &["racer_a", "rider_b", "speed_c"]),
    );
    pick_channel(&app, &alex, server, Some(g.id)).await;
    let t0 = lights_out + TimeDelta::hours(2);
    assert_eq!(tick(&s, t0).await.unwrap(), 0, "no standings yet");
    assert_eq!(tick(&s, t0 + TimeDelta::minutes(30)).await.unwrap(), 0);
    assert_eq!(tick(&s, t0 + TimeDelta::minutes(61)).await.unwrap(), 1);
    let lines = f1_lines(&app, &sam, g.id).await;
    assert_eq!(
        lines.len(),
        1,
        "nobody predicted (no weekend card): no scores"
    );
    let Some(F1Card::Results(r)) = lines[0].f1.as_deref() else {
        panic!()
    };
    assert!(r.standings.is_empty());
    assert_eq!(r.results.len(), 3);
}
