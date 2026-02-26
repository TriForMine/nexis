use codec::{Codec, CodecError};
use protocol::Message;

#[derive(Debug, Default)]
pub struct MsgpackCodec;

impl Codec for MsgpackCodec {
    fn name(&self) -> &'static str {
        "msgpack"
    }

    fn encode(&self, message: &Message) -> Vec<u8> {
        rmp_serde::to_vec_named(message).expect("message serialization should always succeed")
    }

    fn decode(&self, bytes: &[u8]) -> Result<Message, CodecError> {
        rmp_serde::from_slice(bytes).map_err(|err| CodecError::Decode(err.to_string()))
    }
}
