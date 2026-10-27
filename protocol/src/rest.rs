//! REST request/response bodies.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{ChannelId, MessageId, ServerId, UserId};

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
    pub username: String,
    pub avatar_hash: Option<String>,
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
}

impl ChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
            Self::Dm => "dm",
            Self::Group => "group",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "text" => Self::Text,
            "voice" => Self::Voice,
            "dm" => Self::Dm,
            "group" => Self::Group,
            _ => return None,
        })
    }

    /// DMs and groups are private: only their members may see anything about them.
    pub fn is_private(self) -> bool {
        matches!(self, Self::Dm | Self::Group)
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
                        avatar_hash: None,
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
}
