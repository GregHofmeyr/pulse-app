//! The builds routes (#616): adding, reading, listing, editing, archiving and deleting builds.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{ChannelId, GameId, MessageId, UserId};
use pulse_protocol::rest::{AttachmentKind, Channel, ChannelKind, CreateBuildRequest, Message};
use pulse_protocol::rest::{BuildGameEdit, UpdateBuildRequest};
use serde::Deserialize;

use super::{
    MAX_SCREENSHOTS, PAGE, UNTITLED, clean_link, clean_notes, clean_title, game_name, link_title,
    render_content,
};
use crate::AppState;
use crate::access::channel_for;
use crate::auth::AuthUser;
use crate::db::{begin_write, now};
use crate::error::{AppError, AppResult};
use crate::messages::nonce;
use crate::messages::routes::load;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/channels/{id}/builds", post(create).get(list))
        .route("/builds/search", get(search))
        .route(
            "/builds/{message_id}",
            get(get_one).put(edit).delete(super::archive::remove),
        )
        .merge(super::archive::router())
        .merge(super::favourites::router())
}

pub(crate) fn bad(msg: &str) -> AppError {
    AppError::BadRequest(msg.into())
}

/// A builds channel `me` may read (404 for anyone outside its server, as any channel); 400 for another kind.
pub(crate) async fn builds_channel(s: &AppState, me: UserId, id: ChannelId) -> AppResult<Channel> {
    let ch = channel_for(&s.db, me, id).await?;
    if ch.kind != ChannelKind::Builds {
        return Err(bad("this isn't a builds channel"));
    }
    Ok(ch)
}

/// The game, checked to be in the channel's server.
pub(crate) async fn checked_game(
    s: &AppState,
    ch: &Channel,
    game: Option<GameId>,
) -> AppResult<Option<GameId>> {
    if let Some(g) = game {
        let server = ch.server_id.ok_or(AppError::NotFound)?;
        if !crate::clips::games::in_server(&s.db, server, g).await? {
            return Err(bad("That game isn't in this server."));
        }
    }
    Ok(game)
}

/// The uploads a build names as its screenshots, claimed (see `attachments::claim`): at most
/// [`MAX_SCREENSHOTS`], images only.
pub(crate) async fn claim_shots(
    s: &AppState,
    me: UserId,
    channel: ChannelId,
    ids: &[pulse_protocol::ids::AttachmentId],
) -> AppResult<Vec<pulse_protocol::rest::Attachment>> {
    if ids.len() > MAX_SCREENSHOTS {
        return Err(bad("Up to 10 screenshots."));
    }
    if ids.is_empty() {
        return Ok(vec![]);
    }
    // A screenshot is kept with its build: never a file that expires or vanishes once seen.
    let mut q = sqlx::QueryBuilder::new("SELECT COUNT(*) FROM attachments WHERE user_id = ");
    q.push_bind(me.to_string())
        .push(" AND (expires_at IS NOT NULL OR view_once = 1) AND id IN (");
    let mut list = q.separated(", ");
    for id in ids {
        list.push_bind(id.to_string());
    }
    q.push(")");
    let fleeting: i64 = q.build_query_scalar().fetch_one(&s.db).await?;
    if fleeting > 0 {
        return Err(bad(
            "Screenshots are kept with the build: not a temporary or view-once file.",
        ));
    }
    let claimed = crate::attachments::claim(s, me, channel, ids).await?;
    if claimed.iter().any(|a| a.kind != AttachmentKind::Image) {
        return Err(bad("Screenshots must be images."));
    }
    Ok(claimed)
}

/// `POST /channels/{id}/builds`: a new build. Only a title, a link or a screenshot is needed; with none of them typed,
/// the title is made from the link, or is "Untitled build". Anyone who can post in the channel may add one.
async fn create(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(channel): Path<ChannelId>,
    Json(req): Json<CreateBuildRequest>,
) -> AppResult<Json<Message>> {
    if !s.limits.send_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    let ch = builds_channel(&s, me, channel).await?;
    crate::access::require_open(&ch)?;
    nonce::check(req.nonce.as_deref())?;
    // A retry of a create that already landed gets the same build back (its files belong to it now).
    if let Some(existing) = nonce::existing(
        &mut *s.db.acquire().await?,
        me,
        channel,
        req.nonce.as_deref(),
    )
    .await?
    {
        return replay(&s, me, existing).await;
    }
    let typed = clean_title(req.title.as_deref())?;
    let link = req.link_url.as_deref().map(clean_link).transpose()?;
    let notes = clean_notes(req.notes.as_deref())?;
    let game = checked_game(&s, &ch, req.game_id).await?;
    let mut shots = claim_shots(&s, me, channel, &req.attachments).await?;
    let (title, title_auto) = match (typed, &link) {
        (Some(t), _) => (t, false),
        (None, Some(l)) => (link_title(l), true),
        (None, None) if !shots.is_empty() => (UNTITLED.to_string(), true),
        (None, None) => return Err(bad("A build needs a title, a link or a screenshot.")),
    };
    let quick_drop = game.is_none() && notes.is_none();
    let content = render_content(
        &title,
        game_name(&s.db, game).await?.as_deref(),
        None,
        notes.as_deref(),
    );

    let id = MessageId::new();
    // As in a send: publishes (and the absolute unread counts) go out in commit order.
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, channel).await?; // channel deleted since (#109)
    crate::access::open_in(&mut tx, channel).await?; // archived since (#303)
    if let Some(existing) = nonce::existing(&mut tx, me, channel, req.nonce.as_deref()).await? {
        tx.rollback().await?;
        return replay(&s, me, existing).await;
    }
    // a member still: one who left since the read above can't post
    crate::access::member_of_in(&mut tx, me, channel).await?;
    sqlx::query(
        "INSERT INTO messages (id, channel_id, author_id, kind, content, created_at, nonce)
         VALUES (?, ?, ?, 'normal', ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(channel.to_string())
    .bind(me.to_string())
    .bind(&content)
    .bind(now())
    .bind(&req.nonce)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO builds (message_id, channel_id, title, title_auto, game_id, link_url, notes, quick_drop)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(channel.to_string())
    .bind(&title)
    .bind(title_auto)
    .bind(game.map(|g| g.to_string()))
    .bind(&link)
    .bind(&notes)
    .bind(quick_drop)
    .execute(&mut *tx)
    .await?;
    crate::attachments::bind(&mut tx, me, id, &req.attachments, &mut shots).await?;
    let moved = crate::reads::advance(&mut tx, me, channel, id).await?;
    tx.commit().await?;

    if let Some(message) = load(&s.db, None, id).await? {
        s.hub
            .publish(
                &s.db,
                Event::MessageCreated {
                    message,
                    nonce: req.nonce.clone(),
                },
            )
            .await;
    }
    if moved
        && let Some((unread, mentions)) =
            crate::reads::counts_after_commit(&s.db, me, channel).await
    {
        s.hub
            .publish(
                &s.db,
                Event::ReadStateUpdated {
                    user_id: me,
                    channel_id: channel,
                    last_read_message_id: Some(id),
                    mentions: Some(mentions),
                    unread: Some(unread),
                },
            )
            .await;
    }
    drop(_order);
    // the link's page fills in its title, thumbnail and game afterwards
    if let Some(link) = link {
        super::unfurl::spawn_fill(&s, id, me, link);
    }
    let msg = load(&s.db, Some(me), id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(msg))
}

/// The build an earlier create with the same nonce made (nothing new is published); 409 when that nonce was spent on
/// something that isn't a build.
async fn replay(s: &AppState, me: UserId, id: MessageId) -> AppResult<Json<Message>> {
    let msg = load(&s.db, Some(me), id).await?.ok_or(AppError::NotFound)?;
    if msg.build.is_none() {
        return Err(AppError::Conflict("That request was already used.".into()));
    }
    Ok(Json(msg))
}

/// `GET /builds/{id}`: one build, archived or not, for anyone who can read its channel.
async fn get_one(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
) -> AppResult<Json<Message>> {
    let (channel, _) = super::build_channel(&s.db, id).await?;
    channel_for(&s.db, me, channel).await?;
    let msg = load(&s.db, Some(me), id).await?.ok_or(AppError::NotFound)?;
    if msg.build.is_none() {
        return Err(AppError::NotFound);
    }
    Ok(Json(msg))
}

#[derive(Deserialize)]
struct ListQuery {
    before: Option<MessageId>,
    limit: Option<u32>,
    game: Option<GameId>,
    #[serde(default)]
    archived: bool,
    /// Words to find in a build's title, game, link title or notes.
    q: Option<String>,
}

/// The search index's query for `q`'s words (`None`: no words, nothing to narrow by).
fn words(q: Option<&str>) -> Option<String> {
    crate::search::text::fts_query(&crate::search::text::terms(q?))
}

/// Narrows `qb` (over `builds b`) to builds whose message matches `fts` in the search index.
fn push_match(qb: &mut sqlx::QueryBuilder<sqlx::Sqlite>, fts: String) {
    qb.push(
        " AND b.message_id IN (SELECT d.message_id FROM search_docs d
           WHERE d.doc IN (SELECT rowid FROM message_search WHERE message_search MATCH ",
    )
    .push_bind(fts)
    .push("))");
}

/// `GET /channels/{id}/builds`: a page of a builds channel's live builds, newest first by when they were saved (an
/// edit never moves one). Archived builds only with `archived=true`; `game` narrows it to one game.
async fn list(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<ChannelId>,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<Vec<Message>>> {
    builds_channel(&s, me, id).await?;
    let limit = q.limit.unwrap_or(PAGE).clamp(1, PAGE);
    let cols = crate::messages::routes::COLS
        .split(", ")
        .map(|c| format!("m.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut qb = sqlx::QueryBuilder::new(format!(
        "SELECT {cols} FROM builds b JOIN messages m ON m.id = b.message_id WHERE b.channel_id = "
    ));
    qb.push_bind(id.to_string());
    qb.push(" AND m.deleted_at IS NULL");
    if !q.archived {
        qb.push(" AND b.archived_at IS NULL");
    }
    if let Some(before) = q.before {
        qb.push(" AND b.message_id < ")
            .push_bind(before.to_string());
    }
    if let Some(game) = q.game {
        qb.push(" AND b.game_id = ").push_bind(game.to_string());
    }
    if let Some(fts) = words(q.q.as_deref()) {
        push_match(&mut qb, fts);
    }
    qb.push(" ORDER BY b.message_id DESC LIMIT ")
        .push_bind(limit);
    let rows: Vec<crate::messages::routes::Row> = qb.build_query_as().fetch_all(&s.db).await?;
    Ok(Json(
        crate::messages::routes::fill(&s.db, Some(me), rows).await?,
    ))
}

/// `PUT /builds/{id}`: the author's edit, replacing every field (same checks as a create). The title is the author's
/// from now on. The game: `game: "unchanged"` leaves it as it is (a guess stays a guess), `{"set": id}` picks or keeps
/// one (no longer "guessed"), `"clear"` drops it; without `game` (an older client) `game_id` is a pick, or none clears
/// it. A quick drop ends with an explicit game, notes or `mark_sorted`, and never comes back. A new link drops what was fetched of the old one and is fetched in turn. The
/// build keeps its place in the list.
async fn edit(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Path(id): Path<MessageId>,
    Json(req): Json<UpdateBuildRequest>,
) -> AppResult<Json<Message>> {
    if !s.limits.mutate_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    crate::media::writes_ok(&s.cfg)?;
    let (msg, ch) = crate::messages::routes::own_message(&s.db, me, id).await?;
    let Some(build) = msg.build.as_deref() else {
        return Err(AppError::NotFound);
    };
    crate::access::require_member_of(&s.db, me, &ch).await?;
    crate::access::require_open(&ch)?;
    let title = clean_title(Some(&req.title))?.ok_or_else(|| bad("A build needs a title."))?;
    let link = req.link_url.as_deref().map(clean_link).transpose()?;
    let notes = clean_notes(req.notes.as_deref())?;
    // `None`: leave the game (and whether it was guessed) as it is
    let game = match req.game {
        Some(BuildGameEdit::Unchanged) => None,
        Some(BuildGameEdit::Set(g)) => Some(checked_game(&s, &ch, Some(g)).await?),
        Some(BuildGameEdit::Clear) => Some(None),
        None => Some(checked_game(&s, &ch, req.game_id).await?),
    };
    let mut seen = std::collections::HashSet::new();
    if !req.attachments.iter().all(|a| seen.insert(*a)) {
        return Err(bad("A screenshot is listed twice."));
    }
    let had: std::collections::HashSet<_> = msg.attachments.iter().map(|a| a.id).collect();
    let new: Vec<_> = req
        .attachments
        .iter()
        .copied()
        .filter(|a| !had.contains(a))
        .collect();
    if req.attachments.len() > MAX_SCREENSHOTS {
        return Err(bad("Up to 10 screenshots."));
    }
    claim_shots(&s, me, ch.id, &new).await?;
    let old_link = build.link.as_ref().map(|l| l.url.as_str());
    let link_changed = link.as_deref() != old_link;
    // still a quick drop only if it was one (read in the write: it never comes back); a game left as it is ends none
    let still_quick = game.is_none_or(|g| g.is_none()) && notes.is_none() && !req.mark_sorted;

    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    crate::access::channel_live_in(&mut tx, ch.id).await?;
    crate::access::open_in(&mut tx, ch.id).await?;
    crate::access::member_of_in(&mut tx, me, ch.id).await?;
    let changed = sqlx::query(
        "UPDATE builds SET title = ?1, title_auto = 0,
                           game_id = CASE WHEN ?10 THEN game_id ELSE ?2 END,
                           game_guessed = CASE WHEN ?10 THEN game_guessed ELSE 0 END, notes = ?3,
                           quick_drop = quick_drop AND ?4, updated_at = ?5, link_url = ?6,
                           link_title = CASE WHEN ?7 THEN NULL ELSE link_title END,
                           link_thumb = CASE WHEN ?7 THEN NULL ELSE link_thumb END,
                           link_provider = CASE WHEN ?7 THEN NULL ELSE link_provider END,
                           link_provider_id = CASE WHEN ?7 THEN NULL ELSE link_provider_id END
         WHERE message_id = ?8 AND EXISTS (SELECT 1 FROM messages m
                                          WHERE m.id = ?8 AND m.author_id = ?9 AND m.deleted_at IS NULL)",
    )
    .bind(&title)
    .bind(game.flatten().map(|g| g.to_string()))
    .bind(&notes)
    .bind(still_quick)
    .bind(now())
    .bind(&link)
    .bind(link_changed)
    .bind(id.to_string())
    .bind(me.to_string())
    .bind(game.is_none())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::NotFound);
    }
    // The search text from the row as it now stands: a link title fetched while this edit was on its way is kept.
    let (link_title, game_name): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT b.link_title, g.name FROM builds b LEFT JOIN games g ON g.id = b.game_id
         WHERE b.message_id = ?",
    )
    .bind(id.to_string())
    .fetch_one(&mut *tx)
    .await?;
    let content = render_content(
        &title,
        game_name.as_deref(),
        link_title.as_deref(),
        notes.as_deref(),
    );
    sqlx::query("UPDATE messages SET content = ? WHERE id = ?")
        .bind(&content)
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    let gone =
        crate::attachments::set_message_files(&mut tx, me, id, &req.attachments, &new).await?;
    tx.commit().await?;
    // Events carry nobody's comment seen point; the answer carries mine.
    if let Some(message) = load(&s.db, None, id).await? {
        s.hub
            .publish(&s.db, Event::MessageUpdated { message })
            .await;
    }
    drop(_order);
    crate::attachments::remove_objects(&s, gone).await;
    if link_changed && let Some(link) = link {
        super::unfurl::spawn_fill(&s, id, me, link);
    }
    let msg = load(&s.db, Some(me), id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(msg))
}

/// The most builds one search answers.
pub const SEARCH_MAX: u32 = 20;

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
    /// A game's normalised name (`games.name_key`): one name across servers.
    game: Option<String>,
    limit: Option<u32>,
}

/// `GET /builds/search`: builds in every builds channel the caller can read whose title, game, link title or notes
/// have the words, of one game (by name, across servers) when asked: current ones first, then archived ones, each
/// newest first. Neither words nor a game: nothing.
async fn search(
    State(s): State<AppState>,
    AuthUser(me): AuthUser,
    Query(q): Query<SearchQuery>,
) -> AppResult<Json<Vec<Message>>> {
    if !s.limits.search_user.hit(&me.to_string()) {
        return Err(AppError::TooManyRequests(None));
    }
    let fts = words(q.q.as_deref());
    let game = q
        .game
        .as_deref()
        .map(crate::quotes::name_key)
        .filter(|g| !g.is_empty());
    if fts.is_none() && game.is_none() {
        return Ok(Json(vec![]));
    }
    let limit = q.limit.unwrap_or(SEARCH_MAX).clamp(1, SEARCH_MAX);
    let cols = crate::messages::routes::COLS
        .split(", ")
        .map(|c| format!("m.{c}"))
        .collect::<Vec<_>>()
        .join(", ");
    // the channels `me` can read: builds channels of the live servers they're in
    let mut qb = sqlx::QueryBuilder::new(format!(
        "SELECT {cols} FROM builds b
         JOIN messages m ON m.id = b.message_id AND m.deleted_at IS NULL
         JOIN channels c ON c.id = b.channel_id AND c.deleted_at IS NULL AND c.kind = 'builds'
         WHERE c.server_id NOT IN {} AND c.server_id IN (SELECT server_id FROM server_members WHERE user_id = ",
        crate::access::DELETED_SERVERS
    ));
    qb.push_bind(me.to_string()).push(")");
    if let Some(game) = game {
        qb.push(" AND b.game_id IN (SELECT id FROM games WHERE name_key = ")
            .push_bind(game)
            .push(")");
    }
    if let Some(fts) = fts {
        push_match(&mut qb, fts);
    }
    qb.push(" ORDER BY b.archived_at IS NOT NULL, b.message_id DESC LIMIT ")
        .push_bind(limit);
    let rows: Vec<crate::messages::routes::Row> = qb.build_query_as().fetch_all(&s.db).await?;
    Ok(Json(
        crate::messages::routes::fill(&s.db, Some(me), rows).await?,
    ))
}
