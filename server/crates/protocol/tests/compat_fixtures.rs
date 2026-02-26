use protocol::{Handshake, Message, DEFAULT_MAX_PAYLOAD_BYTES};

#[test]
fn handshake_fixture_decodes() {
    let raw = include_str!("../../../../docs/fixtures/protocol/handshake_v1.json");
    let handshake: Handshake = serde_json::from_str(raw).expect("fixture should decode");

    assert_eq!(handshake.v, 1);
    assert_eq!(handshake.project_id, "demo-project");
    assert_eq!(
        handshake.codecs,
        vec!["msgpack".to_owned(), "json".to_owned()]
    );
    assert_eq!(handshake.session_id.as_deref(), Some("s-0123456789abcdef"));
}

#[test]
fn state_patch_with_checksum_fixture_validates() {
    let raw = include_str!("../../../../docs/fixtures/protocol/state_patch_v1_with_checksum.json");
    let message: Message = serde_json::from_str(raw).expect("fixture should decode");
    message
        .validate(DEFAULT_MAX_PAYLOAD_BYTES)
        .expect("fixture should validate");
}

#[test]
fn state_patch_without_checksum_fixture_validates() {
    let raw =
        include_str!("../../../../docs/fixtures/protocol/state_patch_v1_without_checksum.json");
    let message: Message = serde_json::from_str(raw).expect("fixture should decode");
    message
        .validate(DEFAULT_MAX_PAYLOAD_BYTES)
        .expect("fixture should validate");
}
