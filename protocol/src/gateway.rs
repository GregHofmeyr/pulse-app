//! WebSocket gateway frames.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{ChannelId, MessageId, ServerId, UserId};
use crate::rest::{
    Channel, CustomEmoji, Member, Message, Mute, Person, ReadState, Server, StreamEndReason,
    StreamInfo, User,
};

/// `Debug` is written out by hand: `Hello` carries the session token.
#[derive(Clone, Serialize, Deserialize, TS)]
#[serde(tag = "op", content = "d")]
pub enum ClientFrame {
    Hello {
        token: String,
        /// [`crate::PROTOCOL_VERSION`] of the client; absent (0) from pre-versioning clients.
        #[serde(default)]
        client_version: u32,
    },
    Heartbeat,
    Typing {
        channel_id: ChannelId,
    },
    /// Your own mute/deafen state while in a voice room.
    VoiceState {
        flags: VoiceFlags,
    },
}

impl std::fmt::Debug for ClientFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hello { client_version, .. } => f
                .debug_struct("Hello")
                .field("token", &"<redacted>")
                .field("client_version", client_version)
                .finish(),
            Self::Heartbeat => f.write_str("Heartbeat"),
            Self::Typing { channel_id } => f
                .debug_struct("Typing")
                .field("channel_id", channel_id)
                .finish(),
            Self::VoiceState { flags } => {
                f.debug_struct("VoiceState").field("flags", flags).finish()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct VoiceFlags {
    pub muted: bool,
    pub deafened: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct VoiceMember {
    pub user_id: UserId,
    pub flags: VoiceFlags,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct VoiceRoom {
    pub channel_id: ChannelId,
    pub members: Vec<VoiceMember>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct Ready {
    pub me: User,
    pub servers: Vec<Server>,
    pub channels: Vec<Channel>,
    pub members: Vec<ServerMembers>,
    pub dm_members: Vec<DmMembers>,
    /// Who is in which voice room right now (only rooms you may see).
    pub voice: Vec<VoiceRoom>,
    /// Everyone with an account, with presence.
    pub people: Vec<Person>,
    /// Your private read points with unread/mention counts.
    pub read_states: Vec<ReadState>,
    pub mutes: Vec<Mute>,
    /// Conversations you closed (hidden from your list until a new message).
    pub hidden: Vec<ChannelId>,
    /// Latest message of each of your DMs/groups (list previews + sorting).
    pub latest: Vec<Message>,
    /// Streams live right now (only in channels you may see).
    #[serde(default)]
    pub streams: Vec<StreamInfo>,
    /// Every server's custom emoji, deleted ones (without a hash) included.
    #[serde(default)]
    pub custom_emoji: Vec<CustomEmoji>,
    /// Your most used emoji (standard as Unicode, custom as `custom:<id>`), best first.
    #[serde(default)]
    pub frequent_emoji: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ServerMembers {
    pub server_id: ServerId,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct DmMembers {
    pub channel_id: ChannelId,
    pub user_ids: Vec<UserId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "op", content = "d")]
pub enum ServerFrame {
    Ready(Ready),
    HeartbeatAck,
    Event(Event),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "t", content = "d")]
pub enum Event {
    MessageCreated {
        message: Message,
        nonce: Option<String>,
    },
    MessageUpdated {
        message: Message,
    },
    MessageDeleted {
        channel_id: ChannelId,
        message_id: MessageId,
        /// Who wrote it and whom it mentioned, so clients can take it back out of their
        /// unread and mention counts.
        author_id: Option<UserId>,
        mentions: Vec<UserId>,
    },
    ChannelCreated {
        channel: Channel,
    },
    ServerCreated {
        server: Server,
    },
    MemberJoined {
        server_id: ServerId,
        member: Member,
    },
    Typing {
        channel_id: ChannelId,
        user_id: UserId,
    },
    VoiceJoined {
        channel_id: ChannelId,
        user_id: UserId,
        /// The joiner's current mute/deafen (they may have declared it before the join landed).
        flags: VoiceFlags,
    },
    VoiceLeft {
        channel_id: ChannelId,
        user_id: UserId,
    },
    VoiceStateChanged {
        channel_id: ChannelId,
        user_id: UserId,
        flags: VoiceFlags,
    },
    UserCreated {
        user: User,
    },
    PresenceChanged {
        user_id: UserId,
        online: bool,
        last_seen_at: Option<String>,
    },
    GroupMembersChanged {
        channel_id: ChannelId,
        user_ids: Vec<UserId>,
    },
    ChannelUpdated {
        channel: Channel,
    },
    /// Only to the removed/leaving user.
    ChannelRemoved {
        channel_id: ChannelId,
        user_id: UserId,
    },
    /// Only to the owner's own sessions (multi-device sync; no read receipts).
    ReadStateUpdated {
        user_id: UserId,
        channel_id: ChannelId,
        last_read_message_id: Option<MessageId>,
    },
    MutesChanged {
        user_id: UserId,
        mutes: Vec<Mute>,
    },
    ConversationVisibility {
        user_id: UserId,
        channel_id: ChannelId,
        hidden: bool,
    },
    StreamStarted {
        stream: StreamInfo,
    },
    StreamUpdated {
        stream: StreamInfo,
    },
    StreamEnded {
        channel_id: ChannelId,
        user_id: UserId,
        reason: StreamEndReason,
    },
    ViewersChanged {
        channel_id: ChannelId,
        user_id: UserId,
        viewers: Vec<UserId>,
    },
    EmojiCreated {
        emoji: CustomEmoji,
    },
    EmojiUpdated {
        emoji: CustomEmoji,
    },
    /// Carries `hash: None`.
    EmojiDeleted {
        emoji: CustomEmoji,
    },
    /// `count` is the new absolute count, so repeated or crossed events can't drift.
    ReactionAdded {
        channel_id: ChannelId,
        message_id: MessageId,
        emoji: String,
        user_id: UserId,
        count: u32,
    },
    ReactionRemoved {
        channel_id: ChannelId,
        message_id: MessageId,
        emoji: String,
        user_id: UserId,
        count: u32,
    },
    /// Only to the user's own sessions, after a write that counted their emoji use: the new
    /// [`Ready::frequent_emoji`].
    FrequentEmoji {
        user_id: UserId,
        frequent_emoji: Vec<String>,
    },
}

impl Event {
    /// The channel this event is scoped to, if any. Drives privacy fan-out.
    pub fn channel_id(&self) -> Option<ChannelId> {
        match self {
            Self::MessageCreated { message, .. } | Self::MessageUpdated { message } => {
                Some(message.channel_id)
            }
            Self::MessageDeleted { channel_id, .. }
            | Self::Typing { channel_id, .. }
            | Self::VoiceJoined { channel_id, .. }
            | Self::VoiceLeft { channel_id, .. }
            | Self::VoiceStateChanged { channel_id, .. }
            | Self::ReactionAdded { channel_id, .. }
            | Self::ReactionRemoved { channel_id, .. } => Some(*channel_id),
            Self::ChannelCreated { channel } => Some(channel.id),
            Self::GroupMembersChanged { channel_id, .. }
            | Self::ChannelRemoved { channel_id, .. }
            | Self::ReadStateUpdated { channel_id, .. }
            | Self::ConversationVisibility { channel_id, .. } => Some(*channel_id),
            Self::ChannelUpdated { channel } => Some(channel.id),
            Self::StreamStarted { stream } | Self::StreamUpdated { stream } => {
                Some(stream.channel_id)
            }
            Self::StreamEnded { channel_id, .. } | Self::ViewersChanged { channel_id, .. } => {
                Some(*channel_id)
            }
            Self::ServerCreated { .. }
            | Self::MemberJoined { .. }
            | Self::UserCreated { .. }
            | Self::PresenceChanged { .. }
            | Self::MutesChanged { .. }
            | Self::EmojiCreated { .. }
            | Self::EmojiUpdated { .. }
            | Self::EmojiDeleted { .. }
            | Self::FrequentEmoji { .. } => None,
        }
    }

    /// Per-user events go to that user's own sessions only (checked before channel audience).
    pub fn only_for(&self) -> Option<UserId> {
        match self {
            Self::ChannelRemoved { user_id, .. }
            | Self::ReadStateUpdated { user_id, .. }
            | Self::MutesChanged { user_id, .. }
            | Self::FrequentEmoji { user_id, .. }
            | Self::ConversationVisibility { user_id, .. } => Some(*user_id),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rest::{StreamRole, StreamSettings, StreamStatus};

    /// Debug output can end up in logs: `Hello` never shows the session token.
    #[test]
    fn hello_debug_never_shows_the_token() {
        let hello = format!(
            "{:?}",
            ClientFrame::Hello {
                token: "SECRET-TOKEN".into(),
                client_version: 7,
            }
        );
        assert!(!hello.contains("SECRET"), "{hello}");
        assert!(hello.contains("Hello") && hello.contains("7"), "{hello}");
        let typing = format!(
            "{:?}",
            ClientFrame::Typing {
                channel_id: ChannelId::new()
            }
        );
        assert!(typing.contains("Typing"), "{typing}");
    }

    #[test]
    fn hello_without_a_version_is_version_zero() {
        let f: ClientFrame = serde_json::from_str(r#"{"op":"Hello","d":{"token":"t"}}"#).unwrap();
        assert!(matches!(
            f,
            ClientFrame::Hello {
                client_version: 0,
                ..
            }
        ));
    }

    #[test]
    fn hello_ignores_unknown_fields() {
        // a newer client talking to this server still gets in
        let f: ClientFrame = serde_json::from_str(
            r#"{"op":"Hello","d":{"token":"t","client_version":9,"shiny":true}}"#,
        )
        .unwrap();
        assert!(matches!(
            f,
            ClientFrame::Hello {
                client_version: 9,
                ..
            }
        ));
    }

    fn stream_info(channel: ChannelId, user: UserId) -> StreamInfo {
        StreamInfo {
            channel_id: channel,
            user_id: user,
            settings: StreamSettings {
                height: 720,
                fps: 60,
                audio: true,
            },
            status: StreamStatus::Live,
            viewers: vec![],
            started_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn ready_without_streams_parses() {
        let mut v = serde_json::to_value(Ready {
            me: User {
                id: UserId::new(),
                username: "a".into(),
                avatar_hash: None,
            },
            servers: vec![],
            channels: vec![],
            members: vec![],
            dm_members: vec![],
            voice: vec![],
            people: vec![],
            read_states: vec![],
            mutes: vec![],
            hidden: vec![],
            latest: vec![],
            streams: vec![],
            custom_emoji: vec![],
            frequent_emoji: vec![],
        })
        .unwrap();
        v.as_object_mut().unwrap().remove("streams");
        let ready: Ready = serde_json::from_value(v).unwrap();
        assert!(ready.streams.is_empty());
    }

    #[test]
    fn stream_events_are_channel_scoped() {
        let (c, u) = (ChannelId::new(), UserId::new());
        let events = [
            Event::StreamStarted {
                stream: stream_info(c, u),
            },
            Event::StreamUpdated {
                stream: stream_info(c, u),
            },
            Event::StreamEnded {
                channel_id: c,
                user_id: u,
                reason: StreamEndReason::Stopped,
            },
            Event::ViewersChanged {
                channel_id: c,
                user_id: u,
                viewers: vec![u],
            },
        ];
        for e in events {
            assert_eq!(e.channel_id(), Some(c), "{e:?}");
            assert_eq!(e.only_for(), None, "{e:?}");
        }
    }

    #[test]
    fn stream_role_is_a_letter() {
        assert_eq!(serde_json::to_string(&StreamRole::V).unwrap(), "\"v\"");
        assert_eq!(serde_json::to_string(&StreamRole::A).unwrap(), "\"a\"");
    }
}
