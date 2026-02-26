use std::collections::HashMap;
use std::time::{Duration, Instant};

use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum RateLimitError {
    #[error("rate limit exceeded")]
    Exceeded,
}

#[derive(Debug)]
pub struct RateLimiter {
    per_window: u32,
    window: Duration,
    clients: HashMap<String, (u32, Instant)>,
}

impl RateLimiter {
    pub fn new(per_window: u32, window: Duration) -> Self {
        Self {
            per_window,
            window,
            clients: HashMap::new(),
        }
    }

    pub fn check(&mut self, client_id: &str, now: Instant) -> Result<(), RateLimitError> {
        let entry = self.clients.entry(client_id.to_owned()).or_insert((0, now));
        if now.duration_since(entry.1) >= self.window {
            *entry = (0, now);
        }

        if entry.0 >= self.per_window {
            return Err(RateLimitError::Exceeded);
        }

        entry.0 += 1;
        Ok(())
    }
}
