//! REST request/response bodies.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{
    ChannelId, ClipUploadId, GameId, LineId, MessageId, QotyEventId, ServerId, UserId,
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

    /// Kinds that hold messages and belong to a server: its members may read and post.
    pub fn is_server_chat(self) -> bool {
        matches!(self, Self::Text | Self::Quotes | Self::Clips)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Channel {
    pub id: ChannelId,
    pub server_id: Option<ServerId>,
    pub kind: ChannelKind,
    pub name: Option<String>,
    #[ts(type = "number")]
    pub position: i64,
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

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateServerRequest {
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct CreateChannelRequest {
    pub kind: ChannelKind,
    pub name: String,
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
}

/// One emoji on a message: a Unicode emoji or `custom:<id>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ReactionSummary {
    pub emoji: String,
    pub count: u32,
    pub me: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Person {
    pub user: User,
    pub online: bool,
    pub last_seen_at: Option<String>,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AddMembersRequest {
    pub user_ids: Vec<UserId>,
}

/// `None` or blank → back to the default name (the members' names).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct RenameChannelRequest {
    pub name: Option<String>,
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
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct EditMessageRequest {
    pub content: String,
}

/// A LiveKit join token (voice, or a stream ticket) and where to use it.
#[derive(Clone, Serialize, Deserialize, TS)]
pub struct VoiceTokenResponse {
    pub url: String,
    pub token: String,
}
redacting_debug!(VoiceTokenResponse { url ; token });

/// Screen-share quality. `height` is 720, 1080 or 1440; `fps` is 15, 30 or 60.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StreamSettings {
    pub height: u16,
    pub fps: u8,
    pub audio: bool,
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
    pub viewers: Vec<UserId>,
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct GoLiveRequest {
    pub settings: StreamSettings,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct WatchRequest {
    pub streamer: UserId,
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
}

impl ReportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bug => "bug",
            Self::Crash => "crash",
            Self::UnexpectedExit => "unexpected-exit",
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
}

impl SearchHas {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clip => "clip",
            Self::Quote => "quote",
            Self::Link => "link",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "clip" => Self::Clip,
            "quote" => Self::Quote,
            "link" => Self::Link,
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
}
