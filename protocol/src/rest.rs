//! REST request/response bodies.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{
    AttachmentId, BookmarkId, CallId, CategoryId, ChannelId, ClipUploadId, DmCategoryId, GameId,
    LineId, MessageId, QotyEventId, ReminderId, ScheduledId, ServerId, UserId,
};

/// A `Debug` that names every field but prints the secret ones as `<redacted>`, for types that
/// carry passwords, codes, tokens or tickets: debug output can end up in a log. Exported for the
/// server's and client's own types (`pulse_protocol::redacting_debug!`).
#[macro_export]
macro_rules! redacting_debug {
    ($t:ident { $($shown:ident),* ; $($hidden:ident),* }) => {
        impl std::fmt::Debug for $t {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                // Exhaustive: a field added to the type and not listed here won't compile.
                let $t { $($shown,)* $($hidden: _,)* } = self;
                f.debug_struct(stringify!($t))
                    $(.field(stringify!($shown), $shown))*
                    $(.field(stringify!($hidden), &"<redacted>"))*
                    .finish()
            }
        }
    };
}

#[derive(Clone, Serialize, Deserialize, TS)]
pub struct RegisterRequest {
    /// A sign-up code, or a server owner's invite code (#80), which also puts the new account in that server.
    pub invite_code: String,
    pub username: String,
    pub password: String,
    /// Always empty from the app (#80).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub website: Option<String>,
}
redacting_debug!(RegisterRequest { username ; invite_code, password, website });

#[derive(Clone, Serialize, Deserialize, TS)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}
redacting_debug!(LoginRequest { username ; password });

#[derive(Clone, Serialize, Deserialize, TS)]
pub struct SessionResponse {
    pub token: String,
    pub user: User,
}
redacting_debug!(SessionResponse { user ; token });

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct User {
    pub id: UserId,
    /// The login name. Shown only on the profile card; everywhere else shows a display name or
    /// nickname.
    pub username: String,
    /// How the person's name looks by default (spec name layers: not unique, 1–32 characters).
    /// Absent (empty) from older servers: show the username then.
    #[serde(default)]
    pub display_name: String,
    pub avatar_hash: Option<String>,
    /// The first-frame still of an animated avatar (`None` for a still one).
    #[serde(default)]
    pub avatar_still: Option<String>,
    #[serde(default)]
    pub style: NameStyle,
}

/// How a name is drawn: a bundled font and 0 (default), 1 (solid) or 2 (gradient, first to
/// second) `#rrggbb` colours, and a name effect (#468).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NameStyle {
    pub font: Option<String>,
    pub colors: Vec<String>,
    /// One of the name effects (`neon`, `toon`, `pop`, `gummy`, `prism`, `shimmer`, `glitch`,
    /// `ember`), or none. A string, not an enum, so an older client reading an effect it doesn't
    /// know still reads the user (and draws the name without it). Absent from older servers and
    /// clients: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub effect: Option<String>,
}

/// The parts of a profile card that are not part of [`User`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CardProfile {
    pub banner_hash: Option<String>,
    pub banner_still: Option<String>,
    pub theme_top: String,
    pub theme_bottom: String,
    pub pronouns: Option<String>,
    pub bio: Option<String>,
    /// The card glows in its two colours (#156): the owner's choice, the same for everyone who sees
    /// it. Absent from older servers, which always drew the glow: on.
    #[serde(default = "yes")]
    #[ts(as = "Option<bool>", optional)]
    pub glow: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ProfileStats {
    #[ts(type = "number")]
    pub voice_seconds: u64,
    pub member_since: String,
    pub top_reaction: Option<String>,
    pub times_renamed: u32,
    /// Steam account completion rate (#379), 0 to 100: the average completion over games with at least one
    /// achievement unlocked. Only while they share Steam achievements and once there's a game to count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub steam_completion: Option<f32>,
    /// Steam games they've completed 100 % (#379), with `steam_completion`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub steam_perfect_games: Option<u32>,
    /// Minutes played in Steam games over the last two weeks (#383). Only while they share Steam hours and once
    /// the server has read them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub steam_recent_minutes: Option<u32>,
}

/// How rare a badge (or one tier of it) is, from common up to mythical.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS,
)]
#[serde(rename_all = "snake_case")]
pub enum BadgeRarity {
    #[default]
    Common,
    Uncommon,
    Rare,
    Epic,
    Legendary,
    Mythical,
}

/// A badge someone holds. `earned_at` is when the tier held was reached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Badge {
    pub key: String,
    pub earned_at: String,
    /// First to reach the tier held.
    pub first_earner: bool,
    /// The tier held (1 for a badge without tiers).
    #[serde(default)]
    pub tier: u8,
    /// The rarity of the tier held.
    #[serde(default)]
    pub rarity: BadgeRarity,
    /// Every tier of this badge its holder reached first, ascending.
    #[serde(default)]
    pub first_tiers: Vec<u8>,
    /// A stacking badge's count (×2, ×3); 0 for the others.
    #[serde(default)]
    pub count: u32,
    /// A hidden badge shown masked ("???", rarity only).
    #[serde(default)]
    pub hidden: bool,
    /// Its holder has opened their trophy case since earning it (always true on others' profiles).
    #[serde(default)]
    pub seen: bool,
}

/// How a badge's thresholds and progress read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum BadgeUnit {
    #[default]
    Count,
    Hours,
}

/// One tier of a badge in the catalogue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct BadgeTier {
    /// What reaches it, in the badge's unit (0 when masked). Combo's is its message count.
    #[ts(type = "number")]
    pub threshold: u64,
    pub rarity: BadgeRarity,
    /// How many people have reached it.
    pub holders: u32,
}

/// A badge as the viewer may see it. A hidden badge the viewer doesn't hold is masked: `key` is
/// `hidden:<n>`, `name` is "???", no description, a placeholder icon and thresholds of 0.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct BadgeDef {
    pub key: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    #[serde(default)]
    pub hidden: bool,
    /// Only ever given by hand with the admin command.
    #[serde(default)]
    pub hand_given: bool,
    /// A hand-given badge only one person can ever have (#239): its share reads "1 of 1".
    #[serde(default)]
    pub one_of_a_kind: bool,
    #[serde(default)]
    pub stacking: bool,
    #[serde(default)]
    pub unit: BadgeUnit,
    #[serde(default)]
    pub tiers: Vec<BadgeTier>,
    /// The viewer's own value in the badge's unit; `None` when masked, hand-given or not one
    /// number (Combo: see `parts`).
    #[serde(default)]
    #[ts(type = "number | null")]
    pub progress: Option<u64>,
    /// Combo's progress (#191): each of its stats against the next tier the viewer hasn't
    /// reached, the viewer's own. Empty for other badges, when masked and once all are reached.
    #[serde(default)]
    pub parts: Vec<BadgeProgressPart>,
    /// The badge's picture (#225), a media hash (a still WebP), drawn inside its rarity ring
    /// instead of `icon`; `None` (and never when masked) for a badge that keeps its emoji.
    #[serde(default)]
    pub art: Option<String>,
    /// A hidden badge's riddle (#211), masked or not: the trophy case shows it on your own "???"
    /// tile until you earn the badge. `None` for every other badge.
    #[serde(default)]
    pub hint: Option<String>,
}

/// One stat of a badge that needs several at once (Combo, #191), toward its next tier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct BadgeProgressPart {
    /// What it counts, e.g. "Hours in voice".
    pub label: String,
    /// The viewer's value (whole hours for time).
    #[ts(type = "number")]
    pub value: u64,
    /// What the next tier needs, in the same unit.
    #[ts(type = "number")]
    pub target: u64,
}

/// `GET /badges`: every badge in trophy-case order, and how many people there are to have them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct BadgeCatalogue {
    pub people: u32,
    pub badges: Vec<BadgeDef>,
}

/// `PUT /me/badges/showcase`: up to 5 distinct held badge keys, in order; empty clears the
/// choice (back to the rarest five).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SetShowcaseRequest {
    pub keys: Vec<String>,
}

/// One badge, at the tier the trophy case showed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SeenBadge {
    pub key: String,
    pub tier: u8,
}

/// `POST /me/badges/seen`: the badges your trophy case showed [NEW]. Only those, at those tiers,
/// stop being [NEW]: one earned (or a tier reached) after the case loaded stays [NEW] (#129).
/// No body at all (older clients) marks everything seen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MarkBadgesSeenRequest {
    pub badges: Vec<SeenBadge>,
}

/// `POST /me/badges/egg`: the client found a hidden egg (`"logo"`: the logo-click one).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EggRequest {
    pub egg: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NameArchiveEntry {
    pub server_id: ServerId,
    pub name: String,
    pub changed_by: UserId,
    pub changed_at: String,
}

/// `GET /users/{id}/profile`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Profile {
    pub user: User,
    pub card: CardProfile,
    pub stats: ProfileStats,
    pub badges: Vec<Badge>,
    pub name_archive: Vec<NameArchiveEntry>,
    /// Up to 5 badge keys shown on the card, in order, as they appear in `badges`.
    #[serde(default)]
    pub showcase: Vec<String>,
    /// The owner saved a showcase of their own; false while it's the default (the rarest five).
    #[serde(default)]
    pub showcase_chosen: bool,
}

/// `PUT /me/profile`: the whole editable profile at once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct UpdateProfileRequest {
    pub avatar_hash: Option<String>,
    pub banner_hash: Option<String>,
    pub theme_top: String,
    pub theme_bottom: String,
    pub style: NameStyle,
    pub pronouns: Option<String>,
    pub bio: Option<String>,
    /// The new display name; absent keeps the current one (older clients don't send it).
    #[serde(default)]
    pub display_name: Option<String>,
    /// Whether the card glows (#156); absent keeps the current setting (older clients don't send it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub glow: Option<bool>,
}

/// `PUT /servers/{id}/members/{user}/nickname` and `PUT /me/private-nicknames/{user}`: `None`
/// (or blank) resets it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SetNicknameRequest {
    #[serde(default)]
    pub nickname: Option<String>,
}

/// `PUT /me/server-order` (#185): every server in the order you want them in your server rail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ServerOrderRequest {
    pub server_ids: Vec<ServerId>,
}

/// One of your own DM categories (#305): a heading in your conversation list that only you see.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DmCategory {
    pub id: DmCategoryId,
    pub name: String,
    /// Folded shut in your list (on every device).
    #[serde(default)]
    pub collapsed: bool,
}

/// Where one of your conversations sits in your list (#305): pinned to the top, in one of your categories, or both
/// (a pinned one shows under Pinned). Conversations with neither have no placement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DmPlacement {
    pub channel_id: ChannelId,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub category_id: Option<DmCategoryId>,
}

/// Your conversation list's layout (#305): your categories in order and your placements. Yours only, on every device.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DmLayout {
    pub categories: Vec<DmCategory>,
    pub placements: Vec<DmPlacement>,
}

/// `POST /me/dm-categories` (#305): a new category, last in your list.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateDmCategoryRequest {
    pub name: String,
}

/// `PATCH /me/dm-categories/{id}` (#305): rename it, fold it open or shut, or move it (#419). Absent fields stay as
/// they are.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct UpdateDmCategoryRequest {
    #[serde(default)]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub collapsed: Option<bool>,
    /// Where it goes among your categories, from 0 (past the end: last); the others keep their order around it.
    #[serde(default)]
    #[ts(optional)]
    pub position: Option<u32>,
}

/// `PUT /me/dm-placements/{channel_id}` (#305): pin or unpin a conversation and put it in a category (or none).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct DmPlacementRequest {
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub category_id: Option<DmCategoryId>,
}

/// Limits on DM categories (#305).
pub mod dm_layout_limits {
    /// Characters in a category's name, after trimming.
    pub const NAME_CHARS: usize = 32;
    /// Categories one person may have.
    pub const MAX_CATEGORIES: usize = 30;
    /// Names no category may have (ignoring case, and "..." for "…"): the Move to category flyout's own entries sit
    /// beside the categories, and one named like them couldn't be told apart (#419).
    pub const RESERVED_NAMES: [&str; 2] = ["New category…", "Remove from category"];

    /// Whether `name` (trimmed) is one of [`RESERVED_NAMES`].
    pub fn is_reserved(name: &str) -> bool {
        let n = name.trim().replace("...", "…").to_lowercase();
        RESERVED_NAMES.iter().any(|r| r.to_lowercase() == n)
    }
}

/// Someone whose messages don't alert you (#310): no message sound and no desktop notification from them, everywhere
/// or only in DMs and groups (`dms_only`), until `until` (RFC 3339; `None`: until you turn it off). Their messages
/// still arrive and count as unread (and as mentions) as usual. Yours only; they're never told.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PersonDnd {
    pub user_id: UserId,
    #[serde(default)]
    pub dms_only: bool,
    #[serde(default)]
    pub until: Option<String>,
}

/// `PUT /me/person-dnd/{user_id}` (#310): turn Do Not Disturb on for someone (or change it).
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct PersonDndRequest {
    #[serde(default)]
    pub dms_only: bool,
    /// When it ends (RFC 3339, in the future); `None`: until turned off.
    #[serde(default)]
    pub until: Option<String>,
}

/// A name you gave someone that only you see. Only ever sent to its owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PrivateNickname {
    pub user_id: UserId,
    pub nickname: String,
}

/// One server-nickname change in the Name Archive; `name: None` is a reset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NameChange {
    pub name: Option<String>,
    pub changed_by: UserId,
    pub changed_at: String,
}

/// `GET /servers/{id}/members/{user}/names`: a page of someone's Name Archive in one server,
/// newest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NameArchivePage {
    pub entries: Vec<NameChange>,
    /// Every change in this server (resets included): "Renamed N times".
    pub total: u32,
    /// Pass as `cursor` for the next page (opaque); `None` once there is nothing older.
    pub next_cursor: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, TS)]
pub struct InviteResponse {
    pub code: String,
}
redacting_debug!(InviteResponse { ; code });

/// `POST /auth/logout-others`: how many of the caller's other sessions were signed out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct LogoutOthersResponse {
    pub revoked: u32,
}

/// `POST /auth/password`: change the caller's password. `code` (a TOTP or recovery code) is
/// needed only when the account has two-factor on. Success signs out every other session.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
    #[serde(default)]
    pub code: Option<String>,
}
redacting_debug!(ChangePasswordRequest { ; current_password, new_password, code });

/// `GET /auth/2fa`: the caller's own two-factor state. `available` is false when the server has
/// no encryption key, so the UI can say so instead of offering a setup that would fail.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TwoFactorStatus {
    pub enabled: bool,
    pub available: bool,
    pub recovery_codes_left: u32,
}

/// `POST /auth/2fa/setup`: starting (or restarting) setup needs the account password.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct TwoFactorSetupRequest {
    pub password: String,
}
redacting_debug!(TwoFactorSetupRequest { ; password });

/// The pending secret as base32 text and as the `otpauth://` URI a QR code carries.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct TwoFactorSetupResponse {
    pub secret: String,
    pub otpauth_uri: String,
}
redacting_debug!(TwoFactorSetupResponse { ; secret, otpauth_uri });

/// A TOTP code (or, where allowed, a recovery code).
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct TwoFactorCodeRequest {
    pub code: String,
}
redacting_debug!(TwoFactorCodeRequest { ; code });

/// `POST /auth/2fa/enable`: the recovery codes (shown once) and how many other sessions were
/// signed out.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct TwoFactorEnabledResponse {
    pub recovery_codes: Vec<String>,
    pub signed_out: u32,
}
redacting_debug!(TwoFactorEnabledResponse { signed_out ; recovery_codes });

/// `POST /auth/2fa/recovery-codes`: the new set, shown once.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct RecoveryCodesResponse {
    pub recovery_codes: Vec<String>,
}
redacting_debug!(RecoveryCodesResponse { ; recovery_codes });

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Server {
    pub id: ServerId,
    pub name: String,
    pub icon_hash: Option<String>,
    /// Who owns it (its creator): only they may delete or restore it (#184). Absent from older
    /// servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub owner_id: Option<UserId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Text,
    Voice,
    Dm,
    Group,
    /// A server channel for quotes: posts are quotes, replies live in threads.
    Quotes,
    /// A server channel for video clips: posts are clips, replies live in threads.
    Clips,
}

impl ChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
            Self::Dm => "dm",
            Self::Group => "group",
            Self::Quotes => "quotes",
            Self::Clips => "clips",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "text" => Self::Text,
            "voice" => Self::Voice,
            "dm" => Self::Dm,
            "group" => Self::Group,
            "quotes" => Self::Quotes,
            "clips" => Self::Clips,
            _ => return None,
        })
    }

    /// DMs and groups are private: only their members may see anything about them.
    pub fn is_private(self) -> bool {
        matches!(self, Self::Dm | Self::Group)
    }

    /// Server kinds that hold nothing but messages (no call): text, quotes and clips. A voice
    /// channel isn't one, though it has a chat too ([`Self::has_chat`]).
    pub fn is_server_chat(self) -> bool {
        matches!(self, Self::Text | Self::Quotes | Self::Clips)
    }

    /// Server kinds with a chat its members may read and post in: the message-only kinds, and a
    /// voice channel's chat beside its call (#226).
    pub fn has_chat(self) -> bool {
        self.is_server_chat() || self == Self::Voice
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Channel {
    pub id: ChannelId,
    pub server_id: Option<ServerId>,
    pub kind: ChannelKind,
    pub name: Option<String>,
    /// Its order inside its category (or among the uncategorised channels), lowest first.
    #[ts(type = "number")]
    pub position: i64,
    /// The server category it sits in (#54). Absent: uncategorised (and always for DMs and groups).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub category_id: Option<CategoryId>,
    /// A group's own icon (#56): a picture from the avatar upload. Absent: the members' faces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub icon_hash: Option<String>,
    /// That icon's still, when it's animated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub icon_still: Option<String>,
    /// A voice channel's audio bitrate in kbps (#75), one of [`CHANNEL_BITRATES_KBPS`]: everyone
    /// publishing there sends at it. Absent: Default (64 kbps), which older clients always send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bitrate: Option<u32>,
    /// When its owner archived it (#303): a text channel that is read-only (nobody posts, reacts or edits there) and
    /// sits in the list's collapsed Archived group. Absent: not archived (and from older servers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub archived_at: Option<String>,
}

/// The bitrates a voice channel may set (#75), in kbps, above Default's 64. 256 is as much as the
/// mic, published as the app publishes it, really sends (measured in
/// client/src-tauri/tests/voice_bitrate_it.rs: 384 and 510 send ~260).
pub const CHANNEL_BITRATES_KBPS: [u32; 4] = [96, 128, 192, 256];

/// `PUT /channels/{id}/bitrate`: a voice channel's bitrate (#75), one of
/// [`CHANNEL_BITRATES_KBPS`]; `None` = back to Default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SetChannelBitrateRequest {
    pub bitrate: Option<u32>,
}

/// A channel in its server's Recently deleted (`GET /servers/{id}/deleted-channels`), with what
/// the asking member may do with it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeletedChannel {
    pub channel: Channel,
    /// Who deleted it.
    pub deleted_by: Option<UserId>,
    pub deleted_at: String,
    /// When the sweeper purges it (unless `held`).
    pub purge_at: String,
    /// The server owner and whoever deleted it may restore it.
    #[serde(default)]
    pub can_restore: bool,
    /// Only the server owner may purge it now.
    #[serde(default)]
    pub can_purge: bool,
    /// Set while it can't be deleted for good, saying why: a quotes channel holding a Quote of the
    /// Year winner, or with a quote up in a live vote. The sweeper keeps it past `purge_at`.
    #[serde(default)]
    pub held: Option<String>,
}

/// `DELETE /servers/{id}` (#184): the server's name, typed by its owner to confirm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeleteServerRequest {
    pub name: String,
}

/// One of your servers in your Recently deleted (`GET /servers/deleted`, #184).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeletedServer {
    pub server: Server,
    pub deleted_at: String,
    /// When the sweeper purges it, with everything in it.
    pub purge_at: String,
    /// False once it is due to be purged (it may be going right now).
    pub can_restore: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateServerRequest {
    pub name: String,
}

/// `PUT /servers/{id}/owner` (#418): its owner hands the server to another of its members.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct TransferServerRequest {
    pub user_id: UserId,
}

/// `GET /servers/{id}/blocked` (#418): someone its owner removed three times within 48 hours, who can't rejoin until
/// the owner allows them back (`DELETE /servers/{id}/blocked/{user}`). The owner's view only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ServerBlock {
    pub user_id: UserId,
    /// When the block began (the third removal; RFC 3339 UTC).
    pub blocked_at: String,
}

/// `POST /servers/{id}/invites` (#80): make an invite link, or a direct invite for one person (`for_user`). Any member
/// may; only the owner may make one that never expires or has no use limit.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct CreateServerInviteRequest {
    /// A direct invite: only this person can use it, once. It also arrives as a card in your DM with them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub for_user: Option<UserId>,
    /// Seconds until it expires; `0` = never (the owner only). Absent: one day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub max_age_secs: Option<i64>,
    /// How many people can join with it; `0` = no limit (the owner only). Absent: one. A direct invite is always one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub max_uses: Option<i64>,
    /// The join button's text on the card, up to [`INVITE_JOIN_TEXT_CHARS`] characters on one line. Absent or blank:
    /// the client's default ("Join <server>").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub join_text: Option<String>,
}

/// The longest custom join button text an invite may carry.
pub const INVITE_JOIN_TEXT_CHARS: usize = 32;

/// Whether an invite still lets someone in, and if not, why (#80). In an invite's preview it is as the viewer sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum InviteState {
    Valid,
    Expired,
    UsedUp,
    Revoked,
    /// A direct invite for someone else (previews only).
    ForSomeoneElse,
    /// The viewer is already in the server (previews only).
    AlreadyMember,
    /// The viewer is banned from the server (previews only).
    Banned,
}

/// An invite as its maker and the server's owner see it (`GET /servers/{id}/invites`, #80).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ServerInvite {
    pub code: String,
    pub server_id: ServerId,
    pub created_by: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub for_user: Option<UserId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub join_text: Option<String>,
    pub created_at: String,
    /// Absent: never expires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub expires_at: Option<String>,
    /// Absent: no limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub max_uses: Option<i64>,
    pub uses: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub revoked_at: Option<String>,
    /// Valid, expired, used up or revoked.
    pub state: InviteState,
}

/// `GET /invites/{code}` (#80): what an invite card shows. Nothing from inside the server but the faces of people the
/// viewer already knows from another server or a DM.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct InvitePreview {
    pub code: String,
    pub server_id: ServerId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub icon_hash: Option<String>,
    /// The server's banner, once servers have one (always absent for now).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub banner_hash: Option<String>,
    pub member_count: i64,
    /// Members who look online to the viewer.
    pub online_count: i64,
    /// Up to three members the viewer shares another server or a DM with.
    pub faces: Vec<User>,
    pub inviter_id: UserId,
    pub inviter_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub join_text: Option<String>,
    /// For a direct invite: whose it is (the card says "For <name> only" to anyone else).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub for_user: Option<UserId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub for_user_name: Option<String>,
    pub state: InviteState,
}

/// A direct invite's card in a DM (#80): the code to preview (`GET /invites/{code}`) and accept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MessageServerInvite {
    pub code: String,
    pub server_id: ServerId,
}

/// `GET /servers/{id}/bans` (#80): someone the owner banned; their invites are refused. The owner's view only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ServerBan {
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub banned_by: Option<UserId>,
    pub banned_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateChannelRequest {
    pub kind: ChannelKind,
    pub name: String,
    /// The category to make it in, at the bottom (#54). Absent: uncategorised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub category_id: Option<CategoryId>,
}

/// A named group of a server's channels, of any kind, shown as a heading in the list (#54).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Category {
    pub id: CategoryId,
    pub server_id: ServerId,
    /// As typed; the list shows it upper-cased.
    pub name: String,
    /// Its order among the server's categories, lowest first.
    #[ts(type = "number")]
    pub position: i64,
    /// The kind a default category was made for (by the update or a new server), so a section
    /// someone had collapsed before categories stays collapsed. Absent for categories people make.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub default_kind: Option<ChannelKind>,
}

/// Where one channel sits: its category (none: uncategorised) and its order there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ChannelPlacement {
    pub channel_id: ChannelId,
    pub category_id: Option<CategoryId>,
    #[ts(type = "number")]
    pub position: i64,
}

/// A server's whole channel list layout: its categories in order and where every live channel
/// sits. `revision` goes up with every change, so a save based on an older one is refused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ServerLayout {
    pub server_id: ServerId,
    #[ts(type = "number")]
    pub revision: i64,
    /// In order.
    pub categories: Vec<Category>,
    /// Every live channel's placement. Empty in `Ready`, where the channels carry their own.
    #[serde(default)]
    pub channels: Vec<ChannelPlacement>,
}

/// `POST /servers/{id}/categories`: a new category, at the bottom.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateCategoryRequest {
    pub name: String,
}

/// `PATCH /categories/{id}`: a new name.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RenameCategoryRequest {
    pub name: String,
}

/// `PUT /servers/{id}/layout`: the whole new order of a server's list, based on the layout
/// `revision` the client last saw.
/// It must name every live channel exactly once and every category, or it is refused.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SaveLayoutRequest {
    #[ts(type = "number")]
    pub revision: i64,
    /// The channels outside any category, in order.
    pub uncategorised: Vec<ChannelId>,
    /// The categories in order, each with its channels in order.
    pub categories: Vec<LayoutCategory>,
}

/// One category in a [`SaveLayoutRequest`] and its channels, in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LayoutCategory {
    pub id: CategoryId,
    pub channels: Vec<ChannelId>,
}

/// Your own order of a server's channel list (#304), drawn over the server's layout for you only: the channels
/// outside any category, then the categories, each with its channels, all in your order. Channels and categories it
/// doesn't name (new since) follow the ones it does, in the server's order; ones gone since are skipped. Archived
/// channels are never in it. Yours only, on every device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ChannelOrder {
    pub server_id: ServerId,
    pub uncategorised: Vec<ChannelId>,
    pub categories: Vec<LayoutCategory>,
}

/// `PUT /me/channel-orders/{server_id}` (#304): your own order of that server's list (replacing any you had).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ChannelOrderRequest {
    pub uncategorised: Vec<ChannelId>,
    pub categories: Vec<LayoutCategory>,
}

/// One other user → a 1:1 `dm` (reused if it exists); 2..=9 others → a new `group`; only yourself →
/// your notes to self (#308), a `dm` with you as its only member (made on first open, then reused).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateDmRequest {
    pub user_ids: Vec<UserId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Member {
    pub user: User,
    pub nickname: Option<String>,
    /// The id of the sound from this server's soundboard that plays when they join one of its
    /// voice channels (#121). Absent from older servers, and when none is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub join_sound: Option<String>,
    /// When they joined the server (RFC 3339 UTC; the latest join, after a leave). Absent from older servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub joined_at: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Normal,
    System,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Message {
    pub id: MessageId,
    pub channel_id: ChannelId,
    pub author_id: Option<UserId>,
    pub kind: MessageKind,
    pub content: String,
    pub reply_to_id: Option<MessageId>,
    pub created_at: String,
    pub edited_at: Option<String>,
    pub deleted: bool,
    /// Users this message @mentions (only people who can see the channel).
    #[serde(default)]
    pub mentions: Vec<UserId>,
    /// In first-reacted order. `me` is per caller; in gateway events it is always false.
    #[serde(default)]
    pub reactions: Vec<ReactionSummary>,
    /// Set on a reply: the quote (a message of a quotes channel) whose thread it belongs to.
    #[serde(default)]
    pub thread_root_id: Option<MessageId>,
    /// Set when this message is a quote.
    #[serde(default)]
    pub quote: Option<Quote>,
    /// Set when this is a Quote of the Year result card.
    #[serde(default)]
    pub qoty_card: Option<QotyCard>,
    /// Set when this message is a clip.
    #[serde(default)]
    pub clip: Option<Clip>,
    /// Previews of the links in the content, in the order they appear (the server adds them
    /// after the send; the sender can hide them).
    #[serde(default)]
    pub embeds: Vec<LinkEmbed>,
    /// True when the sender removed a preview of a link still in the content, so it can be shown
    /// again (#94, `POST /messages/{id}/embeds/show`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub embeds_hidden: bool,
    /// Set on a DM or group's call line (#50): who started it, when it ended, who joined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub call: Option<CallLine>,
    /// Set while the message is pinned (#178); an expired pin is never sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pin: Option<Pin>,
    /// Set on a pin's system line (#178): what was pinned, by whom, for how long.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pin_line: Option<PinLine>,
    /// Set on a picker wheel spin's line (#158); boxed so every message (and event) stays small.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub wheel: Option<Box<WheelSpin>>,
    /// Set on a poll (#157), as the caller sees it (results only once they may see them); events carry the view of
    /// no one in particular, with no results and nothing marked as yours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub poll: Option<Box<Poll>>,
    /// Set on a poll's "ended" system line (#157).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub poll_ended: Option<Box<PollEnded>>,
    /// Set on a "shared to the stage" line (#234): who shared what kind of Spotify link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stage_share_line: Option<StageShareLine>,
    /// Set on a GIF sent from the picker (#58): its GIPHY id, title and size. The content is the GIF's GIPHY link, so
    /// older clients show a link; a pasted GIPHY link has no `gif` and clients draw it from the link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gif: Option<Gif>,
    /// Images, videos, audio and files sent with the message (#57), in the order they were attached.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<Attachment>>", optional)]
    pub attachments: Vec<Attachment>,
    /// True when `content` is only a stand-in for the attachments' names (nothing was typed), so
    /// the UI can hide it instead of showing it as a caption.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub caption_is_fallback: bool,
    /// Set on a Steam achievement card (#379), a system line in a server's Achievements channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub achievement: Option<Box<AchievementCard>>,
    /// Set on a Deadlock card (#390), a system line in a server's Deadlock channel: a match, the weekly rank movers or
    /// a patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub deadlock: Option<Box<DeadlockCard>>,
    /// Set on a plan (#387): the invite card, with everyone's RSVPs (nothing on it is per person).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub plan: Option<Box<Plan>>,
    /// Set on a plan's reminder line (#387), the system line that pings its Going and Maybe people an hour before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub plan_reminder: Option<Box<PlanReminder>>,
    /// Set on a sports card (#385), a system line in a server's Sports channel: a followed team's match, from the
    /// match-day banner through the live score to full time, edited in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sports: Option<Box<SportsCard>>,
    /// Set on an F1 card (#386), a system line in a server's F1 channel. A weekend card carries everyone's podium
    /// predictions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub f1: Option<Box<F1Card>>,
    /// Set on a forward (#302): where the original was said, by whom and when, and what it said. The forward's own
    /// `content` is the forwarder's comment, or a stand-in when they wrote none (`caption_is_fallback`), and its
    /// `attachments` are the original's (by reference, while it is there).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub forwarded: Option<Box<Forwarded>>,
    /// Set on a group chat's own system line (#114): who created it, added, left or was removed, or renamed it or
    /// changed its icon, by id, so clients write it with the names that apply today. `content` keeps the line as
    /// written (the fallback for older clients, and for anyone a client doesn't know).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub group_line: Option<GroupLine>,
    /// Set on a server membership line (#418), a system line in the server's first text channel: someone left it, or
    /// its owner removed them. Clients name them as they're called today; `content` names them as they were.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub member_line: Option<MemberLine>,
    /// Set on an invite to play a game (#436): the card, as everyone in the chat sees it. Where to join is never in it:
    /// a member of the chat asks for that (`POST /messages/{id}/game-invite/join`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub game_invite: Option<Box<GameInvite>>,
    /// Set on a direct invite to a server (#80): the card's code. What the card shows is per viewer, so clients ask
    /// for it (`GET /invites/{code}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub server_invite: Option<MessageServerInvite>,
}

/// What a group chat's system line says happened (#114).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GroupLineKind {
    /// `by` created the group with `users`.
    Created,
    /// `by` added `users`.
    MembersAdded,
    /// `by` left.
    MemberLeft,
    /// `by` removed `users` (one person).
    MemberRemoved,
    /// `by` renamed the group to `name`.
    Renamed,
    /// `by` removed the group name.
    NameRemoved,
    /// `by` changed the group icon.
    IconChanged,
    /// `by` removed the group icon.
    IconRemoved,
}

/// A group chat's system line by id (#114), read from the payload the server stored with it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct GroupLine {
    pub kind: GroupLineKind,
    pub by: UserId,
    #[serde(default)]
    pub users: Vec<UserId>,
    /// The new name, on `Renamed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
}

/// A server membership line (#418): "alex left the server", or "sam was removed" (`removed_by` the owner who did).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MemberLine {
    pub user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub removed_by: Option<UserId>,
}

/// The original of a forwarded message (#302), as it was when forwarded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Forwarded {
    /// The original (never itself a forward: forwarding a forward forwards its original).
    pub message_id: MessageId,
    pub channel_id: ChannelId,
    /// The kind of chat it was said in: a DM or group is never named.
    pub channel_kind: ChannelKind,
    /// The channel's name, for a server's channel only.
    pub channel_name: Option<String>,
    pub server_id: Option<ServerId>,
    /// The thread it was a reply in, if any (to open it there).
    pub thread_root_id: Option<MessageId>,
    pub author_id: Option<UserId>,
    pub created_at: String,
    /// Its text when forwarded (empty when it was only files, or it is unavailable).
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gif: Option<Gif>,
    /// Its link previews.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<LinkEmbed>>", optional)]
    pub embeds: Vec<LinkEmbed>,
    /// The original was deleted (or its chat was): nothing of it is shown.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub unavailable: bool,
}

/// `POST /messages/{id}/forward` (#302): forward a message you can see to a chat you can post in.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ForwardMessageRequest {
    pub channel_id: ChannelId,
    /// Your words above it (may be empty), up to a message's length.
    #[serde(default)]
    pub comment: String,
    /// As a send's: a retry with the same nonce gets the first forward back.
    #[serde(default)]
    pub nonce: Option<String>,
}

/// How long a pin lasts (#178).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PinDuration {
    Hour,
    Day,
    Week,
    Forever,
}

impl PinDuration {
    /// As the system line words it: "for 1 day", nothing for Forever.
    pub fn phrase(self) -> Option<&'static str> {
        match self {
            Self::Hour => Some("for 1 hour"),
            Self::Day => Some("for 1 day"),
            Self::Week => Some("for 1 week"),
            Self::Forever => None,
        }
    }
}

/// `PUT /messages/{id}/pin`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PinRequest {
    pub duration: PinDuration,
}

/// A message's pin (#178). `expires_at` is `None` for a Forever pin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Pin {
    pub pinned_by: UserId,
    pub pinned_at: String,
    pub expires_at: Option<String>,
}

/// One entry of `GET /channels/{id}/pins`, newest pin first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PinnedMessage {
    pub message: Message,
    pub pin: Pin,
}

/// A pin's system line: "alex pinned a message for 1 day · See pins".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PinLine {
    pub message_id: MessageId,
    pub by: UserId,
    pub duration: PinDuration,
}

/// A call line's metadata (#50). Each member's client renders it: "sam started a call", "· 12 min"
/// once `ended_at` is set, and "Missed call from sam" for a member who isn't in `joined` (and didn't
/// start it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CallLine {
    pub call_id: CallId,
    /// `None` once the starter's account is gone.
    #[serde(default)]
    pub started_by: Option<UserId>,
    #[serde(default)]
    pub ended_at: Option<String>,
    /// Members who were in the call at some point; filled in when it ends.
    #[serde(default)]
    pub joined: Vec<UserId>,
}

/// A picker wheel spin's line (#158): the system line a spin posts, which every client animates to the same
/// `angle` when it arrives live and shows settled otherwise.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct WheelSpin {
    /// `None` once the spinner's account is gone.
    #[serde(default)]
    pub spinner_id: Option<UserId>,
    /// The saved wheel it was spun from; `None` for a quick (one-off) wheel, or once that wheel is deleted.
    #[serde(default)]
    pub wheel_id: Option<String>,
    /// The saved wheel's name when spun; `None` for a quick wheel.
    #[serde(default)]
    pub name: Option<String>,
    /// The slices in order, clockwise from 12 o'clock.
    pub options: Vec<String>,
    /// Index into `options`.
    pub winner: u32,
    /// The wheel's final clockwise rotation in degrees, in [0, 360): the pointer at 12 o'clock then sits inside
    /// the winner's slice ([`wheel_winner_at_weighted`]).
    pub angle: f64,
    /// Each option's weight (#259), as the saved wheel had it: a slice's share of the wheel, and of the draw. `None`
    /// is every option alike (a quick wheel, or a wheel without weights; older clients draw it so).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub weights: Option<Vec<u32>>,
    /// The saved wheel's palette (#259; [`wheel_palettes`]); `None` is the neon spectrum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub palette: Option<String>,
}

/// A quiet picker wheel spin (#270): drawn by the server and shown to the chat live, then gone. Nothing is stored,
/// so it has no line and no history; `id` only tells one quiet spin from another.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct QuietSpin {
    pub id: String,
    pub channel_id: ChannelId,
    pub spin: WheelSpin,
}

/// The slice under the pointer at 12 o'clock when the wheel has turned `angle` degrees clockwise: slice `i` of
/// `n` covers [i·360/n, (i+1)·360/n) clockwise from 12 o'clock.
pub fn wheel_winner_at(angle: f64, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let seg = 360.0 / n as f64;
    let at = (360.0 - angle.rem_euclid(360.0)).rem_euclid(360.0);
    ((at / seg).floor() as usize).min(n - 1)
}

/// [`wheel_winner_at`] for a weighted wheel (#259): slice `i` covers its share of the turn, `weights[i]` of their
/// sum, the slices in order clockwise from 12 o'clock. Empty or all-zero weights: every slice alike.
pub fn wheel_winner_at_weighted(angle: f64, weights: &[u32]) -> usize {
    let total: u64 = weights.iter().map(|&w| u64::from(w)).sum();
    if total == 0 {
        return wheel_winner_at(angle, weights.len());
    }
    let at = (360.0 - angle.rem_euclid(360.0)).rem_euclid(360.0);
    let mut end = 0u64;
    for (i, &w) in weights.iter().enumerate() {
        end += u64::from(w);
        if at < 360.0 * end as f64 / total as f64 {
            return i;
        }
    }
    weights.len() - 1
}

/// A wheel's palettes (#259): `neon` (the default, a spectrum made per wheel) and named sets of theme colours. The
/// client draws them; the server only checks the name.
pub mod wheel_palettes {
    pub const NEON: &str = "neon";
    pub const ALL: [&str; 5] = [NEON, "gems", "signal", "meadow", "accent"];
}

/// Picker wheel limits (#158), shared so the client can say them before the server refuses.
pub mod wheel_limits {
    pub const MIN_OPTIONS: usize = 2;
    pub const MAX_OPTIONS: usize = 100;
    /// Characters per option, after trimming.
    pub const OPTION_CHARS: usize = 60;
    /// Characters in a saved wheel's name, after trimming.
    pub const NAME_CHARS: usize = 40;
    pub const WHEELS_PER_SERVER: i64 = 50;
    pub const WHEELS_PER_PERSON: i64 = 50;
    /// An option's weight (#259): 1 (the default) to this.
    pub const MAX_WEIGHT: u32 = 10;
}

/// A GIPHY GIF as Pulse keeps it (#58): its id and what to show before it loads. No GIPHY URLs: GIPHY's terms
/// forbid storing them, so clients build GIPHY's own media links from the id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Gif {
    /// GIPHY's id (letters and digits).
    pub id: String,
    /// GIPHY's title, the alt text (may be empty).
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// A GIPHY sticker (#293): drawn on the chat itself, without a GIF's frame (it has a transparent background).
    /// Left out when false, so older clients and stored GIFs read as before.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub sticker: bool,
}

/// What a signed-in person's app needs to search GIPHY itself (#58; GIPHY forbids proxying): the server's key
/// (`None`: GIFs aren't set up there) and the content rating to ask for.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct GifConfig {
    pub api_key: Option<String>,
    /// `g`, `pg`, `pg-13` or `r`.
    pub rating: String,
}
redacting_debug!(GifConfig { rating ; api_key });

/// GIF limits (#58), shared so the client can say them before the server refuses.
pub mod gif_limits {
    /// Characters of a GIF id.
    pub const ID_CHARS: usize = 64;
    /// Characters of a GIF's title kept (longer ones are cut).
    pub const TITLE_CHARS: usize = 140;
    /// Largest width or height accepted.
    pub const MAX_SIDE: u32 = 8192;
    pub const FAVOURITES_PER_PERSON: i64 = 200;
}

/// Poll limits (#157), shared so the client can say them before the server refuses.
pub mod poll_limits {
    /// Characters in the question, after trimming.
    pub const QUESTION_CHARS: usize = 200;
    /// Characters per option, after trimming.
    pub const OPTION_CHARS: usize = 80;
    pub const MIN_OPTIONS: usize = 2;
    pub const MAX_OPTIONS: usize = 10;
}

/// How long a poll stays open (#157).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PollDuration {
    Hour,
    Day,
    Week,
    /// Open until its creator ends it.
    Never,
}

/// One option's results (#157).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PollTally {
    pub votes: u32,
    /// Who voted for it, in the order they did; always empty on an anonymous poll.
    pub voters: Vec<UserId>,
}

/// A poll (#157), as one person sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Poll {
    pub question: String,
    pub options: Vec<String>,
    /// Multiple choice: each option is its own vote. Otherwise one choice per person.
    pub multi: bool,
    /// Nobody (the creator included) ever learns who voted for what.
    pub anonymous: bool,
    /// `None`: no end (until its creator ends it).
    pub ends_at: Option<String>,
    /// When it was closed (by its creator, or when its time ran out and the sweep closed it).
    pub closed_at: Option<String>,
    /// Closed, or its time is up (a poll past `ends_at` is over before the sweep has closed it).
    pub closed: bool,
    /// How many people have voted.
    pub voters: u32,
    /// The options you voted for (indexes into `options`).
    pub mine: Vec<u32>,
    /// Per option, in order. `None` until you may see results: after you vote, as its creator, or once it closes.
    pub results: Option<Vec<PollTally>>,
    /// When its creator last changed its question or options (#265); the card says "(edited)". Absent from older
    /// servers: never edited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub edited_at: Option<String>,
}

/// One option in a poll edit (#265): its text, and which of the poll's options (by index, before the edit) it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PollOptionEdit {
    pub text: String,
    /// The option it keeps (its place before the edit), or `None` for a new one. Each at most once; an option no
    /// entry keeps is removed. A kept option whose text changes loses its votes, as a removed one does.
    #[serde(default)]
    pub from: Option<u32>,
}

/// `PUT /messages/{id}/poll` (#265): its creator changes an open poll's question and options. Multiple choice,
/// Anonymous and the end time stay as posted. An edit that would take votes away (an option with votes removed or
/// reworded) is refused with 409 unless `clear_votes` says the creator agreed to it.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct EditPollRequest {
    pub question: String,
    pub options: Vec<PollOptionEdit>,
    #[serde(default)]
    pub clear_votes: bool,
}

/// `POST /channels/{id}/polls`.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreatePollRequest {
    pub question: String,
    pub options: Vec<String>,
    #[serde(default)]
    pub multi: bool,
    #[serde(default)]
    pub anonymous: bool,
    pub ends_in: PollDuration,
}

/// `PUT /messages/{id}/poll/votes`: your choices, replacing any before (empty takes your vote back).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct PollVoteRequest {
    pub options: Vec<u32>,
}

/// A poll's "ended" line (#157): "alex's poll ended: Valheim won (4 votes) · See poll".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PollEnded {
    pub poll_message_id: MessageId,
    /// The poll's creator; `None` once their account is gone.
    #[serde(default)]
    pub author_id: Option<UserId>,
    /// The option(s) with the most votes (all of them on a tie); empty when nobody voted.
    pub winners: Vec<String>,
    /// The winners' vote count (each, on a tie).
    pub votes: u32,
}

/// Plan limits (#387), shared so the client can say them before the server refuses.
pub mod plan_limits {
    /// Characters in the title, after trimming.
    pub const TITLE_CHARS: usize = 100;
    /// Characters in the place, after trimming.
    pub const PLACE_CHARS: usize = 100;
    /// Characters in the note, after trimming.
    pub const NOTE_CHARS: usize = 500;
    /// How far ahead a plan may be, in days.
    pub const MAX_DAYS_AHEAD: i64 = 730;
    /// How long before it starts its Going and Maybe people are reminded, in minutes.
    pub const REMIND_MINUTES: i64 = 60;
    /// How far ahead a forecast reaches, in days (Open-Meteo's daily forecast).
    pub const FORECAST_DAYS: i64 = 16;
    /// How long a plan with an end may run, in hours (#399).
    pub const MAX_HOURS_LONG: i64 = 7 * 24;
    /// How long a plan without an end lasts in a calendar, in minutes (#399): a block, not a point.
    pub const DEFAULT_MINUTES: i64 = 60;
}

/// `PUT /me/plan-reminders` (#399): whether plans' reminder lines ping you (mention you), on every device.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct PlanRemindersRequest {
    pub on: bool,
}

/// Scheduled message and reminder limits (#299, #300), shared so the client can say them before the server refuses.
pub mod schedule_limits {
    /// Scheduled messages one person may have waiting (failed ones count until deleted).
    pub const MAX_SCHEDULED: i64 = 50;
    /// Reminders one person may have set and not yet had.
    pub const MAX_REMINDERS: i64 = 100;
    /// How far ahead a scheduled message or a reminder may be, in days.
    pub const MAX_DAYS_AHEAD: i64 = 365;
}

/// A message written now that the server posts later, as its author (#299). Only its author ever sees it. Text only:
/// mentions and emoji are text, files and GIFs aren't scheduled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ScheduledMessage {
    pub id: ScheduledId,
    pub channel_id: ChannelId,
    pub content: String,
    /// The message it will reply to (in the same chat), if any.
    pub reply_to_id: Option<MessageId>,
    /// When it goes (UTC, RFC 3339, milliseconds).
    pub send_at: String,
    pub created_at: String,
    pub edited_at: Option<String>,
    /// Why it couldn't be posted when its time came (left the chat, the chat was deleted, ...): it stays until its
    /// author deletes it (or sends it again, when the reason is gone). `None` while it waits.
    pub failed: Option<String>,
}

/// `POST /channels/{id}/scheduled` (#299).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ScheduleMessageRequest {
    pub content: String,
    #[serde(default)]
    pub reply_to_id: Option<MessageId>,
    /// When to post it (RFC 3339, any offset): in the future, at most [`schedule_limits::MAX_DAYS_AHEAD`] away.
    pub send_at: String,
}

/// `PATCH /scheduled/{id}` (#299): a new text, a new time, or both. A failed one given a new time waits again.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct EditScheduledRequest {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub send_at: Option<String>,
}

/// "Remind me about this" (#300): private to the person who set it. Once due it is in their inbox (and they get a
/// notification), unread until seen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MessageReminder {
    pub id: ReminderId,
    pub message_id: MessageId,
    pub channel_id: ChannelId,
    /// The thread the message is a reply in, if any (to open it there).
    pub thread_root_id: Option<MessageId>,
    /// When it is (was) due (UTC, RFC 3339, milliseconds).
    pub remind_at: String,
    pub created_at: String,
    /// It has come due: it's in the inbox.
    pub due: bool,
    /// Due and opened (or marked read).
    pub seen: bool,
    /// Came due while none of your devices was connected, and no device has told you yet (#412): the first to claim
    /// it (`POST /reminders/{id}/alerted`) does.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub missed: bool,
    /// The message, cut to one short line.
    pub message: InboxSnippet,
}

/// `POST /messages/{id}/reminders` (#300).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateReminderRequest {
    /// When (RFC 3339, any offset): in the future, at most [`schedule_limits::MAX_DAYS_AHEAD`] away.
    pub remind_at: String,
}

/// Saved message limits (#315), shared so the client can say them before the server refuses.
pub mod bookmark_limits {
    /// Saved messages one person may have.
    pub const MAX_BOOKMARKS: i64 = 500;
    /// The longest note on one, in characters.
    pub const NOTE_MAX_CHARS: usize = 500;
    /// How much of the message a saved one shows, in characters.
    pub const SNIPPET_CHARS: usize = 300;
}

/// What a saved message shows of the message while its person can still see it (#315).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SavedMessage {
    pub channel_id: ChannelId,
    /// The thread the message is a reply in, if any (to open it there).
    pub thread_root_id: Option<MessageId>,
    pub author_id: Option<UserId>,
    /// At most [`bookmark_limits::SNIPPET_CHARS`] characters of it.
    pub content: String,
    pub created_at: String,
}

/// A saved message (#315): a private bookmark, with an optional note. Only its person ever sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MessageBookmark {
    pub id: BookmarkId,
    pub message_id: MessageId,
    pub note: Option<String>,
    /// When it was saved.
    pub created_at: String,
    /// The message, while they can see it; `None` once it is deleted or out of their reach (left the chat, the chat
    /// deleted): nothing of it is shown then.
    pub message: Option<SavedMessage>,
}

/// `POST /messages/{id}/bookmark` (#315).
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct SaveMessageRequest {
    /// At most [`bookmark_limits::NOTE_MAX_CHARS`] characters; empty or none: no note.
    #[serde(default)]
    pub note: Option<String>,
}

/// `PATCH /bookmarks/{id}` (#315): the new note (empty or none: no note).
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct EditBookmarkRequest {
    #[serde(default)]
    pub note: Option<String>,
}

/// An answer to a plan (#387).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Rsvp {
    Going,
    Maybe,
    Cant,
}

/// A plan (#387): an invite card posted in a chat. Everyone sees the same card, so events carry it whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Plan {
    pub title: String,
    /// When it starts (UTC, RFC 3339); clients show it in the viewer's own time.
    pub starts_at: String,
    /// When it ends (UTC, RFC 3339), when its creator gave an end (#399): after the start, at most
    /// [`plan_limits::MAX_HOURS_LONG`] later. Absent: a point in time (the calendar file gives it an hour).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ends_at: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// Who answered what, in the order they did.
    pub going: Vec<UserId>,
    pub maybe: Vec<UserId>,
    pub cant: Vec<UserId>,
    /// Set once its creator cancelled it: no more answers, no reminder.
    #[serde(default)]
    pub cancelled_at: Option<String>,
    /// When its creator last changed it.
    #[serde(default)]
    pub edited_at: Option<String>,
}

/// `POST /channels/{id}/plans`, and `PUT /messages/{id}/plan` (its creator's edit, replacing every field).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreatePlanRequest {
    pub title: String,
    /// RFC 3339, any offset (stored as UTC).
    pub starts_at: String,
    /// RFC 3339, any offset (#399): an end after the start; none (or absent) = no end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ends_at: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// `PUT /messages/{id}/plan/rsvp`: your answer, replacing any before (`None` takes it back).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct PlanRsvpRequest {
    pub rsvp: Option<Rsvp>,
}

/// `GET /messages/{id}/plan/forecast` (#387): Open-Meteo's daily forecast for the plan's place on its day, through
/// the server (cached there). `None` when there's none: no place, a place Open-Meteo doesn't know, a day outside
/// the forecast's reach, or the forecast unavailable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PlanForecast {
    /// The place as Open-Meteo found it ("Cape Town, South Africa").
    pub place: String,
    /// The day at the place (`YYYY-MM-DD`).
    pub date: String,
    /// The WMO weather code (0 clear … 99 thunderstorm with hail).
    pub code: u8,
    /// Degrees Celsius.
    pub temp_max: f32,
    pub temp_min: f32,
    /// The day's highest chance of rain, in percent.
    #[serde(default)]
    pub rain_chance: Option<u8>,
    /// Degrees Celsius in the hour the plan starts, at the place (#399); none from an older server or forecast.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub temp_at_start: Option<f32>,
}

/// A plan's reminder line (#387): "⏰ Braai at kim's starts in an hour · See plan".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PlanReminder {
    pub plan_message_id: MessageId,
    pub title: String,
    pub starts_at: String,
}

/// What a watch party plays (#459). Older parties and clients know only YouTube, the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WatchSource {
    /// A YouTube video: synced play, pause and seek. Its id is 11 of `[A-Za-z0-9_-]`.
    #[default]
    Youtube,
    /// A Twitch VOD (`twitch.tv/videos/<id>`): synced play, pause and seek. Its id is 1 to 15 digits.
    TwitchVideo,
    /// A Twitch channel, live: play and pause only, and it plays until skipped. Its id is the channel's login, 1 to 25
    /// of `[a-z0-9_]`.
    TwitchChannel,
    /// A Twitch clip: everyone's starts together, but there's no control (pause is each viewer's own). Its id is the
    /// clip's slug, 1 to 100 of `[A-Za-z0-9_-]`.
    TwitchClip,
}

impl WatchSource {
    /// Whether `id` is well formed for this source. Ids hold only characters that need no escaping in a URL.
    pub fn valid_id(self, id: &str) -> bool {
        let within = |min: usize, max: usize, ok: fn(u8) -> bool| {
            (min..=max).contains(&id.len()) && id.bytes().all(ok)
        };
        let slug = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
        match self {
            Self::Youtube => within(11, 11, slug),
            Self::TwitchVideo => within(1, 15, |b| b.is_ascii_digit()),
            Self::TwitchChannel => within(1, 25, |b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'
            }),
            Self::TwitchClip => within(1, 100, slug),
        }
    }

    /// Whether everyone's player follows the party's position (seek included).
    pub fn seeks(self) -> bool {
        matches!(self, Self::Youtube | Self::TwitchVideo)
    }
}

/// A YouTube watch party in a server voice channel (#384): one per channel, its state held in memory on the server.
/// Each viewer's player is at `position_ms + (server now - set_at_ms)` while `playing`, else at `position_ms`. Gone
/// when its last viewer leaves it or the call. Everyone sees that it's on (the channel list shows it); only the
/// people in the call may watch or control it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct WatchParty {
    pub channel_id: ChannelId,
    /// Who controls it (with everyone else too, while `everyone_controls`).
    pub host_id: UserId,
    /// What `video_id` is on (#459): YouTube unless it says otherwise.
    #[serde(default)]
    pub source: WatchSource,
    /// The video, VOD, channel or clip: valid for `source` ([`WatchSource::valid_id`]).
    pub video_id: String,
    /// The video's title as the starter's card showed it; absent when it had none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    /// Where the video was at `set_at_ms`, in milliseconds.
    pub position_ms: u64,
    pub playing: bool,
    /// When `position_ms` and `playing` were set, by the server's clock (Unix milliseconds).
    pub set_at_ms: i64,
    /// "Let everyone control": anyone watching may play, pause, seek and change the video, not only the host.
    pub everyone_controls: bool,
    /// Who has the player open, in the order they joined it (the host passes to the next of them).
    pub viewers: Vec<UserId>,
    /// What plays next, in order. It goes with the party: ending the party clears it.
    #[serde(default)]
    pub queue: Vec<WatchPartyItem>,
    /// Who put the current video on (#464): its starter, or whoever queued it. They may skip it, as a controller may.
    /// Absent from a party that predates it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub added_by: Option<UserId>,
}

/// A video waiting in a watch party's queue (#384, #464).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct WatchPartyItem {
    #[serde(default)]
    pub source: WatchSource,
    pub video_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    pub added_by: UserId,
    /// The video's thumbnail, from its link card when the server has one (a `link_thumb` media hash).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub thumb: Option<EmbedThumb>,
}

/// `PUT /channels/{id}/watch-party` (#384): start a watch party of a YouTube video (or, if one is on and you may
/// control it, change its video).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StartWatchPartyRequest {
    #[serde(default)]
    pub source: WatchSource,
    pub video_id: String,
    #[serde(default)]
    pub title: Option<String>,
}

/// `POST /channels/{id}/watch-party` (#384): one change to the party. Positions are the sender's player's, in
/// milliseconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum WatchPartyAction {
    Play {
        position_ms: u64,
    },
    Pause {
        position_ms: u64,
    },
    Seek {
        position_ms: u64,
    },
    /// Play another video now, from the start.
    Load {
        #[serde(default)]
        source: WatchSource,
        video_id: String,
        #[serde(default)]
        title: Option<String>,
    },
    /// Add a video to the end of the queue (anyone in the call may, #464).
    Enqueue {
        #[serde(default)]
        source: WatchSource,
        video_id: String,
        #[serde(default)]
        title: Option<String>,
    },
    /// Take the queue's `index`th video out (whoever may control, or whoever added it).
    Unqueue {
        index: u32,
    },
    /// Move the queue's `index`th video to `to` (#464; whoever may control, or whoever added it). A `to` past the end
    /// is the end.
    Move {
        index: u32,
        to: u32,
    },
    /// Play the queue's `index`th video now, from the start (#464; whoever may control).
    PlayNow {
        index: u32,
    },
    /// The queue's next video now (or, with none, stop at the end): a skip, or the player reaching the end. Whoever
    /// may control may, and whoever put the current video on. With `from`, only while `from` is still the current
    /// video: two players reaching the end at once move it on once.
    Next {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        from: Option<String>,
    },
    /// "Let everyone control" (the host only).
    EveryoneControls {
        on: bool,
    },
    /// Make someone in the call the host (the host only).
    HandOff {
        user_id: UserId,
    },
    /// Open the player: you're one of its viewers.
    Join,
    /// Close the player, staying in the call. A host who leaves passes it on, and the last viewer out ends it.
    Leave,
}

/// A Spotify link shared to a server voice channel's stage (#234): at most one per channel, gone when its sharer
/// stops it or leaves the call, someone in the call stops it, or at `expires_at`. In memory on the server only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StageShare {
    /// Tells one share from the next (a stop names the share it means).
    pub id: String,
    pub channel_id: ChannelId,
    pub shared_by: UserId,
    pub shared_at: String,
    pub expires_at: String,
    /// The link's Spotify card, as the embed resolver built it (`provider` is always Spotify).
    pub embed: LinkEmbed,
    /// A Jam's live "Now playing" (#275): what its sharer's Spotify is playing, while their Pulse reads it. Absent
    /// otherwise, on anything but a Jam, and from older servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub now_playing: Option<Listening>,
}

/// `PUT /channels/{id}/stage-share` (#234): a Spotify link (web, `spotify:socialsession:` or `spotify.link`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StageShareRequest {
    pub url: String,
}

/// A "shared to the stage" system line (#234): "🎵 alex shared a Jam to the stage".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StageShareLine {
    pub by: UserId,
    /// The Spotify kind shared: `jam`, `track`, `album`, `playlist`, `episode`, `show` or `artist`.
    pub kind: String,
}

/// Who a saved wheel is for (#158).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WheelScope {
    /// Everyone in its server sees and edits it.
    Server,
    /// "Just me": only its owner sees it, and may spin it in any chat.
    Personal,
}

/// A saved picker wheel (#158).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Wheel {
    pub id: String,
    pub scope: WheelScope,
    /// Set for a server wheel.
    #[serde(default)]
    pub server_id: Option<ServerId>,
    /// Set for a Just me wheel.
    #[serde(default)]
    pub owner_id: Option<UserId>,
    pub name: String,
    pub options: Vec<String>,
    pub updated_at: String,
    /// Each option's weight (#259), one per option, 1 to [`wheel_limits::MAX_WEIGHT`]; `None` is all alike.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub weights: Option<Vec<u32>>,
    /// Its palette (#259; [`wheel_palettes`]); `None` is the neon spectrum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub palette: Option<String>,
}

/// `POST /wheels`: a new saved wheel, in `server_id` (a server wheel) or, without one, Just me.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateWheelRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub server_id: Option<ServerId>,
    pub name: String,
    #[serde(default)]
    pub options: Vec<String>,
    /// One per option (#259); absent = all alike.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub weights: Option<Vec<u32>>,
    /// A [`wheel_palettes`] name; absent = the neon spectrum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub palette: Option<String>,
}

/// `PATCH /wheels/{id}`: what changes (absent = kept).
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct UpdateWheelRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub options: Option<Vec<String>>,
    /// One per option (#259), with or without `options`. New options without weights: all alike again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub weights: Option<Vec<u32>>,
    /// A [`wheel_palettes`] name (`neon` for the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub palette: Option<String>,
}

/// `POST /channels/{id}/wheel-spins` (and `.../wheel-spins/quiet`, #270): spin a saved wheel (`wheel_id`) or a quick
/// one (`options`). Both: the saved wheel, named, with only these of its options (Remove winner on a quiet spin,
/// which has no stored spin to run from).
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct SpinWheelRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub wheel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub options: Option<Vec<String>>,
}

/// `POST /messages/{id}/spin-again`: the same options again, or without the winner.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct SpinAgainRequest {
    #[serde(default)]
    pub remove_winner: bool,
}

/// A call in progress in a DM or group (#50). It started when the room went from empty to occupied
/// and lasts until the room has been empty for 30 s.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CallInfo {
    pub id: CallId,
    pub channel_id: ChannelId,
    pub started_by: Option<UserId>,
    pub started_at: String,
    /// The call's line in the conversation.
    pub line_id: Option<MessageId>,
}

/// You are being rung (#50): the call, who rang, and when the ring gives up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CallRing {
    pub call_id: CallId,
    pub channel_id: ChannelId,
    pub caller_id: UserId,
    pub expires_at: String,
}

/// Why your ring stopped (#50). The caller is never told any of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RingEndReason {
    /// You joined the call (on this device or another).
    Answered,
    /// You declined (on this device or another).
    Declined,
    TimedOut,
    CallEnded,
}

/// `POST /calls/{channel}`: the call you started or joined; `rang` is false when one was already
/// going (joining it rings nobody).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StartCallResponse {
    pub call: CallInfo,
    pub rang: bool,
}

/// One emoji on a message: a Unicode emoji or `custom:<id>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ReactionSummary {
    pub emoji: String,
    pub count: u32,
    pub me: bool,
}

/// A person with their presence as the receiving viewer may see it (#45): someone Invisible, or
/// appearing offline to this viewer, is `Offline` with a frozen `last_seen_at`, exactly as if they
/// had left.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Person {
    pub user: User,
    /// `status != Offline` (kept for older clients).
    pub online: bool,
    pub last_seen_at: Option<String>,
    /// Absent from older servers: go by `online` then.
    #[serde(default)]
    pub status: PresenceStatus,
    /// Online with every device idle (#53); only ever with `status: Online`. Omitted when false, and absent from
    /// older servers.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub idle: bool,
    /// What they're listening to on Spotify (#275), only while they share it and look online to this viewer.
    /// Absent otherwise, and from older servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub listening: Option<Listening>,
    /// The game they're playing (#380), by the same rule as `listening`. Absent otherwise, and from older servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub playing: Option<Playing>,
    /// Their custom status (#152), by the same rule as `listening`: only while they look online to this viewer (and
    /// always to themselves). Absent otherwise, and from older servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub custom_status: Option<CustomStatus>,
}

/// What someone's music player is playing (#275, other players #322), read by their own Pulse from the OS media
/// session (MPRIS on Linux, the system media controls on Windows): no player login, nothing that controls their
/// player.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Listening {
    pub title: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    /// `track:<id>` or `episode:<id>` (Spotify's 22-character id), when the media session gave one: the link-out is
    /// then `open.spotify.com/<kind>/<id>`. Only ever with Spotify.
    #[serde(default)]
    pub spotify_id: Option<String>,
    /// The album art, from that link's Spotify card (the server's embed resolver); none without a `spotify_id`.
    #[serde(default)]
    pub art: Option<EmbedThumb>,
    /// Which player it's from (#322). Absent means Spotify: older servers and clients only ever knew Spotify, and a
    /// Spotify activity is sent without it, as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub player: Option<MusicPlayer>,
}

impl Listening {
    /// Whether it's from Spotify (no `player`, or `player: spotify`): only Spotify's has links, art, or a Jam.
    pub fn is_spotify(&self) -> bool {
        matches!(self.player, None | Some(MusicPlayer::Spotify))
    }
}

/// The music players "Listening to" reads (#322), each from a strict allowlist of OS media sessions (see
/// `docs/listening-players.md`). A player this build doesn't know reads as `Other`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MusicPlayer {
    Spotify,
    YoutubeMusic,
    AppleMusic,
    Tidal,
    Deezer,
    Foobar2000,
    /// A music site in a browser that Pulse can't name (Windows doesn't say which site a browser plays).
    Browser,
    #[serde(other)]
    Other,
}

/// What a client says its player is playing (#275, `ClientFrame::Listening`); the server checks it and adds the art.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ListeningUpdate {
    pub title: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub spotify_id: Option<String>,
    /// Which player (#322); absent means Spotify (older clients).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub player: Option<MusicPlayer>,
}

/// The game someone is playing (#380), as their own Pulse detects it on Windows (Steam's running app, or an exe they
/// added by hand). Only what their switches allow is here: no app id (and so no art) with "Show game art" off, no
/// start with "Show play time" off. The exe's path never leaves their computer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Playing {
    pub name: String,
    /// Steam's app id, when Steam named the game and they share its art.
    #[serde(default)]
    pub steam_app_id: Option<u32>,
    /// When they started, in server time (RFC 3339), when they share play time.
    #[serde(default)]
    pub started_at: Option<String>,
    /// The game's art from Steam's CDN, fetched and stored by the server; none without a `steam_app_id`.
    #[serde(default)]
    pub art: Option<GameArt>,
    /// Their Steam lobby or game server others can join (#381), when Steam shows one publicly and they share Join.
    /// Set by the server only (never from a client's update). Older servers never send it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub join: Option<SteamJoin>,
}

/// Where a friend's Steam game can be joined (#381), checked by the server: `Lobby` becomes
/// `steam://joinlobby/<app_id>/<lobby_id>/<steam_id>`, `Server` becomes `steam://connect/<address>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SteamJoin {
    Lobby {
        app_id: u32,
        /// The lobby's 64-bit SteamID, as digits.
        lobby_id: String,
        /// The host's (the friend's) 64-bit SteamID, as digits.
        steam_id: String,
    },
    Server {
        app_id: u32,
        /// `a.b.c.d:port` (IPv4, as Steam reports it), or `[ipv6]:port` in its canonical form (#397).
        address: String,
    },
}

/// An invite to play a game (#436), sent into a DM or group by its author while they play: the game as their
/// "Playing" showed it then. `joinable` while there's a Steam lobby or game server to join (the server hands it only to
/// the chat's members, on `POST /messages/{id}/game-invite/join`); `ended` once the author left that game or its
/// lobby, for good. A server restart ends it too, but it opens again when the author is seen in the same game within
/// minutes (#438).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct GameInvite {
    pub name: String,
    #[serde(default)]
    pub steam_app_id: Option<u32>,
    #[serde(default)]
    pub art: Option<GameArt>,
    #[serde(default)]
    pub joinable: bool,
    #[serde(default)]
    pub ended: bool,
}

/// A Steam game's art (#380), each re-encoded like a link thumbnail; any may be missing (not every game has a hero or
/// a logo).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct GameArt {
    /// The wide library hero (`library_hero.jpg`).
    #[serde(default)]
    pub hero: Option<EmbedThumb>,
    /// The store capsule (`header.jpg`).
    #[serde(default)]
    pub header: Option<EmbedThumb>,
    /// The transparent logo (`logo.png`).
    #[serde(default)]
    pub logo: Option<EmbedThumb>,
}

/// What a client says it's playing (#380, `ClientFrame::Playing`); the server checks it and adds the art.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PlayingUpdate {
    pub name: String,
    #[serde(default)]
    pub steam_app_id: Option<u32>,
    /// How long they've been playing, in seconds, when they share play time (the server turns it into `started_at`
    /// on its own clock).
    #[serde(default)]
    pub elapsed_secs: Option<u64>,
    /// "Show game art" is off but Steam achievements are shared (#379): the server keeps `steam_app_id` for the
    /// achievement poller only, fetches no art and shows nobody the id. Older clients never send it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub hide_art: bool,
}

/// What a viewer sees of someone (#45). Never `Invisible`: that looks exactly like `Offline`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus {
    Online,
    Dnd,
    #[default]
    Offline,
}

/// The status you set for yourself (#45); saved on your account and applied everywhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Online,
    Dnd,
    Invisible,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Dnd => "dnd",
            Self::Invisible => "invisible",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "online" => Some(Self::Online),
            "dnd" => Some(Self::Dnd),
            "invisible" => Some(Self::Invisible),
            _ => None,
        }
    }
}

/// Your own status, for your own sessions only. `dnd_until`: when a timed DND or Invisible ends
/// (RFC 3339 UTC; #153 kept the name); `None` means until you turn it off.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OwnStatus {
    pub status: Status,
    #[serde(default)]
    pub dnd_until: Option<String>,
}

/// `PUT /me/status`. `dnd_until` goes with `Dnd` or `Invisible` (#153), never `Online`: a time in
/// the next 7 days.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SetStatusRequest {
    pub status: Status,
    #[serde(default)]
    pub dnd_until: Option<String>,
}

/// A custom status (#152): a short line of plain text and an optional emoji, a standard one as Unicode or a server's
/// as `custom:<id>` (like a reaction). The server keeps the text sanitised (trimmed, whitespace collapsed, no control
/// or bidi characters, at most [`CUSTOM_STATUS_MAX`] characters); `text` may be empty when there's an emoji.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CustomStatus {
    pub text: String,
    #[serde(default)]
    pub emoji: Option<String>,
}

/// The longest custom status text, in characters (#152).
pub const CUSTOM_STATUS_MAX: usize = 128;

/// Your own custom status (#152), for your own sessions only: what it is, when it clears (RFC 3339 UTC; `None`:
/// never), and the last few you set, newest first, for picking again.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MyCustomStatus {
    #[serde(default)]
    pub current: Option<CustomStatus>,
    #[serde(default)]
    pub until: Option<String>,
    #[serde(default)]
    pub recent: Vec<CustomStatus>,
}

/// `PUT /me/custom-status` (#152): `until` is when it clears, a time in the next 8 days (the client works out "today"
/// and "this week" in local time), or `None` to keep it until you clear it (`DELETE /me/custom-status`).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SetCustomStatusRequest {
    pub text: String,
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub until: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ReadState {
    pub channel_id: ChannelId,
    pub last_read_message_id: Option<MessageId>,
    pub unread: u32,
    pub mentions: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MuteTarget {
    Server,
    Channel,
}

impl MuteTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Channel => "channel",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "server" => Some(Self::Server),
            "channel" => Some(Self::Channel),
            _ => None,
        }
    }
}

/// `until: None` = muted until unmuted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Mute {
    pub target_kind: MuteTarget,
    pub target_id: String,
    pub until: Option<String>,
}

/// Which server messages alert you (the message sound and the desktop notification, #195). A mute still lets only
/// mentions through, and Do Not Disturb nothing; unread badges count whatever the level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum NotifyLevel {
    All,
    Mentions,
    Nothing,
}

impl NotifyLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Mentions => "mentions",
            Self::Nothing => "nothing",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "all" => Some(Self::All),
            "mentions" => Some(Self::Mentions),
            "nothing" => Some(Self::Nothing),
            _ => None,
        }
    }
}

/// A level you picked for a server or a server channel. Absent = the default: All messages for a server, following
/// its server for a channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct NotifyLevelPref {
    pub target_kind: MuteTarget,
    pub target_id: String,
    pub level: NotifyLevel,
}

/// `level: None` = back to the default (a server: All messages; a channel: follow its server).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SetNotifyLevelRequest {
    pub target_kind: MuteTarget,
    pub target_id: String,
    pub level: Option<NotifyLevel>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AddMembersRequest {
    pub user_ids: Vec<UserId>,
}

/// `None` or blank → back to the default name (the members' names).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct RenameChannelRequest {
    pub name: Option<String>,
}

/// A group's icon (#56): the hash of a picture from `POST /media/avatar`, or `None` to clear it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SetGroupIconRequest {
    pub hash: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MarkReadRequest {
    pub message_id: MessageId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SetMuteRequest {
    pub target_kind: MuteTarget,
    pub target_id: String,
    pub until: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SendMessageRequest {
    pub content: String,
    pub reply_to_id: Option<MessageId>,
    pub nonce: Option<String>,
    /// Post into this quote's thread (quotes channels only).
    #[serde(default)]
    pub thread_root_id: Option<MessageId>,
    /// A GIF from the picker (#58): `content` must be a GIPHY link to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gif: Option<Gif>,
    /// Uploads (from `POST /attachments`) to attach; sends from before attachments omit it.
    #[serde(default)]
    pub attachments: Vec<AttachmentId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct EditMessageRequest {
    pub content: String,
}

/// How many earlier versions of an edited message are kept (#301): older ones go as new edits come.
pub const MAX_EDIT_VERSIONS: i64 = 20;

/// One version of an edited message (#301): its text, and when it was written (sent, or the edit that made it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MessageVersion {
    pub content: String,
    pub written_at: String,
}

// ---------- Attachments (#57) ----------

/// What an attachment is, which decides how the UI shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Image,
    Video,
    Audio,
    File,
}

/// A file sent with a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Attachment {
    pub id: AttachmentId,
    pub name: String,
    pub content_type: String,
    pub kind: AttachmentKind,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u32>,
    /// Temporary sends only (RFC 3339, millisecond precision, Z).
    pub expires_at: Option<String>,
    /// A temporary send whose bytes are gone.
    pub expired: bool,
    /// The sender marked it as a spoiler (#289): shown blurred until clicked. Off the wire when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub spoiler: bool,
    /// View once (#311): each recipient can open it once (`POST /messages/{id}/view-once/open`); the server signs no
    /// other link to it for them, before or after. Its sender always can. Off the wire when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub view_once: bool,
}

/// `POST /attachments`: what is about to be uploaded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StartAttachmentRequest {
    pub name: String,
    pub size: u64,
    pub content_type: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub duration_ms: Option<u32>,
    /// Keep the file for this many days, then delete it (a temporary send).
    #[serde(default)]
    pub temp_days: Option<u8>,
    /// Show it blurred until clicked (#289).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub spoiler: bool,
    /// Each recipient can open it once (#311).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub view_once: bool,
}

/// Where to put the bytes. `url` is `None` when the upload has no single PUT URL (big sends).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AttachmentUpload {
    pub id: AttachmentId,
    pub url: Option<String>,
    pub content_type: String,
    pub expires_at: String,
    /// A big temporary send goes up in parts instead (`url` is then `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub multipart: Option<MultipartInfo>,
}

/// How a big temporary send goes up (#57): in parts of `part_size` bytes (the last one shorter),
/// each PUT to its own signed URL (`POST /attachment-uploads/{id}/parts`), then
/// `POST /attachment-uploads/{id}/complete`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MultipartInfo {
    pub part_size: u64,
    pub parts: u16,
}

/// Parts of a big upload to sign, numbered from 1 (at most `attachment_limits::PARTS_PER_ASK`).
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct PartUrlsRequest {
    pub parts: Vec<u16>,
}

/// A signed PUT for one part; the store takes exactly `size` bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PartUrl {
    pub part: u16,
    pub url: String,
    pub size: u64,
}

/// A part the store already has (a resumed upload skips it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct UploadedPart {
    pub part: u16,
    pub size: u64,
}

/// A fresh link to download an attachment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AttachmentUrl {
    pub id: AttachmentId,
    pub url: Option<String>,
    pub expires_at: Option<String>,
}

/// Who has opened a message's view-once files (#311), `GET /messages/{id}/view-once`: for its sender, everyone who
/// opened them (first opened first); for anyone else, only whether they did (`viewers` empty).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ViewOnceState {
    /// You've opened them (always false for their sender).
    pub opened: bool,
    pub viewers: Vec<ViewOnceViewer>,
}

/// Someone who opened a message's view-once files, and when (#311).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ViewOnceViewer {
    pub user_id: UserId,
    pub viewed_at: String,
}

/// Limits on attachments, shared so the client can refuse before uploading.
pub mod attachment_limits {
    pub const PER_MESSAGE: usize = 10;
    pub const MAX_BYTES: u64 = 50 * 1024 * 1024;
    pub const TEMP_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
    pub const TEMP_CAP_BYTES: u64 = 20 * 1024 * 1024 * 1024;
    pub const NAME_MAX_CHARS: usize = 200;
    /// A big send's part size (the last part is shorter): 2 GB is 128 parts.
    pub const PART_SIZE: u64 = 16 * 1024 * 1024;
    /// Parts signed per request.
    pub const PARTS_PER_ASK: usize = 16;
    /// How long a temporary send can last, in days: the sender picks one.
    pub const TEMP_DAYS: [u8; 2] = [1, 7];
    /// One person's big uploads not yet sent hold at most this much of the server's room.
    pub const TEMP_PENDING_PER_USER: u64 = 4 * 1024 * 1024 * 1024;
}

/// A LiveKit join token (voice, or a stream ticket) and where to use it.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct VoiceTokenResponse {
    pub url: String,
    pub token: String,
}
redacting_debug!(VoiceTokenResponse { url ; token });

/// A server's server mute / server deafen of one of its members (#140): who did each, `None`
/// when not in force. Lasts across leaving and rejoining until someone undoes it. Deafen
/// implies mute (no publishing either way).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ServerVoiceMod {
    pub server_id: ServerId,
    pub user_id: UserId,
    pub muted_by: Option<UserId>,
    pub deafened_by: Option<UserId>,
}

/// `POST /servers/{server}/voice/{user}`: set (`true`) or undo (`false`) a server mute or
/// deafen; a field left out stays as it is.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct SetServerVoiceRequest {
    #[serde(default)]
    pub mute: Option<bool>,
    #[serde(default)]
    pub deafen: Option<bool>,
}

/// Screen-share quality. `height` is 720, 1080 or 1440; `fps` is 15, 30 or 60.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StreamSettings {
    pub height: u16,
    pub fps: u8,
    pub audio: bool,
    /// Server members outside the voice channel may watch anonymously (spec §5). On unless said.
    #[serde(default = "yes")]
    pub allow_ghosts: bool,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum StreamStatus {
    Live,
    Reconnecting,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StreamInfo {
    pub channel_id: ChannelId,
    pub user_id: UserId,
    pub settings: StreamSettings,
    pub status: StreamStatus,
    /// Named viewers: people in that voice channel.
    pub viewers: Vec<UserId>,
    /// How many ghosts watch. Who they are never leaves the server.
    #[serde(default)]
    pub ghosts: u32,
    pub started_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum StreamEndReason {
    Stopped,
    LeftVoice,
    Crashed,
    TimedOut,
}

/// Which stream-room identity a ticket is for: `v` is the video connection, `a` the audio one.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum StreamRole {
    V,
    A,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StreamTicketRequest {
    pub role: StreamRole,
    /// A subscribe-only ticket under a fresh anonymous identity, for watching from outside the
    /// voice channel (spec §5.2).
    #[serde(default)]
    pub ghost: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct GoLiveRequest {
    pub settings: StreamSettings,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct WatchRequest {
    pub streamer: UserId,
    /// Watching as a ghost (from outside the voice channel), counted but never named.
    #[serde(default)]
    pub ghost: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum ReportKind {
    Bug,
    Crash,
    UnexpectedExit,
    /// A feature request (#218): the person's words only, no log or diagnostics.
    Idea,
}

impl ReportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bug => "bug",
            Self::Crash => "crash",
            Self::UnexpectedExit => "unexpected-exit",
            Self::Idea => "idea",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateReportRequest {
    pub kind: ReportKind,
    pub text: String,
    /// App version, OS, connection and voice state, crash details: free-form so new fields need no migration.
    #[ts(type = "Record<string, unknown>")]
    pub meta: serde_json::Value,
    pub log: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateReportResponse {
    pub id: String,
}

/// A stored, processed upload (`POST /media/emoji`). Fetch the bytes at `GET /media/{hash}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MediaInfo {
    pub hash: String,
    pub content_type: String,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub animated: bool,
    /// The first frame alone, for animations.
    pub still_hash: Option<String>,
}

/// A server's custom emoji. `hash: None` means it was deleted: the name stays so old messages
/// and reactions can still show a `:name:` chip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CustomEmoji {
    pub id: String,
    pub server_id: ServerId,
    pub name: String,
    pub hash: Option<String>,
    /// The first frame, for animated emoji.
    pub still_hash: Option<String>,
    pub animated: bool,
    pub added_by: Option<UserId>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateEmojiRequest {
    pub name: String,
    /// From `POST /media/emoji`.
    pub media_hash: String,
}

/// A server soundboard sound.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Sound {
    pub id: String,
    pub server_id: ServerId,
    pub name: String,
    /// A Unicode emoji shown on the tile.
    pub emoji: Option<String>,
    /// Or a custom emoji's id (from any server the uploader is in) instead. At most one of the
    /// two is set; a custom emoji deleted later leaves the tile without one.
    #[serde(default)]
    pub emoji_id: Option<String>,
    /// The Ogg Opus file in the media store.
    pub media_hash: String,
    /// Playback gain in percent (10 to 200).
    pub volume: u16,
    pub duration_ms: u32,
    pub added_by: Option<UserId>,
    pub created_at: String,
    /// Times it has been played in this server.
    pub plays: u32,
}

fn default_sound_volume() -> u16 {
    100
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateSoundRequest {
    pub name: String,
    #[serde(default)]
    pub emoji: Option<String>,
    /// A custom emoji instead of `emoji` (not both).
    #[serde(default)]
    pub emoji_id: Option<String>,
    /// From `POST /media/sound`.
    pub media_hash: String,
    #[serde(default = "default_sound_volume")]
    pub volume: u16,
}

/// `PUT /servers/{id}/join-sound` (#121): the sound that plays when you join that server's voice channels.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SetJoinSoundRequest {
    /// A sound of that server's soundboard.
    pub sound_id: String,
}

/// Edit a sound. Absent fields stay as they are; `emoji: null` clears the emoji. Setting `emoji`
/// clears `emoji_id` and the other way round; setting both at once is refused.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct UpdateSoundRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_or_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional = nullable, type = "string | null")]
    pub emoji: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "present_or_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional = nullable, type = "string | null")]
    pub emoji_id: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub volume: Option<u16>,
}

/// Tells an absent field (`None`, via `default`) from an explicit `null` (`Some(None)`).
fn present_or_null<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RenameEmojiRequest {
    pub name: String,
}

/// The 403 body `POST /auth/login` answers when the password is right but a second factor is
/// needed. `code` is always `"mfa_required"`; the `ticket` is sent back with the code.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct MfaRequired {
    pub code: String,
    pub message: String,
    pub ticket: String,
}
redacting_debug!(MfaRequired { code, message ; ticket });

/// `POST /auth/login/2fa`: the ticket from `MfaRequired` plus a TOTP or recovery code.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct LoginMfaRequest {
    pub ticket: String,
    pub code: String,
}
redacting_debug!(LoginMfaRequest { ; ticket, code });

// ---------- Quotes ----------

/// The quote part of a message in a quotes channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Quote {
    pub lines: Vec<QuoteLine>,
    pub year: i32,
    /// `YYYY-MM-DD`, set only for imported quotes.
    pub said_on: Option<String>,
    pub reply_count: u32,
    /// The newest reply in the thread.
    pub last_reply_id: Option<MessageId>,
    /// Per caller: the newest reply you have seen (gateway events carry `None`).
    pub replies_seen: Option<MessageId>,
    /// Years this quote won Quote of the Year.
    pub wins: Vec<i32>,
}

/// The clip part of a message in a clips channel. The caption is the message content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Clip {
    pub thumb_hash: String,
    pub duration_ms: u32,
    pub width: u32,
    pub height: u32,
    #[ts(type = "number")]
    pub size: u64,
    pub game_id: Option<GameId>,
    pub reply_count: u32,
    /// The newest reply in the thread.
    pub last_reply_id: Option<MessageId>,
    /// Per caller: the newest reply you have seen (gateway events carry `None`).
    pub replies_seen: Option<MessageId>,
}

/// A game in a server's registry; clips are tagged with one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Game {
    pub id: GameId,
    pub server_id: ServerId,
    pub name: String,
    pub created_by: Option<UserId>,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AddGameRequest {
    pub name: String,
}

/// How a user looks at a clips channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ClipView {
    List,
    Wall,
}

/// A user's chosen view of one clips channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ClipViewPref {
    pub channel_id: ChannelId,
    pub view: ClipView,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SetClipViewRequest {
    pub view: ClipView,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ClipUploadRequest {
    #[ts(type = "number")]
    pub size: u64,
    pub duration_ms: u32,
}

/// An upload ticket: PUT the file to `url`, then confirm.
#[derive(Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ClipUpload {
    pub id: ClipUploadId,
    pub url: String,
    pub content_type: String,
    pub expires_at: String,
}
// `url` is a live presigned link.
redacting_debug!(ClipUpload { id, content_type, expires_at ; url });

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ConfirmClipRequest {
    pub caption: String,
    pub game_id: Option<GameId>,
    pub duration_ms: u32,
    pub width: u32,
    pub height: u32,
    pub thumb_hash: String,
    #[serde(default)]
    pub nonce: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct EditClipRequest {
    pub caption: String,
    pub game_id: Option<GameId>,
}

/// A short-lived URL to play a clip.
#[derive(Clone, PartialEq, Serialize, Deserialize, TS)]
pub struct ClipUrl {
    pub url: String,
    pub expires_at: String,
}
redacting_debug!(ClipUrl { expires_at ; url });

/// Filter chips of a clips channel: clip counts per game and per author.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ClipFacets {
    pub games: Vec<GameCount>,
    pub authors: Vec<AuthorCount>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct GameCount {
    pub game_id: GameId,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AuthorCount {
    pub user_id: UserId,
    pub count: u32,
}

/// How much clip storage a server uses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ClipStorage {
    #[ts(type = "number")]
    pub bytes: u64,
    pub count: u32,
    pub est_monthly_usd: f64,
}

/// One line of a quote: a speaker (a member or a plain name) and either text or an image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct QuoteLine {
    pub id: LineId,
    pub speaker_id: Option<UserId>,
    pub speaker_name: Option<String>,
    pub text: Option<String>,
    pub media: Option<LineMedia>,
    pub caption: Option<String>,
    pub awards: u32,
    /// The counted awarders, oldest first; `awards == awarders.len()`.
    pub awarders: Vec<UserId>,
    /// Per caller: you gave this line an award (gateway events carry `false`).
    pub awarded: bool,
}

/// An image line's picture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct LineMedia {
    pub hash: String,
    /// The first frame, set only for an animated image.
    pub still_hash: Option<String>,
    pub width: u32,
    pub height: u32,
}

/// A line as sent by a client. `id` is set when editing a line that already exists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct NewQuoteLine {
    pub id: Option<LineId>,
    pub speaker_id: Option<UserId>,
    pub speaker_name: Option<String>,
    pub text: Option<String>,
    pub media_hash: Option<String>,
    pub caption: Option<String>,
}

/// `POST /channels/{id}/quotes`. `today` is the client's local `YYYY-MM-DD`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PostQuoteRequest {
    pub lines: Vec<NewQuoteLine>,
    pub today: Option<String>,
    pub nonce: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct EditQuoteRequest {
    pub lines: Vec<NewQuoteLine>,
}

/// A plain name a server's members agreed means one of them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SpeakerLink {
    pub server_id: ServerId,
    pub name_key: String,
    pub name: String,
    pub user_id: UserId,
    pub linked_by: UserId,
    pub linked_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct LinkSpeakerRequest {
    pub name: String,
    pub user_id: UserId,
}

/// Why something is in your inbox (#306): a typed `@you`, or only a reply to your message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum InboxKind {
    Mention,
    Reply,
}

/// The message an inbox item replied to, cut to one short line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct InboxSnippet {
    pub message_id: MessageId,
    pub author_id: Option<UserId>,
    /// At most [`INBOX_SNIPPET_CHARS`] characters; empty when it was deleted.
    pub content: String,
    pub deleted: bool,
}

/// How long an inbox snippet of the replied-to message may be.
pub const INBOX_SNIPPET_CHARS: usize = 140;

/// One thing meant for you (#306): a message that mentions you or replies to you. Where it is comes from
/// `message.channel_id` (and `message.thread_root_id` for a reply in a quote's or clip's thread).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct InboxItem {
    pub message: Message,
    pub kind: InboxKind,
    /// What it replied to, when it is a reply.
    pub replied_to: Option<InboxSnippet>,
    /// Read: opened from the inbox, covered by your read point (or thread seen point), or marked all read.
    pub read: bool,
}

/// One of your saved inbox filters (#307): the inbox's All and Mentions tabs narrowed to messages from these people
/// and/or in these chats (either empty: any), and only unread ones when `unread_only`. Named by you; the name is its
/// key (unique ignoring case). Yours only, on every device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct InboxFilter {
    pub name: String,
    #[serde(default)]
    pub people: Vec<UserId>,
    #[serde(default)]
    pub channels: Vec<ChannelId>,
    #[serde(default)]
    pub unread_only: bool,
}

/// `PUT /me/inbox-filters` (#307): your whole list of saved filters, in order.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct InboxFiltersRequest {
    pub filters: Vec<InboxFilter>,
}

/// Limits on saved inbox filters (#307).
pub mod inbox_filter_limits {
    /// Saved filters one person may have.
    pub const MAX_FILTERS: usize = 10;
    /// Characters in a filter's name, after trimming.
    pub const NAME_CHARS: usize = 32;
    /// People, and chats, one filter may name (each).
    pub const MAX_PER_FILTER: usize = 25;
}

/// `GET /inbox`: a page of your inbox, newest first, and how many items are unread in all.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct InboxPage {
    pub items: Vec<InboxItem>,
    pub unread: u32,
}

/// A reply thread you're part of with replies you haven't seen (#326): a quote's or clip's thread you started (you
/// posted the quote or clip) or replied in, with replies by others since you last looked (your seen point, or your own
/// last reply in it, whichever is later).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct InboxThread {
    /// The quote or clip the thread is on, one short line.
    pub root: InboxSnippet,
    pub channel_id: ChannelId,
    /// The newest reply you haven't seen.
    pub latest: Message,
    /// How many replies by others you haven't seen.
    pub new_replies: u32,
    /// You posted the quote or clip (otherwise you replied in it).
    pub started: bool,
}

/// `GET /inbox/threads` (#326): the threads you're part of with new replies, newest reply first; and every thread
/// you're part of (`watching`, newest first, capped), so a client knows which new replies are worth asking again for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct InboxThreads {
    pub threads: Vec<InboxThread>,
    pub watching: Vec<MessageId>,
}

/// `GET /inbox/unread`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct InboxUnread {
    pub unread: u32,
}

/// `POST /inbox/read-all`: every item up to `upto` (the newest one you were shown) is read; none: up to now.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct InboxReadAllRequest {
    #[serde(default)]
    pub upto: Option<MessageId>,
}

/// "I have read this quote's thread up to here."
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ThreadSeenRequest {
    pub message_id: MessageId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum QotyEndReason {
    Early,
    Time,
}

/// A Quote of the Year vote for one year in one server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct QotyEvent {
    pub id: QotyEventId,
    pub server_id: ServerId,
    pub year: i32,
    pub started_by: UserId,
    pub started_at: String,
    /// `None` = open until ended.
    pub ends_at: Option<String>,
    pub ended_at: Option<String>,
    pub ended_by: Option<UserId>,
    pub end_reason: Option<QotyEndReason>,
    /// Until when an ended vote can be reopened.
    pub reopen_until: Option<String>,
    pub options: Vec<QotyOption>,
    /// Members who have voted.
    pub voters: u32,
    /// Members of the server.
    pub members: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct QotyOption {
    pub message: Message,
    pub added_by: Option<UserId>,
    pub votes: u32,
    /// In vote order; `votes == voters.len()`.
    pub voters: Vec<UserId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StartQotyRequest {
    pub year: i32,
    /// `None` = open until ended.
    pub minutes: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ReopenQotyRequest {
    pub minutes: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct QotyVoteRequest {
    pub message_id: MessageId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AddQotyOptionRequest {
    pub message_id: MessageId,
}

/// The payload of a Quote of the Year result card (a system message).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct QotyCard {
    pub event_id: QotyEventId,
    pub year: i32,
    /// `None` when nobody voted.
    pub message_id: Option<MessageId>,
    pub votes: u32,
    pub voters: Vec<UserId>,
    pub reason: QotyEndReason,
    /// Who said each line of the winner when the vote ended, in line order. Empty on a no-vote
    /// card (and on cards from before it was kept).
    #[serde(default)]
    pub speakers: Vec<QuoteSpeaker>,
}

/// Who said a line: a member, a plain name, or both (a plain name linked to a member).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct QuoteSpeaker {
    pub user_id: Option<UserId>,
    pub name: Option<String>,
}

/// How many times a speaker (a member, or a plain name) appears.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SpeakerCount {
    pub user_id: Option<UserId>,
    pub name: Option<String>,
    pub count: u32,
}

/// `GET /servers/{id}/quotes/leaderboard?period=…&today=YYYY-MM-DD`: which quotes `top` and
/// `most_quoted` count, by the day they were posted, in the caller's local time. A week starts on
/// Monday.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LeaderboardPeriod {
    Week,
    Month,
    Year,
    All,
}

/// A quote on the leaderboard and its score: how many people reacted to it (one each, however
/// many emoji they used; replies don't count).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct RankedQuote {
    pub message: Message,
    pub score: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Leaderboard {
    pub winners: Vec<LeaderboardWin>,
    /// Without a `period`: this year's best and the all-time best. Empty with one.
    pub top_year: Vec<Message>,
    pub top_all_time: Vec<Message>,
    /// With a `period`: that period's best, with scores. Empty without one, and from older servers.
    #[serde(default)]
    pub top: Vec<RankedQuote>,
    /// Quotes per person (a quote counts once for each person in it), over the `period` if given.
    pub most_quoted: Vec<SpeakerCount>,
    pub most_awarded: Vec<SpeakerCount>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct LeaderboardWin {
    pub year: i32,
    pub message: Message,
    pub votes: u32,
    pub voters: Vec<UserId>,
    /// Who said each line when the vote ended, in line order: the win stays theirs whatever
    /// later edits and links do.
    pub speakers: Vec<QuoteSpeaker>,
}

// ---------- Link embeds ----------

/// Who a link preview is from. A provider this build doesn't know reads as `Unknown`: a card at
/// most, never a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EmbedProvider {
    Youtube,
    Tiktok,
    Spotify,
    Soundcloud,
    Twitch,
    Instagram,
    /// Any other site: an OpenGraph card.
    Generic,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EmbedKind {
    /// Title, description and thumbnail only.
    Card,
    /// A card whose thumbnail starts the provider's player (through the server's embed page).
    Player,
    /// A kind this build doesn't know: shown as a card.
    #[serde(other)]
    Unknown,
}

/// A preview's thumbnail: a `link_thumb` media hash, a still WebP.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EmbedThumb {
    pub hash: String,
    pub width: u32,
    pub height: u32,
}

/// One link's preview, built by the server from the provider's oEmbed or the page's OpenGraph
/// tags. Plain text throughout: the client never renders any of it as HTML.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct LinkEmbed {
    /// The message's link, normalised (parsed and printed back as a URL: scheme and host
    /// lowercased, a default port dropped, no fragment), so it may differ from the text as typed.
    pub url: String,
    pub provider: EmbedProvider,
    pub kind: EmbedKind,
    /// The provider's validated id for its player (a player has one).
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub site_name: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub thumb: Option<EmbedThumb>,
    /// YouTube start, seconds.
    #[serde(default)]
    pub start: Option<u32>,
    /// A 9:16 player (TikTok, Shorts, reels).
    #[serde(default)]
    pub vertical: bool,
}

/// `POST /messages/{id}/embeds/hide`: the sender hides one preview, or all of them. `POST
/// /messages/{id}/embeds/show` (no body) brings back every hidden one whose link is still in the
/// message (#94).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct HideEmbedRequest {
    /// The preview's `url`; `None` hides every preview on the message.
    #[serde(default)]
    pub url: Option<String>,
}

// ---------- Message search ----------

/// What a search result must carry (`has:` in the box). Sent as one comma-separated `has`
/// parameter of these words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SearchHas {
    Clip,
    Quote,
    Link,
    Media,
    File,
}

impl SearchHas {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clip => "clip",
            Self::Quote => "quote",
            Self::Link => "link",
            Self::Media => "media",
            Self::File => "file",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "clip" => Self::Clip,
            "quote" => Self::Quote,
            "link" => Self::Link,
            "media" => Self::Media,
            "file" => Self::File,
            _ => return None,
        })
    }
}

/// One search, as the UI hands it to the core: the box's filters already parsed out of `q`.
/// All of them must match. Which route (server or conversation) is the caller's choice.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct SearchRequest {
    /// The words: all must appear (prefix matches); a "quoted phrase" must appear as is.
    #[serde(default)]
    pub q: String,
    /// Written by (in a quotes channel, also said by) this person.
    #[serde(default)]
    pub from: Option<UserId>,
    /// One channel of the server (server search only).
    #[serde(default)]
    pub in_channel: Option<ChannelId>,
    #[serde(default)]
    pub has: Vec<SearchHas>,
    /// Messages that pinged this person.
    #[serde(default)]
    pub mentions: Option<UserId>,
    /// The `next_cursor` of the page before (opaque).
    #[serde(default)]
    pub cursor: Option<String>,
}

/// A matched stretch of [`SearchHit::text`], in UTF-16 code units (JavaScript string offsets).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct HighlightRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SearchHit {
    pub message: Message,
    /// The searchable text as shown: custom emoji as `:name:`, then any quote image captions,
    /// one per line. Plain text: the client never renders it as HTML.
    pub text: String,
    pub highlights: Vec<HighlightRange>,
    /// For a reply in a quote's or clip's thread: that quote or clip.
    pub root: Option<Message>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SearchResults {
    /// Newest first. A page can be shorter than the page size and still have more after it.
    pub hits: Vec<SearchHit>,
    /// Pass as `cursor` for the next page (opaque); `None` once there is nothing older.
    pub next_cursor: Option<String>,
    /// How many match, counted up to 51 (show "50+" at 51). First page only.
    pub total: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The card glow (#156) next to 0.2.x: an older server's card (no `glow`) reads as glowing, a
    /// card with it still reads in an older client (serde skips the field it doesn't know), and an
    /// older client's save (no `glow`) keeps the current setting.
    #[test]
    fn card_glow_is_optional_both_ways() {
        let old_card = serde_json::json!({
            "banner_hash": null, "banner_still": null, "theme_top": "#3a3d46",
            "theme_bottom": "#16171b", "pronouns": null, "bio": null,
        });
        let card: CardProfile = serde_json::from_value(old_card).unwrap();
        assert!(card.glow);

        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct OldCardProfile {
            banner_hash: Option<String>,
            banner_still: Option<String>,
            theme_top: String,
            theme_bottom: String,
            pronouns: Option<String>,
            bio: Option<String>,
        }
        let off = CardProfile {
            glow: false,
            ..card
        };
        let json = serde_json::to_value(&off).unwrap();
        assert_eq!(json["glow"], false);
        let old: OldCardProfile = serde_json::from_value(json).unwrap();
        assert_eq!(old.theme_top, "#3a3d46");

        let old_save = serde_json::json!({
            "avatar_hash": null, "banner_hash": null, "theme_top": "#3a3d46",
            "theme_bottom": "#16171b", "style": {"font": null, "colors": []},
            "pronouns": null, "bio": null,
        });
        let req: UpdateProfileRequest = serde_json::from_value(old_save).unwrap();
        assert_eq!(req.glow, None);
        // and a save that doesn't touch it says nothing about it
        assert!(serde_json::to_value(&req).unwrap().get("glow").is_none());
    }

    fn plain_message() -> Message {
        serde_json::from_value(serde_json::json!({
            "id": MessageId::new(), "channel_id": ChannelId::new(), "author_id": null,
            "kind": "normal", "content": "hi", "reply_to_id": null,
            "created_at": "2026-11-11T10:00:00.000Z", "edited_at": null, "deleted": false,
        }))
        .unwrap()
    }

    /// A message with no attachments looks as it did before them (keeps every frame small);
    /// one with attachments round-trips.
    #[test]
    fn message_attachments_round_trip_and_stay_off_the_wire_when_empty() {
        let m = plain_message();
        let json = serde_json::to_value(&m).unwrap();
        assert!(json.get("attachments").is_none(), "{json}");
        assert!(json.get("caption_is_fallback").is_none(), "{json}");
        assert!(m.attachments.is_empty() && !m.caption_is_fallback);

        let with = Message {
            attachments: vec![Attachment {
                id: AttachmentId::new(),
                name: "cat.png".into(),
                content_type: "image/png".into(),
                kind: AttachmentKind::Image,
                size: 1234,
                width: Some(10),
                height: Some(20),
                duration_ms: None,
                expires_at: None,
                expired: false,
                spoiler: false,
                view_once: false,
            }],
            caption_is_fallback: true,
            ..m
        };
        let json = serde_json::to_value(&with).unwrap();
        assert_eq!(json["attachments"][0]["kind"], "image");
        assert_eq!(json["caption_is_fallback"], true);
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), with);
    }

    /// Sends from before attachments have no `attachments` key.
    #[test]
    fn send_request_without_attachments_defaults_to_none() {
        let r: SendMessageRequest =
            serde_json::from_str(r#"{"content":"hi","reply_to_id":null,"nonce":null}"#).unwrap();
        assert!(r.attachments.is_empty());
    }

    /// #259: weighted slices cover their share of the turn; equal weights read as the unweighted wheel, and an old
    /// spin's payload (no weights or palette) still parses and writes neither.
    #[test]
    fn a_weighted_wheel_reads_each_slices_share() {
        // weights 1, 3: slice 0 covers [0, 90), slice 1 [90, 360) clockwise from 12 o'clock
        assert_eq!(wheel_winner_at_weighted(360.0 - 45.0, &[1, 3]), 0);
        assert_eq!(wheel_winner_at_weighted(360.0 - 91.0, &[1, 3]), 1);
        assert_eq!(wheel_winner_at_weighted(1.0, &[1, 3]), 1);
        assert_eq!(wheel_winner_at_weighted(0.0, &[1, 3]), 0);
        for n in 2..=20usize {
            let ones = vec![1; n];
            for k in 0..360 {
                let a = k as f64 + 0.5;
                assert_eq!(wheel_winner_at_weighted(a, &ones), wheel_winner_at(a, n));
            }
        }
        assert_eq!(
            wheel_winner_at_weighted(10.0, &[0, 0]),
            wheel_winner_at(10.0, 2)
        );
        let old: WheelSpin =
            serde_json::from_str(r#"{"options":["a","b"],"winner":1,"angle":10.0}"#).unwrap();
        assert_eq!((old.weights.clone(), old.palette.clone()), (None, None));
        let json = serde_json::to_string(&old).unwrap();
        assert!(
            !json.contains("weights") && !json.contains("palette"),
            "{json}"
        );
    }

    /// The pointer reads slice i of n from a clockwise turn: no turn shows slice 0 at the top's right edge; a
    /// turn just past one slice's width backwards (360 − s·i − ε) shows slice i.
    #[test]
    fn wheel_winner_at_reads_the_slice_under_the_pointer() {
        assert_eq!(wheel_winner_at(0.0, 4), 0);
        assert_eq!(wheel_winner_at(359.0, 4), 0);
        assert_eq!(wheel_winner_at(271.0, 4), 0);
        assert_eq!(wheel_winner_at(269.0, 4), 1);
        assert_eq!(wheel_winner_at(180.0 - 1.0, 4), 2);
        assert_eq!(wheel_winner_at(1.0, 4), 3);
        // whole extra turns and negative angles read the same
        assert_eq!(wheel_winner_at(5.0 * 360.0 + 269.0, 4), 1);
        assert_eq!(wheel_winner_at(-91.0, 4), 1);
        for n in 2..=100 {
            let seg = 360.0 / n as f64;
            for i in 0..n {
                let mid = seg * (i as f64 + 0.5);
                assert_eq!(wheel_winner_at(360.0 - mid, n), i, "n={n} i={i}");
            }
        }
    }

    /// Badge art (#225): an older server sends no `art`, which reads as none; a picture's hash
    /// round-trips.
    #[test]
    fn badge_def_carries_optional_art() {
        let older = serde_json::json!({
            "key": "founder", "name": "Founder", "description": "Made Pulse.", "icon": "👑"
        });
        let d: BadgeDef = serde_json::from_value(older).unwrap();
        assert_eq!(d.art, None);
        let hash = "ab".repeat(32);
        let with = BadgeDef {
            art: Some(hash.clone()),
            ..d
        };
        let json = serde_json::to_value(&with).unwrap();
        assert_eq!(json["art"], hash);
        assert_eq!(serde_json::from_value::<BadgeDef>(json).unwrap(), with);
    }

    /// One of a kind (#239): an older server sends no flag, which reads as not unique; the flag
    /// round-trips.
    #[test]
    fn badge_def_carries_one_of_a_kind() {
        let older = serde_json::json!({
            "key": "founder", "name": "Founder", "description": "Made Pulse.", "icon": "👑"
        });
        let d: BadgeDef = serde_json::from_value(older).unwrap();
        assert!(!d.one_of_a_kind);
        let with = BadgeDef {
            one_of_a_kind: true,
            ..d
        };
        let json = serde_json::to_value(&with).unwrap();
        assert_eq!(json["one_of_a_kind"], true);
        assert_eq!(serde_json::from_value::<BadgeDef>(json).unwrap(), with);
    }

    /// Hidden-badge hints (#211): an older server sends no `hint`, which reads as none; a hint
    /// round-trips.
    #[test]
    fn badge_def_carries_optional_hint() {
        let older = serde_json::json!({
            "key": "hidden:0", "name": "???", "description": "", "icon": "❔", "hidden": true
        });
        let d: BadgeDef = serde_json::from_value(older).unwrap();
        assert_eq!(d.hint, None);
        let with = BadgeDef {
            hint: Some("Keep at it.".into()),
            ..d
        };
        let json = serde_json::to_value(&with).unwrap();
        assert_eq!(json["hint"], "Keep at it.");
        assert_eq!(serde_json::from_value::<BadgeDef>(json).unwrap(), with);
    }

    /// Debug output can end up in logs, so it never shows passwords, codes or tickets.
    #[test]
    fn debug_never_shows_secrets() {
        let s = |v: &str| v.to_string();
        let shown = [
            format!(
                "{:?}",
                RegisterRequest {
                    invite_code: s("SECRET-INVITE"),
                    username: s("alex"),
                    password: s("SECRET-PW"),
                    website: None,
                }
            ),
            format!(
                "{:?}",
                LoginRequest {
                    username: s("alex"),
                    password: s("SECRET-PW"),
                }
            ),
            format!(
                "{:?}",
                TwoFactorSetupRequest {
                    password: s("SECRET-PW"),
                }
            ),
            format!(
                "{:?}",
                TwoFactorSetupResponse {
                    secret: s("SECRET-B32"),
                    otpauth_uri: s("otpauth://totp/Pulse:alex?secret=SECRET-B32"),
                }
            ),
            format!(
                "{:?}",
                TwoFactorCodeRequest {
                    code: s("SECRET-CODE")
                }
            ),
            format!(
                "{:?}",
                TwoFactorEnabledResponse {
                    recovery_codes: vec![s("SECRET-RC")],
                    signed_out: 2,
                }
            ),
            format!(
                "{:?}",
                RecoveryCodesResponse {
                    recovery_codes: vec![s("SECRET-RC")],
                }
            ),
            format!(
                "{:?}",
                MfaRequired {
                    code: s("mfa_required"),
                    message: s("enter your code"),
                    ticket: s("SECRET-TICKET"),
                }
            ),
            format!(
                "{:?}",
                LoginMfaRequest {
                    ticket: s("SECRET-TICKET"),
                    code: s("SECRET-CODE"),
                }
            ),
            format!(
                "{:?}",
                SessionResponse {
                    token: s("SECRET-TOKEN"),
                    user: User {
                        id: UserId::new(),
                        username: s("alex"),
                        display_name: String::new(),
                        avatar_hash: None,
                        avatar_still: None,
                        style: Default::default(),
                    },
                }
            ),
            format!(
                "{:?}",
                VoiceTokenResponse {
                    url: s("wss://livekit.example"),
                    token: s("SECRET-JWT"),
                }
            ),
            format!(
                "{:?}",
                InviteResponse {
                    code: s("SECRET-INVITE"),
                }
            ),
            format!(
                "{:?}",
                ChangePasswordRequest {
                    current_password: s("SECRET-PW"),
                    new_password: s("SECRET-NEW"),
                    code: Some(s("SECRET-CODE")),
                }
            ),
        ];
        for d in &shown {
            assert!(!d.contains("SECRET"), "{d}");
        }
        assert!(shown[9].contains("alex"), "{}", shown[9]);
        assert!(shown[10].contains("livekit.example"), "{}", shown[10]);
        // The harmless fields are still there to debug with.
        assert!(shown[1].contains("alex"), "{}", shown[1]);
        assert!(shown[5].contains("signed_out: 2"), "{}", shown[5]);
        assert!(shown[7].contains("mfa_required"), "{}", shown[7]);
    }

    /// A clip's upload and play links are live presigned URLs (key id and signature).
    #[test]
    fn debug_never_shows_signed_urls() {
        let url = "https://bucket.example/clips/k.mp4?X-Amz-Credential=SECRET-KEY&X-Amz-Signature=SECRET-SIG";
        let up = format!(
            "{:?}",
            ClipUpload {
                id: crate::ids::ClipUploadId::new(),
                url: url.into(),
                content_type: "video/mp4".into(),
                expires_at: "2026-10-28T11:00:00.000Z".into(),
            }
        );
        let play = format!(
            "{:?}",
            ClipUrl {
                url: url.into(),
                expires_at: "2026-10-28T11:00:00.000Z".into(),
            }
        );
        for d in [&up, &play] {
            assert!(!d.contains("SECRET"), "{d}");
            assert!(d.contains("expires_at"), "{d}");
        }
        assert!(up.contains("video/mp4"), "{up}");
    }

    /// The `has` filter travels as one comma-separated query parameter of these words.
    #[test]
    fn search_has_round_trips() {
        for (h, w) in [
            (SearchHas::Clip, "clip"),
            (SearchHas::Quote, "quote"),
            (SearchHas::Link, "link"),
            (SearchHas::Media, "media"),
            (SearchHas::File, "file"),
        ] {
            assert_eq!(serde_json::to_string(&h).unwrap(), format!("\"{w}\""));
            assert_eq!(h.as_str(), w);
            assert_eq!(SearchHas::parse(w), Some(h));
        }
        assert_eq!(SearchHas::parse("emoji"), None);
    }

    /// A provider a newer server knows reads as `Unknown` here (a card at most, never a player).
    #[test]
    fn an_unknown_embed_provider_parses() {
        let e: LinkEmbed = serde_json::from_str(
            r#"{"url":"https://vimeo.example/1","provider":"vimeo","kind":"player",
            "provider_id":"1","title":"t","vertical":false}"#,
        )
        .unwrap();
        assert_eq!(e.provider, EmbedProvider::Unknown);
        assert_eq!(e.kind, EmbedKind::Player);
    }

    /// A kind a newer server knows reads as `Unknown` here (shown as a card), rather than an
    /// error that would lose the whole message.
    #[test]
    fn an_unknown_embed_kind_parses() {
        let e: LinkEmbed = serde_json::from_str(
            r#"{"url":"https://example.com/a","provider":"youtube","kind":"gallery",
            "provider_id":"dQw4w9WgXcQ"}"#,
        )
        .unwrap();
        assert_eq!(e.kind, EmbedKind::Unknown);
        assert_eq!(e.provider, EmbedProvider::Youtube);
    }

    /// Everything after `kind` may be left out.
    #[test]
    fn a_bare_link_embed_parses() {
        let e: LinkEmbed = serde_json::from_str(
            r#"{"url":"https://example.com/a","provider":"generic","kind":"card"}"#,
        )
        .unwrap();
        assert_eq!(
            e,
            LinkEmbed {
                url: "https://example.com/a".into(),
                provider: EmbedProvider::Generic,
                kind: EmbedKind::Card,
                provider_id: None,
                title: None,
                description: None,
                site_name: None,
                author: None,
                thumb: None,
                start: None,
                vertical: false,
            }
        );
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["provider"], "generic");
        assert_eq!(json["kind"], "card");
    }

    /// A channel from an older server (no categories yet) parses, and sends no category back.
    #[test]
    fn a_channel_without_a_category_parses() {
        let id = ChannelId::new();
        let c: Channel = serde_json::from_str(&format!(
            r#"{{"id":"{id}","server_id":null,"kind":"text","name":"general","position":0}}"#
        ))
        .unwrap();
        assert_eq!(c.category_id, None);
        assert!(!serde_json::to_string(&c).unwrap().contains("category_id"));
    }

    /// A channel from an older server has no bitrate (#75), and Default sends none back, so
    /// older clients see the channel as before.
    #[test]
    fn a_channel_bitrate_is_optional_both_ways() {
        let id = ChannelId::new();
        let mut c: Channel = serde_json::from_str(&format!(
            r#"{{"id":"{id}","server_id":null,"kind":"voice","name":"Lounge","position":0}}"#
        ))
        .unwrap();
        assert_eq!(c.bitrate, None);
        assert!(!serde_json::to_string(&c).unwrap().contains("bitrate"));
        c.bitrate = Some(256);
        let back: Channel = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.bitrate, Some(256));
    }

    /// An older client asking for a channel names no category: it goes uncategorised.
    #[test]
    fn create_channel_without_a_category_parses() {
        let r: CreateChannelRequest =
            serde_json::from_str(r#"{"kind":"voice","name":"Lounge"}"#).unwrap();
        assert_eq!(r.category_id, None);
    }
}

#[cfg(test)]
mod sound_tests {
    use super::*;

    #[test]
    fn update_sound_tells_absent_from_null() {
        let absent: UpdateSoundRequest = serde_json::from_str(r#"{"name":"x"}"#).unwrap();
        assert_eq!(absent.emoji, None);
        let cleared: UpdateSoundRequest = serde_json::from_str(r#"{"emoji":null}"#).unwrap();
        assert_eq!(cleared.emoji, Some(None));
        let set: UpdateSoundRequest = serde_json::from_str(r#"{"emoji":"📯"}"#).unwrap();
        assert_eq!(set.emoji, Some(Some("📯".into())));
        assert_eq!(
            serde_json::to_string(&cleared).unwrap(),
            r#"{"emoji":null}"#
        );
        assert!(!serde_json::to_string(&absent).unwrap().contains("emoji"));
    }

    #[test]
    fn create_sound_volume_defaults_to_100() {
        let r: CreateSoundRequest =
            serde_json::from_str(r#"{"name":"a","media_hash":"h"}"#).unwrap();
        assert_eq!((r.volume, r.emoji), (100, None));
    }
}

// ---------- Steam (#379) ----------

/// `GET /me/steam` and friends: what this server can do with Steam, and your link.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SteamStatus {
    /// The server can link Steam accounts (it knows its public URL, which Steam sends you back to).
    pub linking_available: bool,
    /// The server has a Steam Web API key, which achievements need.
    pub achievements_available: bool,
    /// Your linked account; `None` when you haven't linked one.
    pub link: Option<SteamLinked>,
    /// Deadlock stats (#390) can be read right now: deadlock-api answers. Absent from older servers: no.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub deadlock_available: bool,
}

/// A linked Steam account, as its owner sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SteamLinked {
    /// The 64-bit SteamID, as digits.
    pub steam_id: String,
    /// The Steam persona name, when the server could read it.
    pub persona: Option<String>,
    /// "Share Steam achievements" (default on).
    pub share_achievements: bool,
    /// Steam refuses to show this account's game details: achievements can't be read until they're public.
    pub private: bool,
    /// The one-time read of the games already played has finished.
    pub backfill_done: bool,
    /// Account completion rate, 0 to 100, once there's a game to count.
    pub completion_rate: Option<f32>,
    pub perfect_games: u32,
    /// "Share Steam hours" (#383, default on). Absent from older servers: on.
    #[serde(default = "yes")]
    #[ts(as = "Option<bool>", optional)]
    pub share_hours: bool,
    /// Minutes played in Steam games over the last two weeks, once read (with `share_hours`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recent_minutes: Option<u32>,
    /// "Share Join" (#381, default on): friends are offered Join on your public Steam lobby or server. Absent from
    /// older servers: on.
    #[serde(default = "yes")]
    #[ts(as = "Option<bool>", optional)]
    pub share_join: bool,
    /// Deadlock post-match cards (#390, default on). Absent from older servers: on.
    #[serde(default = "yes")]
    #[ts(as = "Option<bool>", optional)]
    pub deadlock_matches: bool,
    /// Deadlock rank badge and the weekly rank movers (#390, default on). Absent from older servers: on.
    #[serde(default = "yes")]
    #[ts(as = "Option<bool>", optional)]
    pub deadlock_rank: bool,
    /// Deadlock squad stats (#390, default on). Absent from older servers: on.
    #[serde(default = "yes")]
    #[ts(as = "Option<bool>", optional)]
    pub deadlock_squad: bool,
}

/// `POST /me/steam/link`: the Steam sign-in page to open in the browser.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SteamLinkStart {
    pub url: String,
}

/// `PUT /me/steam/settings`: the switches to change; one left out stays as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SteamSettingsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub share_achievements: Option<bool>,
    /// "Share Steam hours" (#383).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub share_hours: Option<bool>,
    /// "Share Join" (#381).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub share_join: Option<bool>,
    /// Deadlock post-match cards (#390).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub deadlock_matches: Option<bool>,
    /// Deadlock rank badge and rank movers (#390).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub deadlock_rank: Option<bool>,
    /// Deadlock squad stats (#390).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub deadlock_squad: Option<bool>,
}

/// `GET /servers/{id}/steam-hours` (#383): the server's members who played the most Steam hours over the last two
/// weeks, most first (only those who share Steam hours and played some).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SteamHours {
    pub entries: Vec<SteamHoursEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SteamHoursEntry {
    pub user_id: UserId,
    /// Minutes over the last two weeks.
    pub minutes: u32,
}

/// `GET /servers/{id}/achievements-channel`: where the server's Steam achievement cards go; `None` = nowhere.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct AchievementsChannel {
    pub channel_id: Option<ChannelId>,
}

/// `PUT /servers/{id}/achievements-channel` (the server's owner only): a text channel of that server, or `None`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SetAchievementsChannelRequest {
    pub channel_id: Option<ChannelId>,
}

/// A Steam achievement card (#379): someone's new unlocks in one game, or (`complete`) their 100 % of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AchievementCard {
    pub user_id: UserId,
    pub app_id: u32,
    pub game: String,
    /// The unlocks shown (at most 5); empty on a 100 % card.
    #[serde(default)]
    pub achievements: Vec<AchievementEntry>,
    /// Unlocked in the same go but not shown.
    #[serde(default)]
    pub more: u32,
    pub unlocked: u32,
    pub total: u32,
    /// The 100 % card.
    #[serde(default)]
    pub complete: bool,
    /// The game's header art on a 100 % card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub art: Option<EmbedThumb>,
}

/// One achievement on a card.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AchievementEntry {
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<EmbedThumb>,
    /// The share of all players who have it, 0 to 100.
    pub global_percent: Option<f32>,
}

/// F1 race weekends (#386): the numbers both sides use.
pub mod f1_limits {
    /// Picks in a podium prediction: P1, P2, P3.
    pub const PODIUM: usize = 3;
    /// How long before lights out the "lights out in 1 hour" line goes, in minutes.
    pub const LIGHTS_OUT_MINUTES: i64 = 60;
    /// Points for a pick in its exact place.
    pub const EXACT_POINTS: u32 = 3;
    /// Points for a pick on the podium, in another place.
    pub const PODIUM_POINTS: u32 = 1;
    /// Rows on a results card (the top 10) and in its standings snapshot.
    pub const RESULT_ROWS: usize = 10;
}

/// One session of a race weekend (#386).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum F1SessionKind {
    Practice1,
    Practice2,
    Practice3,
    SprintQualifying,
    Sprint,
    Qualifying,
    Race,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct F1Session {
    pub kind: F1SessionKind,
    /// UTC, RFC 3339; clients show it in the viewer's own time.
    pub starts_at: String,
}

/// A race weekend from the season's calendar (Jolpica-F1, through the server).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct F1Weekend {
    pub season: u16,
    pub round: u16,
    pub race_name: String,
    pub circuit: String,
    /// The town ("Las Vegas").
    pub locality: String,
    /// As Jolpica names it ("USA", "UK", "Australia"); clients pick the flag.
    pub country: String,
    /// Every session with a time, in order; the race is last.
    pub sessions: Vec<F1Session>,
    /// Lights out (UTC, RFC 3339). Podium predictions close then.
    pub race_at: String,
}

/// A driver as a card shows them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct F1Driver {
    /// Jolpica's `driverId` ("max_verstappen"): what a prediction names.
    pub id: String,
    /// The three letters ("VER").
    #[serde(default)]
    pub code: Option<String>,
    /// "Given Family".
    pub name: String,
    #[serde(default)]
    pub team: Option<String>,
}

/// Someone's podium prediction on a weekend card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct F1Prediction {
    pub user_id: UserId,
    /// Driver ids for P1, P2, P3.
    pub picks: Vec<String>,
    /// Once the race is scored: 3 a pick in its exact place, 1 a pick on the podium elsewhere.
    #[serde(default)]
    pub points: Option<u32>,
}

/// The "race weekend" card, posted in the server's F1 channel as the weekend starts. Everyone sees the same card
/// (predictions are open), so `F1PredictionsChanged` carries the predictions whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct F1WeekendCard {
    pub weekend: F1Weekend,
    /// Who can be picked: the last race's field (or the season's entry list before the first race).
    pub drivers: Vec<F1Driver>,
    /// In the order they were made; filled when read (never stored on the line).
    #[serde(default)]
    pub predictions: Vec<F1Prediction>,
    /// When predictions close (UTC, RFC 3339), by the server's choice (#403): lights out, or qualifying's start. Filled
    /// when read; older servers never send it (they close at lights out).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub picks_close_at: Option<String>,
}

/// The "lights out in 1 hour" line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct F1LightsOut {
    pub season: u16,
    pub round: u16,
    pub race_name: String,
    pub race_at: String,
    /// The weekend card in the same channel, to jump to (none when it was never posted).
    #[serde(default)]
    pub weekend_message_id: Option<MessageId>,
}

/// One finisher on a results card.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct F1ResultRow {
    pub position: u16,
    pub driver: F1Driver,
    /// The race time for the winner, the gap for the rest ("+5.2s"), or the status ("+1 Lap", "Retired").
    pub time: String,
    pub points: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct F1FastestLap {
    pub driver: F1Driver,
    #[serde(default)]
    pub lap: Option<u16>,
    /// "1:12.983".
    pub time: String,
}

/// One row of the drivers' championship.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct F1StandingRow {
    pub position: u16,
    pub driver: F1Driver,
    pub points: f32,
    pub wins: u16,
}

/// The podium and results card, posted once the race's results are in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct F1ResultsCard {
    pub season: u16,
    pub round: u16,
    pub race_name: String,
    pub country: String,
    /// The top 10, in order.
    pub results: Vec<F1ResultRow>,
    #[serde(default)]
    pub fastest_lap: Option<F1FastestLap>,
    /// The drivers' championship after this round, top 10 (empty when it wasn't out in time).
    #[serde(default)]
    pub standings: Vec<F1StandingRow>,
    /// The constructors' championship after this round (#403), every team (empty when it wasn't out in time, and on
    /// cards from before it was added).
    #[serde(default)]
    pub constructor_standings: Vec<F1ConstructorRow>,
}

/// One row of the constructors' championship (#403).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct F1ConstructorRow {
    pub position: u16,
    /// The team as Jolpica names it ("McLaren").
    pub team: String,
    pub points: f32,
    pub wins: u16,
}

/// The sprint's results card (#403), posted once a sprint weekend's sprint results are in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct F1SprintCard {
    pub season: u16,
    pub round: u16,
    pub race_name: String,
    pub country: String,
    /// The top 10, in order (points go to the top 8).
    pub results: Vec<F1ResultRow>,
}

/// When a server's podium predictions close (#403): at lights out (the default), or when qualifying starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum F1PicksClose {
    #[default]
    LightsOut,
    Qualifying,
}

impl F1PicksClose {
    /// The key stored (`lights_out`, `qualifying`).
    pub fn key(self) -> &'static str {
        match self {
            Self::LightsOut => "lights_out",
            Self::Qualifying => "qualifying",
        }
    }

    /// A stored key; anything else is the default.
    pub fn parse(key: &str) -> Self {
        if key == "qualifying" {
            Self::Qualifying
        } else {
            Self::LightsOut
        }
    }
}

/// One person's score for a race.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct F1Score {
    pub user_id: UserId,
    pub picks: Vec<String>,
    pub points: u32,
}

/// One person's season total of prediction points in this server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct F1SeasonRow {
    pub user_id: UserId,
    pub points: u32,
    /// Races they predicted and were scored on.
    pub races: u32,
}

/// The predictions' scores for a race, posted after its results card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct F1ScoresCard {
    pub season: u16,
    pub round: u16,
    pub race_name: String,
    /// The real podium, P1 to P3.
    pub podium: Vec<F1Driver>,
    /// The weekend card's field, to name everyone's picks.
    #[serde(default)]
    pub drivers: Vec<F1Driver>,
    /// Highest first.
    pub scores: Vec<F1Score>,
    /// The season so far in this server, highest first.
    pub season_table: Vec<F1SeasonRow>,
    #[serde(default)]
    pub weekend_message_id: Option<MessageId>,
}

/// An F1 card (#386): a system line in the server's F1 channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum F1Card {
    Weekend(F1WeekendCard),
    LightsOut(F1LightsOut),
    Results(F1ResultsCard),
    Scores(F1ScoresCard),
    /// A sprint's results (#403).
    Sprint(F1SprintCard),
}

/// `GET /servers/{id}/f1`: the server's F1 channel (`None` = no F1 cards), the next race weekend for the countdown
/// banner (`None` between seasons, or when Jolpica can't be reached and nothing is cached), and the season's
/// prediction table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct F1Status {
    pub channel_id: Option<ChannelId>,
    pub next: Option<F1Weekend>,
    pub season_table: Vec<F1SeasonRow>,
    /// When the server's podium predictions close (#403). Older servers never send it (lights out).
    #[serde(default)]
    pub picks_close: F1PicksClose,
}

/// `PUT /servers/{id}/f1-channel` (the server's owner only): a text channel of that server, or `None`, and when
/// predictions close (#403; left out, it stays as it is).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SetF1ChannelRequest {
    pub channel_id: Option<ChannelId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub picks_close: Option<F1PicksClose>,
}

/// `PUT /messages/{id}/f1/prediction`: your P1, P2, P3 (three different drivers from the card), replacing any before;
/// `None` takes it back. Closed at lights out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct F1PredictionRequest {
    pub picks: Option<Vec<String>>,
}

// ---------- Deadlock (#390) ----------

/// Deadlock's Steam app id ("Citadel").
pub const DEADLOCK_APP_ID: u32 = 1_422_450;

/// A Deadlock rank: `badge` is `tier * 10 + subrank` (0 = unranked, Obscurus); the image is the server's copy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockRank {
    pub badge: u32,
    pub tier: u32,
    pub subrank: u32,
    /// The tier's name ("Oracle").
    pub name: String,
    pub image: Option<EmbedThumb>,
}

/// One linked person's line on a post-match card.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockMatchPlayer {
    pub user_id: UserId,
    pub hero_id: u32,
    pub hero: String,
    /// The hero's card art (the server's copy).
    pub hero_art: Option<EmbedThumb>,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub won: bool,
    /// The ranked progress the match gave them (ranked matches only).
    pub rank_delta: Option<i32>,
    /// Their rank after the match (ranked matches only).
    pub rank: Option<DeadlockRank>,
}

/// A Deadlock match the server's linked people played: one card per match, everyone in it on it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockMatchCard {
    #[ts(type = "number")]
    pub match_id: u64,
    pub duration_s: u32,
    pub players: Vec<DeadlockMatchPlayer>,
}

/// One person's rank movement over the week.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockMover {
    pub user_id: UserId,
    pub from: DeadlockRank,
    pub to: DeadlockRank,
    /// Subranks moved: positive up, negative down.
    pub steps: i32,
}

/// The weekly rank movers: who moved, most up first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockMoversCard {
    pub movers: Vec<DeadlockMover>,
}

/// A hero named in a patch, with its icon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockHeroIcon {
    pub name: String,
    pub icon: Option<EmbedThumb>,
}

/// A Deadlock update: its title, a plain-text start of the notes, the link, and the heroes it names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockPatchCard {
    pub title: String,
    /// An `https` link to the notes.
    pub link: String,
    pub published_at: String,
    pub summary: Option<String>,
    #[serde(default)]
    pub heroes: Vec<DeadlockHeroIcon>,
}

/// A Deadlock card (#390) in a server's Deadlock channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeadlockCard {
    Match(DeadlockMatchCard),
    RankMovers(DeadlockMoversCard),
    Patch(DeadlockPatchCard),
}

/// `GET /servers/{id}/deadlock-channel`: where the server's Deadlock cards go (`None` = nowhere) and which kinds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockChannel {
    pub channel_id: Option<ChannelId>,
    pub match_cards: bool,
    pub rank_movers: bool,
    pub patch_notes: bool,
}

/// `PUT /servers/{id}/deadlock-channel` (the server's owner only): a text channel of that server or `None`, and the
/// switches; one left out stays as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SetDeadlockChannelRequest {
    pub channel_id: Option<ChannelId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub match_cards: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rank_movers: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub patch_notes: Option<bool>,
}

/// `GET /servers/{id}/deadlock-ranks`: the members' ranks, for the member list (those who share theirs and have
/// one). Empty while deadlock-api is unreachable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockRanks {
    pub entries: Vec<DeadlockRankEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockRankEntry {
    pub user_id: UserId,
    pub rank: DeadlockRank,
}

/// Someone's last Deadlock match, for their profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockLastMatch {
    pub hero: String,
    pub hero_icon: Option<EmbedThumb>,
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub won: bool,
    pub started_at: String,
}

/// A hero someone plays most.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockHeroLine {
    pub hero: String,
    pub icon: Option<EmbedThumb>,
    pub matches: u32,
    pub wins: u32,
}

/// Matches two people played on the same team.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockDuo {
    pub user_id: UserId,
    pub matches: u32,
    pub wins: u32,
}

/// `GET /servers/{id}/members/{user}/deadlock`: the Deadlock section of a profile card. Each part only while its
/// owner shares it; everything empty when there's nothing (not linked, deadlock-api unreachable).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct DeadlockProfile {
    pub rank: Option<DeadlockRank>,
    pub last_match: Option<DeadlockLastMatch>,
    /// Their most-played heroes, most first (at most 3).
    pub heroes: Vec<DeadlockHeroLine>,
    /// Their games with you (when you share squad stats too).
    pub with_you: Option<DeadlockDuo>,
    /// Their best teammate among this server's members: the best win rate over at least a few games together.
    pub best_duo: Option<DeadlockDuo>,
}

// ---------- Sports (#385) ----------

/// A league people can follow teams in. Football from api-football, rugby from api-rugby (API-Sports).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SportsLeague {
    /// South Africa's Premiership (PSL), football.
    Psl,
    /// The United Rugby Championship.
    Urc,
    /// England's Premier League, football.
    Epl,
}

impl SportsLeague {
    pub const ALL: [SportsLeague; 3] = [SportsLeague::Psl, SportsLeague::Urc, SportsLeague::Epl];

    /// The key stored and sent (`psl`, `urc`, `epl`).
    pub fn key(self) -> &'static str {
        match self {
            SportsLeague::Psl => "psl",
            SportsLeague::Urc => "urc",
            SportsLeague::Epl => "epl",
        }
    }

    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.key() == key)
    }

    /// Rugby (api-rugby) rather than football (api-football).
    pub fn rugby(self) -> bool {
        matches!(self, SportsLeague::Urc)
    }
}

/// A team in a league.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SportsTeam {
    pub league: SportsLeague,
    pub team_id: u32,
    pub name: String,
}

/// `GET /me/sports`: what this server can do with sports and your own follows and switch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SportsStatus {
    /// The server has an API-Sports key: false = sports aren't set up on this server (nothing is fetched or posted).
    pub available: bool,
    /// "Sports score pings" (default on): off, your follows bring no cards to your servers' Sports channels.
    pub pings: bool,
    pub follows: Vec<SportsTeam>,
    /// The leagues teams can be followed in.
    pub leagues: Vec<SportsLeague>,
    /// A quiet line when scores are paused (the day's requests are spent, the provider isn't answering).
    pub note: Option<String>,
}

/// `PUT /me/sports`: your switch; left out stays as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SportsSettingsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pings: Option<bool>,
}

/// `POST /me/sports/follows`: follow one of the league's teams (from `GET /sports/teams`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FollowTeamRequest {
    pub league: SportsLeague,
    pub team_id: u32,
}

/// `GET /sports/teams?league=…`: the league's teams this season (cached on the server for days), by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SportsTeams {
    pub teams: Vec<SportsTeam>,
}

/// `GET /servers/{id}/sports-channel`: where the server's sports cards go; `None` = nowhere (the default).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SportsChannel {
    pub channel_id: Option<ChannelId>,
    /// The leagues whose cards post here (#406), by the owner's choice; `None` = every league (the default). Older
    /// servers never send it.
    #[serde(default)]
    pub leagues: Option<Vec<SportsLeague>>,
}

/// `PUT /servers/{id}/sports-channel` (the server's owner only): a text channel of that server, or `None`, and the
/// leagues that post (#406; left out, they stay as they are; every league = no limit). Leagues need a channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SetSportsChannelRequest {
    pub channel_id: Option<ChannelId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub leagues: Option<Vec<SportsLeague>>,
}

/// Where a match is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SportsPhase {
    /// Match day, not kicked off yet.
    Upcoming,
    Live,
    /// Full time (after extra time or penalties too).
    Final,
    /// Postponed, cancelled or abandoned.
    Off,
}

/// One side of a match.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SportsSide {
    pub team_id: u32,
    pub name: String,
    /// `None` until kick-off.
    pub score: Option<u32>,
    /// The team's logo (#406), the server's copy of API-Sports' (clients never reach API-Sports); none when it
    /// couldn't be had. Older servers never send it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub logo: Option<EmbedThumb>,
}

/// A goal (football only: api-rugby names no try scorers).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SportsScorer {
    /// For the home side.
    pub home: bool,
    pub name: String,
    pub minute: Option<u32>,
    /// `pen` or `og`.
    pub note: Option<String>,
}

/// A sports card (#385): a followed team's match in a server's Sports channel, one card per match, edited in place
/// from the match-day banner through the live score to full time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SportsCard {
    pub league: SportsLeague,
    pub fixture_id: u32,
    pub phase: SportsPhase,
    /// Kick-off, RFC 3339 (UTC): apps show it in local time.
    pub kickoff: String,
    /// The provider's short status (`NS`, `1H`, `HT`, `FT`, …).
    pub status: String,
    /// The match minute while live (football).
    pub minute: Option<u32>,
    pub home: SportsSide,
    pub away: SportsSide,
    #[serde(default)]
    pub scorers: Vec<SportsScorer>,
    /// The server's members who follow either team (with pings on).
    #[serde(default)]
    pub followers: Vec<UserId>,
    /// The match's window passed with no live data to end it (#406): the card stopped updating, and says so rather
    /// than look stuck on match day or mid-match.
    #[serde(default)]
    pub no_updates: bool,
}
