use pulse_protocol::ids::MessageId;

/// IDs double as the time-order cursor for history paging, so they must be strictly
/// increasing even when many are minted in the same millisecond.
#[test]
fn ids_are_strictly_increasing_within_a_millisecond() {
    let ids: Vec<String> = (0..10_000).map(|_| MessageId::new().to_string()).collect();
    for w in ids.windows(2) {
        assert!(w[0] < w[1], "{} !< {}", w[0], w[1]);
    }
}
