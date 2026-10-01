//! WebSocket gateway frames.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{ChannelId, MessageId, ServerId, UserId};
use crate::rest::{Channel, Member, Message, Server, User};

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "op", content = "d")]
pub enum ClientFrame {
    Hello {
        token: String,
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
            | Self::VoiceStateChanged { channel_id, .. } => Some(*channel_id),
            Self::ChannelCreated { channel } => Some(channel.id),
            Self::ServerCreated { .. } | Self::MemberJoined { .. } => None,
        }
    }
}
