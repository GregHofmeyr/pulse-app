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
    assert!(matches!(f, pulse_protocol::gateway::ClientFrame::Hello { token } if token == "abc"));
}
