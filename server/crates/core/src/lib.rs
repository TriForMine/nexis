use auth::verify_token;
use chrono::{DateTime, Utc};
use codec::Codec;
use protocol::Handshake;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub default_codec: String,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            default_codec: "msgpack".to_owned(),
        }
    }
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("codec not available")]
    CodecUnavailable,
    #[error("auth failed")]
    AuthFailed,
}

pub struct NexisCore {
    pub config: NetworkConfig,
}

impl NexisCore {
    pub fn new(config: NetworkConfig) -> Self {
        Self { config }
    }

    pub fn negotiate_codec<'a>(
        &self,
        offered: &[String],
        codecs: &'a [Box<dyn Codec>],
    ) -> Result<&'a dyn Codec, CoreError> {
        let mut candidates: Vec<&str> = Vec::new();
        if offered.iter().any(|codec| codec == "msgpack") {
            candidates.push("msgpack");
        }
        if offered
            .iter()
            .any(|codec| codec == &self.config.default_codec)
        {
            candidates.push(&self.config.default_codec);
        }
        if offered.iter().any(|codec| codec == "json") {
            candidates.push("json");
        }

        for offered_codec in offered {
            candidates.push(offered_codec);
        }

        for candidate in candidates {
            if let Some(codec) = codecs.iter().find(|codec| codec.name() == candidate) {
                return Ok(codec.as_ref());
            }
        }

        Err(CoreError::CodecUnavailable)
    }

    pub fn verify_handshake(
        &self,
        handshake: &Handshake,
        secret: &str,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        verify_token(&handshake.token, &handshake.project_id, secret, now)
            .map(|_| ())
            .map_err(|_| CoreError::AuthFailed)
    }
}
