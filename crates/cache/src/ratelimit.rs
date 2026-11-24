//! A fixed-window request counter, backed by Redis (T05.15).
//!
//! For endpoints that anybody can reach and that cost a query each: allow at most
//! `limit` calls per `window` for a subject (a client address), and say no after
//! that until the window is over. One Redis `INCR` (with the window's expiry set
//! by the first call) per request; concurrent requests and several API instances
//! all count in the same place.
//!
//! Subjects are part of key names: pass addresses and other values the server
//! chose, never free text from the request.

use std::{sync::Arc, time::Duration};

use crate::{CacheError, RedisHandle};

/// KEYS[1] = counter. ARGV[1] = window ms. Returns the count including this call.
const HIT: &str = r"
local n = redis.call('INCR', KEYS[1])
if n == 1 then
  redis.call('PEXPIRE', KEYS[1], ARGV[1])
end
return n
";

/// How many calls a subject may make in a window. Clone freely.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    redis: RedisHandle,
    namespace: Arc<str>,
    limit: u32,
    window: Duration,
}

impl RateLimiter {
    /// `namespace` separates independent limiters, e.g. `tls-ask`.
    #[must_use]
    pub fn new(
        redis: RedisHandle,
        namespace: impl Into<Arc<str>>,
        limit: u32,
        window: Duration,
    ) -> Self {
        Self {
            redis,
            namespace: namespace.into(),
            limit: limit.max(1),
            window,
        }
    }

    /// Counts one call for `subject`. `true`: allowed. `false`: over the limit
    /// for this window.
    ///
    /// # Errors
    /// Connection or command failures.
    pub async fn hit(&self, subject: &str) -> Result<bool, CacheError> {
        let key = format!("builtpanel:ratelimit:{}:{subject}", self.namespace);
        let window_ms = u64::try_from(self.window.as_millis())
            .unwrap_or(u64::MAX)
            .max(1);
        let mut conn = self.redis.connection().await?;
        let count: u64 = redis::cmd("EVAL")
            .arg(HIT)
            .arg(1)
            .arg(&key)
            .arg(window_ms)
            .query_async(&mut conn)
            .await?;
        Ok(count <= u64::from(self.limit))
    }
}
