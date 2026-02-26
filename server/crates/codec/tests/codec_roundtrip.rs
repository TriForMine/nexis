use codec::Codec;
use codec_json::JsonCodec;
use codec_msgpack::MsgpackCodec;
use protocol::{Message, PROTOCOL_VERSION};
use serde_json::json;

fn message_fixtures() -> Vec<Message> {
    let with_checksum =
        include_str!("../../../../docs/fixtures/protocol/state_patch_v1_with_checksum.json");
    let without_checksum =
        include_str!("../../../../docs/fixtures/protocol/state_patch_v1_without_checksum.json");
    let snapshot = include_str!("../../../../docs/fixtures/protocol/state_snapshot_v1.json");

    vec![
        serde_json::from_str(with_checksum).expect("fixture should decode"),
        serde_json::from_str(without_checksum).expect("fixture should decode"),
        serde_json::from_str(snapshot).expect("fixture should decode"),
    ]
}

fn message_fixture() -> Message {
    Message {
        v: PROTOCOL_VERSION,
        t: "state.patch".to_owned(),
        rid: Some("rid-123".to_owned()),
        room: Some("counter-room".to_owned()),
        p: Some(json!({"counter": 2})),
    }
}

#[test]
fn fixture_variants_roundtrip_across_codecs() {
    let json = JsonCodec;
    let msgpack = MsgpackCodec;

    for fixture in message_fixtures() {
        let json_decoded = json
            .decode(&json.encode(&fixture))
            .expect("json roundtrip should succeed");
        let msgpack_decoded = msgpack
            .decode(&msgpack.encode(&fixture))
            .expect("msgpack roundtrip should succeed");

        assert_eq!(json_decoded, fixture);
        assert_eq!(msgpack_decoded, fixture);
    }
}

#[test]
fn json_roundtrip_success() {
    let codec = JsonCodec;
    let source = message_fixture();

    let encoded = codec.encode(&source);
    let decoded = codec.decode(&encoded).expect("decode should succeed");

    assert_eq!(decoded, source);
}

#[test]
fn messagepack_roundtrip_success() {
    let codec = MsgpackCodec;
    let source = message_fixture();

    let encoded = codec.encode(&source);
    let decoded = codec.decode(&encoded).expect("decode should succeed");

    assert_eq!(decoded, source);
}

#[test]
fn invalid_bytes_rejected() {
    let json = JsonCodec;
    let msgpack = MsgpackCodec;
    let garbage = [0u8, 159, 255, 0, 10];

    assert!(json.decode(&garbage).is_err());
    assert!(msgpack.decode(&garbage).is_err());
}

#[test]
fn decoding_corrupted_data_fails_safely() {
    let codec = MsgpackCodec;
    let source = message_fixture();
    let mut encoded = codec.encode(&source);
    encoded.truncate(encoded.len() / 2);

    let result = codec.decode(&encoded);
    assert!(result.is_err());
}
