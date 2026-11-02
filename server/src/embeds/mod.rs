//! Link embeds: previews of the links in a message (migration 0012), stored per message and link
//! and sent out in the message's `embeds`.

use std::collections::HashMap;

use pulse_protocol::ids::MessageId;
use pulse_protocol::rest::{EmbedKind, EmbedProvider, EmbedThumb, LinkEmbed, Message};
use sqlx::{SqliteConnection, SqlitePool};

use crate::error::AppResult;

/// An `EmbedProvider` as stored (its wire name). `Unknown` is never stored.
pub fn provider_str(p: EmbedProvider) -> &'static str {
    match p {
        EmbedProvider::Youtube => "youtube",
        EmbedProvider::Tiktok => "tiktok",
        EmbedProvider::Spotify => "spotify",
        EmbedProvider::Soundcloud => "soundcloud",
        EmbedProvider::Twitch => "twitch",
        EmbedProvider::Instagram => "instagram",
        EmbedProvider::Generic => "generic",
        EmbedProvider::Unknown => "unknown",
    }
}

/// A stored provider; a name this build doesn't know reads as `Unknown`.
pub fn parse_provider(s: &str) -> EmbedProvider {
    match s {
        "youtube" => EmbedProvider::Youtube,
        "tiktok" => EmbedProvider::Tiktok,
        "spotify" => EmbedProvider::Spotify,
        "soundcloud" => EmbedProvider::Soundcloud,
        "twitch" => EmbedProvider::Twitch,
        "instagram" => EmbedProvider::Instagram,
        "generic" => EmbedProvider::Generic,
        _ => EmbedProvider::Unknown,
    }
}

pub fn kind_str(k: EmbedKind) -> &'static str {
    match k {
        EmbedKind::Card => "card",
        EmbedKind::Player => "player",
    }
}

pub fn parse_kind(s: &str) -> EmbedKind {
    match s {
        "player" => EmbedKind::Player,
        _ => EmbedKind::Card,
    }
}

type EmbedRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    bool,
);

/// Fill `embeds` on every live message in `msgs`: its rows that aren't hidden, in link order.
pub async fn attach(db: &SqlitePool, msgs: &mut [Message]) -> AppResult<()> {
    let ids: Vec<String> = msgs
        .iter()
        .filter(|m| !m.deleted)
        .map(|m| m.id.to_string())
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let mut q = sqlx::QueryBuilder::new(
        "SELECT message_id, url, provider, provider_id, kind, title, description, site_name,
                author, thumb_hash, thumb_width, thumb_height, start_secs, vertical
         FROM link_embeds WHERE hidden = 0 AND message_id IN (",
    );
    let mut list = q.separated(", ");
    for id in &ids {
        list.push_bind(id);
    }
    q.push(") ORDER BY message_id, position");
    let rows: Vec<EmbedRow> = q.build_query_as().fetch_all(db).await?;
    let mut by_msg: HashMap<String, Vec<LinkEmbed>> = HashMap::new();
    for (
        message_id,
        url,
        provider,
        provider_id,
        kind,
        title,
        description,
        site_name,
        author,
        thumb_hash,
        thumb_width,
        thumb_height,
        start_secs,
        vertical,
    ) in rows
    {
        // A thumbnail needs all three; a row written without its size has none.
        let thumb = match (thumb_hash, thumb_width, thumb_height) {
            (Some(hash), Some(w), Some(h)) => Some(EmbedThumb {
                hash,
                width: w as u32,
                height: h as u32,
            }),
            _ => None,
        };
        by_msg.entry(message_id).or_default().push(LinkEmbed {
            url,
            provider: parse_provider(&provider),
            kind: parse_kind(&kind),
            provider_id,
            title,
            description,
            site_name,
            author,
            thumb,
            start: start_secs.map(|s| s as u32),
            vertical,
        });
    }
    for m in msgs.iter_mut().filter(|m| !m.deleted) {
        m.embeds = by_msg.remove(&m.id.to_string()).unwrap_or_default();
    }
    Ok(())
}

/// Part of deleting a message (the soft delete is the caller's): its previews go, so the media
/// cleanup can drop their thumbnails.
pub async fn on_delete(tx: &mut SqliteConnection, id: MessageId) -> AppResult<()> {
    sqlx::query("DELETE FROM link_embeds WHERE message_id = ?")
        .bind(id.to_string())
        .execute(&mut *tx)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for p in [
            EmbedProvider::Youtube,
            EmbedProvider::Tiktok,
            EmbedProvider::Spotify,
            EmbedProvider::Soundcloud,
            EmbedProvider::Twitch,
            EmbedProvider::Instagram,
            EmbedProvider::Generic,
        ] {
            assert_eq!(parse_provider(provider_str(p)), p);
            // The stored name is the wire name.
            assert_eq!(
                serde_json::to_value(p).unwrap(),
                serde_json::json!(provider_str(p))
            );
        }
        assert_eq!(parse_provider("vimeo"), EmbedProvider::Unknown);
        for k in [EmbedKind::Card, EmbedKind::Player] {
            assert_eq!(parse_kind(kind_str(k)), k);
        }
    }
}
