//! Hero and rank names and art (#390). The names come from deadlock-api's asset lists, read at most once a day. Art is
//! fetched by the server, only from [`crate::config::DeadlockConfig::art_hosts`], through the guarded fetcher, re-encoded
//! like a link thumbnail and remembered by its URL (`deadlock_media`, kept from the media sweep): clients get media
//! hashes and never reach a third party.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pulse_protocol::ids::UserId;
use pulse_protocol::rest::{DeadlockRank, EmbedThumb};
use reqwest::Url;

use crate::AppState;
use crate::config::DeadlockConfig;

/// How long the hero and rank lists are trusted.
pub const CATALOG_TTL: Duration = Duration::from_secs(24 * 3600);
/// How soon a failed read of them is tried again.
pub const CATALOG_RETRY: Duration = Duration::from_secs(600);

/// One hero: its name and art URLs (card portrait, small icon).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hero {
    pub name: String,
    pub card: Option<String>,
    pub icon: Option<String>,
}

/// One rank tier: its name and art URLs by name (`small`, `small_subrank3`, …).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tier {
    pub name: String,
    pub images: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    pub heroes: HashMap<u32, Hero>,
    pub tiers: HashMap<u32, Tier>,
}

impl Catalog {
    pub fn hero_name(&self, id: u32) -> String {
        self.heroes
            .get(&id)
            .map(|h| h.name.clone())
            .unwrap_or_else(|| format!("Hero {id}"))
    }

    /// Tier `tier`'s badge image URL for `subrank` (1–6), else its plain one.
    pub fn rank_image(&self, tier: u32, subrank: u32) -> Option<String> {
        let t = self.tiers.get(&tier)?;
        let pick = |k: &str| t.images.get(k).cloned();
        if (1..=6).contains(&subrank)
            && let Some(u) = pick(&format!("small_subrank{subrank}_webp"))
                .or_else(|| pick(&format!("small_subrank{subrank}")))
        {
            return Some(u);
        }
        pick("small_webp")
            .or_else(|| pick("small"))
            .or_else(|| pick("large_webp"))
            .or_else(|| pick("large"))
    }

    pub fn tier_name(&self, tier: u32) -> String {
        self.tiers
            .get(&tier)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| {
                if tier == 0 {
                    "Unranked".into()
                } else {
                    format!("Rank {tier}")
                }
            })
    }
}

struct State {
    catalog: Option<Arc<Catalog>>,
    fetched: Option<Instant>,
    tried: Option<Instant>,
}

pub struct Assets {
    hosts: Vec<String>,
    state: Mutex<State>,
    refresh: tokio::sync::Mutex<()>,
}

impl Assets {
    pub fn new(cfg: &DeadlockConfig) -> Self {
        Self {
            hosts: cfg
                .art_hosts
                .iter()
                .map(|h| h.to_ascii_lowercase())
                .collect(),
            state: Mutex::new(State {
                catalog: None,
                fetched: None,
                tried: None,
            }),
            refresh: tokio::sync::Mutex::new(()),
        }
    }

    /// Whether art may be fetched from `url`: http(s), on an allowed host.
    pub fn allowed(&self, url: &str) -> bool {
        let Ok(u) = Url::parse(url) else {
            return false;
        };
        matches!(u.scheme(), "https" | "http")
            && u.host_str()
                .is_some_and(|h| self.hosts.iter().any(|a| a.eq_ignore_ascii_case(h)))
    }

    fn cached(&self) -> (Option<Arc<Catalog>>, bool) {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = st.fetched.is_some_and(|t| t.elapsed() < CATALOG_TTL)
            || st.tried.is_some_and(|t| t.elapsed() < CATALOG_RETRY);
        (st.catalog.clone(), fresh)
    }
}

/// The hero and rank lists: cached for a day; on a failure whatever's cached (perhaps nothing).
pub async fn catalog(s: &AppState) -> Option<Arc<Catalog>> {
    let a = s.deadlock.assets();
    let (cat, fresh) = a.cached();
    if fresh {
        return cat;
    }
    let _one = a.refresh.lock().await;
    let (cat, fresh) = a.cached();
    if fresh {
        return cat;
    }
    let api = s.deadlock.api();
    let got = match (api.heroes().await, api.rank_assets().await) {
        (Ok(heroes), Ok(ranks)) => Some(build(heroes, ranks)),
        _ => None,
    };
    let mut st = a.state.lock().unwrap_or_else(|e| e.into_inner());
    st.tried = Some(Instant::now());
    if let Some(c) = got {
        st.catalog = Some(Arc::new(c));
        st.fetched = Some(Instant::now());
    }
    st.catalog.clone()
}

fn build(heroes: Vec<super::api::HeroAsset>, ranks: Vec<super::api::RankAsset>) -> Catalog {
    let flat = |m: HashMap<String, Option<String>>| -> HashMap<String, String> {
        m.into_iter()
            .filter_map(|(k, v)| Some((k, v.filter(|u| !u.is_empty())?)))
            .collect()
    };
    Catalog {
        heroes: heroes
            .into_iter()
            .filter_map(|h| {
                let images = flat(h.images);
                let name = crate::steam::one_line(&h.name, 60)?;
                Some((
                    h.id,
                    Hero {
                        name,
                        card: images
                            .get("icon_hero_card_webp")
                            .or_else(|| images.get("icon_hero_card"))
                            .cloned(),
                        icon: images
                            .get("icon_image_small_webp")
                            .or_else(|| images.get("icon_image_small"))
                            .cloned(),
                    },
                ))
            })
            .collect(),
        tiers: ranks
            .into_iter()
            .filter_map(|r| {
                Some((
                    r.tier,
                    Tier {
                        name: crate::steam::one_line(&r.name, 40)?,
                        images: flat(r.images),
                    },
                ))
            })
            .collect(),
    }
}

/// The picture at `url` as the server's copy: remembered by URL, else fetched (only from an allowed host) for `by`.
pub async fn art(s: &AppState, by: UserId, url: &str) -> Option<EmbedThumb> {
    if !s.deadlock.assets().allowed(url) {
        tracing::debug!("Deadlock art from a host that isn't allowed: skipped");
        return None;
    }
    let known: Option<(String, i64, i64)> =
        sqlx::query_as("SELECT hash, width, height FROM deadlock_media WHERE url = ?")
            .bind(url)
            .fetch_optional(&s.db)
            .await
            .ok()
            .flatten();
    if let Some((hash, w, h)) = known {
        let t = EmbedThumb {
            hash,
            width: w.max(0) as u32,
            height: h.max(0) as u32,
        };
        // still on disk (it's kept from the sweep, but a restore may have lost it)
        if let Ok(Some(t)) = crate::embeds::keep_thumb(&s.db, t).await {
            return Some(t);
        }
    }
    let t = s
        .embeds
        .remote_thumb(&s.db, &s.cfg.media_dir, by, url)
        .await?;
    if let Err(e) = sqlx::query(
        "INSERT INTO deadlock_media (url, hash, width, height) VALUES (?, ?, ?, ?)
         ON CONFLICT (url) DO UPDATE SET hash = excluded.hash, width = excluded.width, height = excluded.height",
    )
    .bind(url)
    .bind(&t.hash)
    .bind(t.width)
    .bind(t.height)
    .execute(&s.db)
    .await
    {
        tracing::warn!(error = %e, "Deadlock art not kept");
    }
    Some(t)
}

/// A hero's card art (`card`) or small icon.
pub async fn hero_art(
    s: &AppState,
    by: UserId,
    cat: Option<&Catalog>,
    hero: u32,
    card: bool,
) -> Option<EmbedThumb> {
    let h = cat?.heroes.get(&hero)?;
    let url = if card {
        h.card.as_ref().or(h.icon.as_ref())
    } else {
        h.icon.as_ref()
    }?;
    art(s, by, url).await
}

/// Rank `badge` (`tier * 10 + subrank`) with its name and image.
pub async fn rank(s: &AppState, by: UserId, cat: Option<&Catalog>, badge: u32) -> DeadlockRank {
    let (tier, subrank) = (badge / 10, badge % 10);
    let image = match cat.and_then(|c| c.rank_image(tier, subrank)) {
        Some(url) => art(s, by, &url).await,
        None => None,
    };
    DeadlockRank {
        badge,
        tier,
        subrank,
        name: cat
            .map(|c| c.tier_name(tier))
            .unwrap_or_else(|| Catalog::default().tier_name(tier)),
        image,
    }
}

/// "Oracle 3", "Unranked".
pub fn rank_text(r: &DeadlockRank) -> String {
    if r.tier == 0 || r.subrank == 0 {
        r.name.clone()
    } else {
        format!("{} {}", r.name, r.subrank)
    }
}

/// A badge as a position on one ladder (6 subranks a tier), for counting how far someone moved.
pub fn ladder(badge: u32) -> i32 {
    (badge / 10) as i32 * 6 + (badge % 10) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cat() -> Catalog {
        let images = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        Catalog {
            heroes: [(
                1,
                Hero {
                    name: "Infernus".into(),
                    card: Some("https://art.test/inferno_card.webp".into()),
                    icon: None,
                },
            )]
            .into(),
            tiers: [
                (
                    8,
                    Tier {
                        name: "Oracle".into(),
                        images: images(&[
                            ("small_webp", "https://art.test/r8.webp"),
                            ("small_subrank3_webp", "https://art.test/r8_3.webp"),
                        ]),
                    },
                ),
                (
                    0,
                    Tier {
                        name: "Obscurus".into(),
                        images: images(&[("small", "https://art.test/r0.png")]),
                    },
                ),
            ]
            .into(),
        }
    }

    #[test]
    fn names_and_rank_images() {
        let c = cat();
        assert_eq!(c.hero_name(1), "Infernus");
        assert_eq!(c.hero_name(99), "Hero 99");
        assert_eq!(
            c.rank_image(8, 3).as_deref(),
            Some("https://art.test/r8_3.webp")
        );
        assert_eq!(
            c.rank_image(8, 5).as_deref(),
            Some("https://art.test/r8.webp"),
            "no subrank art: the tier's"
        );
        assert_eq!(
            c.rank_image(0, 0).as_deref(),
            Some("https://art.test/r0.png")
        );
        assert_eq!(c.rank_image(5, 1), None);
        assert_eq!(c.tier_name(8), "Oracle");
        assert_eq!(Catalog::default().tier_name(0), "Unranked");
    }

    #[test]
    fn only_allowed_hosts() {
        let a = Assets::new(&DeadlockConfig::default());
        assert!(a.allowed("https://assets-bucket.deadlock-api.com/x/inferno_card.png"));
        assert!(a.allowed("https://API.deadlock-api.com/v1/assets/ranks/5/1/image"));
        assert!(!a.allowed("https://evil.example/x.png"));
        assert!(!a.allowed("https://deadlock-api.com.evil.example/x.png"));
        assert!(!a.allowed("file:///etc/passwd"));
        assert!(!a.allowed("not a url"));
    }

    #[test]
    fn rank_text_and_ladder() {
        let r = |tier, subrank, name: &str| DeadlockRank {
            badge: tier * 10 + subrank,
            tier,
            subrank,
            name: name.into(),
            image: None,
        };
        assert_eq!(rank_text(&r(8, 3, "Oracle")), "Oracle 3");
        assert_eq!(rank_text(&r(0, 0, "Obscurus")), "Obscurus");
        assert_eq!(ladder(83) - ladder(76), 3);
        assert_eq!(ladder(81) - ladder(86), -5);
    }
}
