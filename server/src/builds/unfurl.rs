//! A build's link, fetched (#616): its page's title, thumbnail and player through the link preview unfurler, and the
//! game guessed from that title. The build is saved first; this fills it in afterwards, and never over what the author
//! typed or picked meanwhile.

use pulse_protocol::gateway::Event;
use pulse_protocol::ids::{GameId, MessageId, UserId};

use super::{fit_title, render_content};
use crate::AppState;
use crate::db::begin_write;
use crate::error::AppResult;

/// Fetch `link` for build `message_id` in the background (as a send's previews are: the author's unfurl allowance and
/// in-flight cap apply, and nothing happens when the disk is low).
pub fn spawn_fill(s: &AppState, message_id: MessageId, author: UserId, link: String) {
    if let Err(e) = crate::media::writes_ok(&s.cfg) {
        tracing::warn!(error = ?e, %message_id, "low on disk; no build link preview");
        return;
    }
    if !s.limits.unfurl_user.hit(&author.to_string()) {
        tracing::debug!(%message_id, "over the unfurl limit; no build link preview");
        return;
    }
    let s = s.clone();
    tokio::spawn(async move {
        if let Err(e) = fill(&s, message_id, author, link).await {
            tracing::warn!(error = ?e, %message_id, "couldn't fill in a build's link");
        }
    });
}

/// What [`spawn_fill`] runs: fetch, then write what came back while the build is live and its link is still `link`.
/// The fetched title replaces only a title made for the author (`title_auto`); a guessed game only fills an empty one.
pub async fn fill(
    s: &AppState,
    message_id: MessageId,
    author: UserId,
    link: String,
) -> AppResult<()> {
    let Some(_in_flight) = s.embeds.claim(author) else {
        tracing::debug!(%message_id, "too many unfurls in flight; no build link preview");
        return Ok(());
    };
    let Some(got) = s
        .embeds
        .unfurl(
            &s.db,
            &s.cfg.media_dir,
            author,
            &s.limits.link_thumb_user,
            &link,
        )
        .await
    else {
        // unreachable, refused or nothing there: the link stays the title
        return Ok(());
    };
    let games: Vec<(String, String)> = sqlx::query_as(
        "SELECT g.id, g.name FROM builds b
         JOIN channels c ON c.id = b.channel_id
         JOIN games g ON g.server_id = c.server_id
         WHERE b.message_id = ?",
    )
    .bind(message_id.to_string())
    .fetch_all(&s.db)
    .await?;
    let games: Vec<(GameId, String)> = games
        .into_iter()
        .filter_map(|(id, name)| Some((id.parse().ok()?, name)))
        .collect();

    // Written and published in order with edits, so an older copy never goes out last.
    let _order = s.hub.in_order().await;
    let mut tx = begin_write(&s.db).await?;
    type Current = (String, bool, Option<String>, Option<String>);
    let current: Option<Current> = sqlx::query_as(
        "SELECT b.title, b.title_auto, b.game_id, b.notes FROM builds b
         JOIN messages m ON m.id = b.message_id AND m.deleted_at IS NULL
         WHERE b.message_id = ? AND b.link_url = ?",
    )
    .bind(message_id.to_string())
    .bind(&link)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((mut title, mut title_auto, game, notes)) = current else {
        // deleted, or its link changed meanwhile: that link's own fetch fills it in
        return Ok(());
    };
    // The thumbnail's cleanup clock restarts as it's taken (as `embeds::store` does); one cleaned up already is none.
    let mut thumb = got.thumb;
    if let Some(t) = &thumb {
        let touched = sqlx::query("UPDATE media SET created_at = ? WHERE hash = ?")
            .bind(crate::db::now())
            .bind(&t.hash)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if touched == 0 {
            thumb = None;
        }
    }
    if title_auto && let Some(t) = &got.title {
        title = fit_title(t);
        title_auto = false;
    }
    let (game, guessed) = match game {
        Some(g) => (Some(g), false),
        None => match got.title.as_deref().and_then(|t| guess_game(t, &games)) {
            Some(g) => (Some(g.to_string()), true),
            None => (None, false),
        },
    };
    sqlx::query(
        "UPDATE builds SET link_title = ?, link_thumb = ?, link_provider = ?, link_provider_id = ?, title = ?,
                           title_auto = ?, game_id = ?, game_guessed = CASE WHEN ? THEN 1 ELSE game_guessed END
         WHERE message_id = ?",
    )
    .bind(&got.title)
    .bind(thumb.as_ref().map(|t| t.hash.clone()))
    .bind(crate::embeds::provider_str(got.provider))
    .bind(&got.provider_id)
    .bind(&title)
    .bind(title_auto)
    .bind(&game)
    .bind(guessed)
    .bind(message_id.to_string())
    .execute(&mut *tx)
    .await?;
    let game_name: Option<String> = match &game {
        Some(g) => {
            sqlx::query_scalar("SELECT name FROM games WHERE id = ?")
                .bind(g)
                .fetch_optional(&mut *tx)
                .await?
        }
        None => None,
    };
    let content = render_content(
        &title,
        game_name.as_deref(),
        got.title.as_deref(),
        notes.as_deref(),
    );
    sqlx::query("UPDATE messages SET content = ? WHERE id = ?")
        .bind(&content)
        .bind(message_id.to_string())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if let Some(message) = crate::messages::routes::load(&s.db, None, message_id).await? {
        s.hub
            .publish(&s.db, Event::MessageUpdated { message })
            .await;
    }
    Ok(())
}

/// The registry game a page title names: the longest name whose normalised form (`quotes::name_key`) appears in the
/// title's as whole words ("Far Far West" over "West"; "Warframe" not in "Warframes").
pub fn guess_game(title: &str, games: &[(GameId, String)]) -> Option<GameId> {
    let hay = crate::quotes::name_key(title);
    games
        .iter()
        .filter(|(_, name)| {
            let key = crate::quotes::name_key(name);
            !key.is_empty() && contains_words(&hay, &key)
        })
        .max_by_key(|(_, name)| name.chars().count())
        .map(|(id, _)| *id)
}

/// Whether `needle` appears in `hay` with no letter or digit right before or after it.
fn contains_words(hay: &str, needle: &str) -> bool {
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let mut from = 0;
    while let Some(at) = hay[from..].find(needle) {
        let start = from + at;
        let end = start + needle.len();
        if !word(hay[..start].chars().next_back()) && !word(hay[end..].chars().next()) {
            return true;
        }
        // step past this match's first character
        from = start + hay[start..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guess_game_takes_the_longest_whole_word_match() {
        let (west, ffw, wf) = (GameId::new(), GameId::new(), GameId::new());
        let games = vec![
            (west, "West".to_string()),
            (ffw, "Far  Far West".to_string()),
            (wf, "Warframe".to_string()),
        ];
        assert_eq!(guess_game("My FAR FAR WEST build", &games), Some(ffw));
        assert_eq!(guess_game("Best Warframes of 2026", &games), None);
        assert_eq!(guess_game("Warframe: Saryn guide", &games), Some(wf));
        assert_eq!(guess_game("Westworld", &games), None);
        assert_eq!(guess_game("Wild West", &games), Some(west));
        assert_eq!(guess_game("nothing here", &games), None);
    }
}
