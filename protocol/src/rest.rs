//! REST request/response bodies.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{
    AttachmentId, CallId, CategoryId, ChannelId, ClipUploadId, GameId, LineId, MessageId,
    QotyEventId, ServerId, UserId,
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
    pub invite_code: String,
    pub username: String,
    pub password: String,
}
redacting_debug!(RegisterRequest { username ; invite_code, password });

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
/// second) `#rrggbb` colours.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NameStyle {
    pub font: Option<String>,
    pub colors: Vec<String>,
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ProfileStats {
    #[ts(type = "number")]
    pub voice_seconds: u64,
    pub member_since: String,
    pub top_reaction: Option<String>,
    pub times_renamed: u32,
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
}

/// `PUT /servers/{id}/members/{user}/nickname` and `PUT /me/private-nicknames/{user}`: `None`
/// (or blank) resets it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SetNicknameRequest {
    #[serde(default)]
    pub nickname: Option<String>,
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
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct LayoutCategory {
    pub id: CategoryId,
    pub channels: Vec<ChannelId>,
}

/// One other user → a 1:1 `dm` (reused if it exists); 2..=9 others → a new `group`.
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
    /// the winner's slice ([`wheel_winner_at`]).
    pub angle: f64,
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

/// Your own status, for your own sessions only. `dnd_until`: when a timed DND ends (RFC 3339 UTC);
/// `None` with `Dnd` means until you turn it off.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct OwnStatus {
    pub status: Status,
    #[serde(default)]
    pub dnd_until: Option<String>,
}

/// `PUT /me/status`. `dnd_until` only goes with `Dnd`: a time in the next 7 days.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct SetStatusRequest {
    pub status: Status,
    #[serde(default)]
    pub dnd_until: Option<String>,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Leaderboard {
    pub winners: Vec<LeaderboardWin>,
    pub top_year: Vec<Message>,
    pub top_all_time: Vec<Message>,
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
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

/// `POST /messages/{id}/embeds/hide`: the sender hides one preview, or all of them.
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
