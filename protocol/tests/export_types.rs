use std::path::Path;

#[test]
fn export_bindings() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../client/ui/src/lib/protocol");
    pulse_protocol::export_all(&dir).unwrap();
    assert!(dir.join("Message.ts").exists());
    assert!(dir.join("ServerFrame.ts").exists());
    assert!(dir.join("ClientFrame.ts").exists());
}

#[test]
fn event_wire_format_is_stable() {
    let e = pulse_protocol::gateway::Event::Typing {
        channel_id: "01J0000000000000000000000A".parse().unwrap(),
        user_id: "01J0000000000000000000000B".parse().unwrap(),
    };
    assert_eq!(
        serde_json::to_string(&e).unwrap(),
        r#"{"t":"Typing","d":{"channel_id":"01J0000000000000000000000A","user_id":"01J0000000000000000000000B"}}"#
    );
}

#[test]
fn client_hello_parses() {
    let f: pulse_protocol::gateway::ClientFrame =
        serde_json::from_str(r#"{"op":"Hello","d":{"token":"abc"}}"#).unwrap();
    assert!(
        matches!(f, pulse_protocol::gateway::ClientFrame::Hello { token, .. } if token == "abc")
    );
}

#[test]
fn owner_only_events_name_their_owner() {
    use pulse_protocol::gateway::Event;
    use pulse_protocol::ids::{ChannelId, UserId};
    let u = UserId::new();
    let c = ChannelId::new();
    let e = Event::ReadStateUpdated {
        user_id: u,
        channel_id: c,
        last_read_message_id: None,
        mentions: None,
        unread: None,
    };
    assert_eq!(e.only_for(), Some(u));
    assert_eq!(e.channel_id(), Some(c));
    let p = Event::PresenceChanged {
        user_id: u,
        online: true,
        last_seen_at: None,
        status: Default::default(),
    };
    assert_eq!(p.only_for(), None);
    assert_eq!(p.channel_id(), None);
    let v = serde_json::to_value(&Event::ChannelRemoved {
        channel_id: c,
        user_id: u,
    })
    .unwrap();
    assert_eq!(v["t"], "ChannelRemoved");
}

#[test]
fn message_mentions_default_to_empty() {
    let m: pulse_protocol::rest::Message = serde_json::from_value(serde_json::json!({
        "id": "01J00000000000000000000000", "channel_id": "01J00000000000000000000001",
        "author_id": null, "kind": "normal", "content": "hi", "reply_to_id": null,
        "created_at": "2026-10-08T00:00:00Z", "edited_at": null, "deleted": false
    }))
    .unwrap();
    assert!(m.mentions.is_empty());
}

#[test]
fn old_presence_payloads_default_their_status() {
    use pulse_protocol::gateway::Event;
    use pulse_protocol::rest::{Person, PresenceStatus};
    let p: Person = serde_json::from_value(serde_json::json!({
        "user": { "id": "01J00000000000000000000000", "username": "sam", "avatar_hash": null },
        "online": true, "last_seen_at": null
    }))
    .unwrap();
    assert_eq!(p.status, PresenceStatus::Offline);
    let e: Event = serde_json::from_value(serde_json::json!({
        "t": "PresenceChanged",
        "d": { "user_id": "01J00000000000000000000000", "online": false, "last_seen_at": null }
    }))
    .unwrap();
    assert!(matches!(
        e,
        Event::PresenceChanged {
            status: PresenceStatus::Offline,
            ..
        }
    ));
}

#[test]
fn status_events_are_owner_only() {
    use pulse_protocol::gateway::Event;
    use pulse_protocol::ids::UserId;
    use pulse_protocol::rest::OwnStatus;
    let u = UserId::new();
    for e in [
        Event::StatusChanged {
            user_id: u,
            status: OwnStatus::default(),
        },
        Event::AppearOfflineChanged {
            user_id: u,
            target_id: UserId::new(),
            hidden: true,
        },
        Event::LiveNotifyChanged {
            user_id: u,
            target_id: UserId::new(),
            on: true,
        },
    ] {
        assert_eq!(e.only_for(), Some(u));
        assert_eq!(e.channel_id(), None);
    }
}
