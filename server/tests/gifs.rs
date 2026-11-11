//! GIFs (#58): the GIPHY config handed to signed-in people (the server never calls GIPHY itself), favourites, and a
//! picked GIF on a message.
mod common;

use common::*;
use pulse_protocol::ids::ChannelId;
use pulse_protocol::rest::{Gif, GifConfig, Message, gif_limits};
use pulse_server::config::GiphyConfig;
use pulse_server::limits::{LimitsConfig, Rate};
use reqwest::Method;
use serde_json::{Value, json};

const ID: &str = "3o7TKSjRrfIPjeiVyM";

fn link(id: &str) -> String {
    format!("https://giphy.com/gifs/excited-yes-{id}")
}

fn gif(id: &str) -> Value {
    json!({ "id": id, "title": "Excited Yes GIF", "width": 480, "height": 270 })
}

async fn with_key() -> TestApp {
    let mut cfg = test_config();
    cfg.giphy = GiphyConfig {
        api_key: Some("test-giphy-key".into()),
        rating: "pg-13".into(),
    };
    spawn_with(cfg).await
}

async fn call(
    app: &TestApp,
    method: Method,
    token: &str,
    path: &str,
    body: Option<Value>,
) -> reqwest::Response {
    let mut r = app.http.request(method, app.url(path)).bearer_auth(token);
    if let Some(b) = body {
        r = r.json(&b);
    }
    r.send().await.unwrap()
}

async fn chat(app: &TestApp, token: &str) -> ChannelId {
    let s = create_server(app, token, "Main").await;
    general(app, token, s.id).await.id
}

async fn send_gif(app: &TestApp, token: &str, ch: ChannelId, body: Value) -> reqwest::Response {
    post_json(app, token, &format!("/channels/{ch}/messages"), body).await
}

#[tokio::test]
async fn config_without_a_key_says_gifs_arent_set_up() {
    let app = spawn().await;
    let (_, t) = register(&app, "alex").await;
    let c: GifConfig = get_json(&app, &t, "/gifs/config").await;
    assert!(c.api_key.is_none());
    assert_eq!(c.rating, "pg-13");
}

#[tokio::test]
async fn config_hands_the_key_to_signed_in_people_only() {
    let app = with_key().await;
    let r = app.http.get(app.url("/gifs/config")).send().await.unwrap();
    assert_eq!(r.status(), 401);
    let (_, t) = register(&app, "alex").await;
    let c: GifConfig = get_json(&app, &t, "/gifs/config").await;
    assert_eq!(c.api_key.as_deref(), Some("test-giphy-key"));
    assert_eq!(c.rating, "pg-13");
}

#[tokio::test]
async fn config_is_rate_limited_per_person() {
    let mut cfg = test_config();
    cfg.limits = LimitsConfig {
        gif_user: Rate::new(2, 0.0001),
        ..LimitsConfig::unlimited()
    };
    let app = spawn_with(cfg).await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    for _ in 0..2 {
        assert_eq!(
            call(&app, Method::GET, &alex, "/gifs/config", None)
                .await
                .status(),
            200
        );
    }
    assert_eq!(
        call(&app, Method::GET, &alex, "/gifs/config", None)
            .await
            .status(),
        429
    );
    // someone else has their own bucket
    assert_eq!(
        call(&app, Method::GET, &sam, "/gifs/config", None)
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn favourites_add_list_remove_newest_first_and_per_person() {
    let app = spawn().await;
    let (_, alex) = register(&app, "alex").await;
    let (_, sam) = register(&app, "sam").await;
    for id in ["aaa111", "bbb222"] {
        let r = call(&app, Method::POST, &alex, "/gifs/favourites", Some(gif(id))).await;
        assert_eq!(r.status(), 204, "add {id}");
    }
    // adding one again is fine and doesn't duplicate it
    let r = call(
        &app,
        Method::POST,
        &alex,
        "/gifs/favourites",
        Some(gif("aaa111")),
    )
    .await;
    assert_eq!(r.status(), 204);
    let mine: Vec<Gif> = get_json(&app, &alex, "/gifs/favourites").await;
    assert_eq!(
        mine.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
        ["bbb222", "aaa111"]
    );
    assert_eq!(mine[0].title, "Excited Yes GIF");
    assert_eq!((mine[0].width, mine[0].height), (480, 270));
    let theirs: Vec<Gif> = get_json(&app, &sam, "/gifs/favourites").await;
    assert!(theirs.is_empty());

    let r = call(&app, Method::DELETE, &alex, "/gifs/favourites/bbb222", None).await;
    assert_eq!(r.status(), 204);
    // removing one that isn't there is fine too
    let r = call(&app, Method::DELETE, &alex, "/gifs/favourites/bbb222", None).await;
    assert_eq!(r.status(), 204);
    let mine: Vec<Gif> = get_json(&app, &alex, "/gifs/favourites").await;
    assert_eq!(
        mine.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
        ["aaa111"]
    );
}

#[tokio::test]
async fn favourites_refuse_bad_gifs_and_signed_out_callers() {
    let app = spawn().await;
    let (_, t) = register(&app, "alex").await;
    for bad in [
        json!({ "id": "", "title": "x", "width": 10, "height": 10 }),
        json!({ "id": "has spaces", "title": "x", "width": 10, "height": 10 }),
        json!({ "id": "../etc", "title": "x", "width": 10, "height": 10 }),
        json!({ "id": "a".repeat(gif_limits::ID_CHARS + 1), "title": "x", "width": 10, "height": 10 }),
        json!({ "id": "abc", "title": "x", "width": 0, "height": 10 }),
        json!({ "id": "abc", "title": "x", "width": 10, "height": gif_limits::MAX_SIDE + 1 }),
    ] {
        let r = call(
            &app,
            Method::POST,
            &t,
            "/gifs/favourites",
            Some(bad.clone()),
        )
        .await;
        assert_eq!(r.status(), 400, "{bad}");
    }
    let r = app
        .http
        .get(app.url("/gifs/favourites"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn a_long_title_is_cut_and_control_characters_go() {
    let app = spawn().await;
    let (_, t) = register(&app, "alex").await;
    let title = format!("  a\u{0}b{}  ", "x".repeat(300));
    let r = call(
        &app,
        Method::POST,
        &t,
        "/gifs/favourites",
        Some(json!({ "id": "abc", "title": title, "width": 10, "height": 10 })),
    )
    .await;
    assert_eq!(r.status(), 204);
    let mine: Vec<Gif> = get_json(&app, &t, "/gifs/favourites").await;
    assert!(mine[0].title.starts_with("abx"), "{}", mine[0].title);
    assert_eq!(mine[0].title.chars().count(), gif_limits::TITLE_CHARS);
}

#[tokio::test]
async fn favourites_are_capped() {
    let app = spawn().await;
    let (alex, t) = register(&app, "alex").await;
    // fill to the cap straight in the database, then one more through the route
    for i in 0..gif_limits::FAVOURITES_PER_PERSON {
        sqlx::query(
            "INSERT INTO gif_favourites (user_id, gif_id, title, width, height, created_at) VALUES (?, ?, '', 1, 1, ?)",
        )
        .bind(alex.to_string())
        .bind(format!("g{i}"))
        .bind(format!("2026-01-01T00:00:{:02}.000Z", i % 60))
        .execute(&app.db)
        .await
        .unwrap();
    }
    let r = call(&app, Method::POST, &t, "/gifs/favourites", Some(gif("one"))).await;
    assert_eq!(r.status(), 400);
    // one already there is still fine (nothing new is stored)
    let r = call(&app, Method::POST, &t, "/gifs/favourites", Some(gif("g7"))).await;
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn a_picked_gif_is_stored_on_its_message() {
    let app = spawn().await;
    let (_, t) = register(&app, "alex").await;
    let ch = chat(&app, &t).await;
    let r = send_gif(
        &app,
        &t,
        ch,
        json!({ "content": link(ID), "gif": gif(ID), "nonce": null, "reply_to_id": null }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let sent: Message = r.json().await.unwrap();
    let g = sent.gif.expect("gif on the sent message");
    assert_eq!(g.id, ID);
    assert_eq!(g.title, "Excited Yes GIF");
    assert_eq!(sent.content, link(ID));

    let list: Vec<Message> = get_json(&app, &t, &format!("/channels/{ch}/messages")).await;
    assert_eq!(list[0].gif.as_ref().map(|g| g.id.as_str()), Some(ID));
}

#[tokio::test]
async fn any_giphy_link_form_to_the_same_gif_is_accepted() {
    let app = spawn().await;
    let (_, t) = register(&app, "alex").await;
    let ch = chat(&app, &t).await;
    for content in [
        format!("https://giphy.com/gifs/{ID}"),
        format!("https://media.giphy.com/media/{ID}/giphy.gif"),
        format!("https://media3.giphy.com/media/v1.Y2lkPTc5/{ID}/200w.webp?cid=abc&rid=200w.webp"),
        format!("https://i.giphy.com/{ID}.webp"),
        format!("https://giphy.com/embed/{ID}"),
    ] {
        let r = send_gif(&app, &t, ch, json!({ "content": content, "gif": gif(ID) })).await;
        assert_eq!(r.status(), 200, "{content}");
    }
}

#[tokio::test]
async fn a_gif_must_match_its_link() {
    let app = spawn().await;
    let (_, t) = register(&app, "alex").await;
    let ch = chat(&app, &t).await;
    for content in [
        link("someOtherId"),
        "look at this".to_string(),
        format!("{} and more words", link(ID)),
        format!("https://giphy.com.evil.example/gifs/{ID}"),
    ] {
        let r = send_gif(&app, &t, ch, json!({ "content": content, "gif": gif(ID) })).await;
        assert_eq!(r.status(), 400, "{content}");
    }
    let r = send_gif(
        &app,
        &t,
        ch,
        json!({ "content": link("has space"), "gif": gif("has space") }),
    )
    .await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn editing_the_link_away_drops_the_gif() {
    let app = spawn().await;
    let (_, t) = register(&app, "alex").await;
    let ch = chat(&app, &t).await;
    let sent: Message = send_gif(&app, &t, ch, json!({ "content": link(ID), "gif": gif(ID) }))
        .await
        .json()
        .await
        .unwrap();
    // the same GIF in another form keeps it
    let r = call(
        &app,
        Method::PATCH,
        &t,
        &format!("/messages/{}", sent.id),
        Some(json!({ "content": format!("https://giphy.com/gifs/{ID}") })),
    )
    .await;
    assert_eq!(r.status(), 200);
    let kept: Message = r.json().await.unwrap();
    assert!(kept.gif.is_some());
    let r = call(
        &app,
        Method::PATCH,
        &t,
        &format!("/messages/{}", sent.id),
        Some(json!({ "content": "never mind" })),
    )
    .await;
    assert_eq!(r.status(), 200);
    let edited: Message = r.json().await.unwrap();
    assert!(edited.gif.is_none());
    let list: Vec<Message> = get_json(&app, &t, &format!("/channels/{ch}/messages")).await;
    assert!(list[0].gif.is_none());
}

#[tokio::test]
async fn a_deleted_gif_message_shows_no_gif() {
    let app = spawn().await;
    let (_, t) = register(&app, "alex").await;
    let ch = chat(&app, &t).await;
    let sent: Message = send_gif(&app, &t, ch, json!({ "content": link(ID), "gif": gif(ID) }))
        .await
        .json()
        .await
        .unwrap();
    let r = call(
        &app,
        Method::DELETE,
        &t,
        &format!("/messages/{}", sent.id),
        None,
    )
    .await;
    assert!(r.status().is_success());
    let list: Vec<Message> = get_json(&app, &t, &format!("/channels/{ch}/messages")).await;
    assert!(list[0].deleted);
    assert!(list[0].gif.is_none());
    // and nothing of it is kept: the delete clears the stored GIF as it clears the text
    let stored: Option<String> = sqlx::query_scalar("SELECT gif FROM messages WHERE id = ?")
        .bind(sent.id.to_string())
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(stored.is_none());
}

#[tokio::test]
async fn a_message_without_a_gif_has_none() {
    let app = spawn().await;
    let (_, t) = register(&app, "alex").await;
    let ch = chat(&app, &t).await;
    // a pasted GIPHY link is just a message: clients draw it from the link
    let m = send(&app, &t, ch, &link(ID)).await;
    assert!(m.gif.is_none());
    let raw: Value = get_json(&app, &t, &format!("/channels/{ch}/messages")).await;
    assert!(
        raw[0].get("gif").is_none(),
        "absent, not null, for older clients"
    );
}
