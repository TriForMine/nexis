use protocol::Message;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("decode failed: {0}")]
    Decode(String),
}

pub trait Codec: Send + Sync {
    fn name(&self) -> &'static str;
    fn encode(&self, message: &Message) -> Vec<u8>;
    fn decode(&self, bytes: &[u8]) -> Result<Message, CodecError>;
}
