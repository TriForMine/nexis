use codec::{Codec, CodecError};
use protocol::Message;

#[derive(Debug, Default)]
pub struct JsonCodec;

impl Codec for JsonCodec {
    fn name(&self) -> &'static str {
        "json"
    }

    fn encode(&self, message: &Message) -> Vec<u8> {
        serde_json::to_vec(message).expect("message serialization should always succeed")
    }

    fn decode(&self, bytes: &[u8]) -> Result<Message, CodecError> {
        serde_json::from_slice(bytes).map_err(|err| CodecError::Decode(err.to_string()))
    }
}
