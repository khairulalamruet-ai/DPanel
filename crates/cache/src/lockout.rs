//! Failure counting with lockout, backed by Redis.
//!
//! Each subject (an account, an IP network, …) has a sliding window of recent
//! failures (a sorted set scored by Redis server time) and a lock key. When
//! `threshold` failures fall inside `window`, the lock key is set for
//! `lockout` and the window is cleared, so the subject starts afresh once the
//! lock expires.
//!
//! All updates run as one Lua script, so concurrent requests (also across API
//! instances) cannot race past the threshold, and only the Redis clock is used.
//!
//! Subjects are embedded in key names and must not contain user-controlled
//! free text; hash such values first.

use std::{sync::Arc, time::Duration};

use crate::{CacheError, RedisHandle};

/// KEYS[1] = window zset, KEYS[2] = lock key.
/// ARGV[1] = window ms, ARGV[2] = threshold, ARGV[3] = lockout ms.
/// Returns {failures in window, 1 if this call set the lock else 0}.
const RECORD_FAILURE: &str = r"
local t = redis.call('TIME')
local now = tonumber(t[1]) * 1000000 + tonumber(t[2])
local window_us = tonumber(ARGV[1]) * 1000
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', now - window_us)
local n = redis.call('ZCARD', KEYS[1])
redis.call('ZADD', KEYS[1], now, now .. '-' .. n)
n = n + 1
if n >= tonumber(ARGV[2]) then
  redis.call('SET', KEYS[2], '1', 'PX', ARGV[3])
  redis.call('DEL', KEYS[1])
  return {n, 1}
end
redis.call('PEXPIRE', KEYS[1], ARGV[1])
return {n, 0}
";

/// Thresholds for one kind of subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockoutPolicy {
    /// Failures inside `window` that trigger a lockout (at least 1).
    pub threshold: u32,
    /// Sliding window over which failures are counted.
    pub window: Duration,
    /// How long the subject stays locked.
    pub lockout: Duration,
}

/// Result of [`FailureLimiter::record_failure`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureRecorded {
    /// Failures counted in the window, including this one.
    pub failures: u32,
    /// This failure reached the threshold and started a lockout.
    pub locked: bool,
}

/// Counts failures per subject and locks subjects out. Clone freely.
#[derive(Debug, Clone)]
pub struct FailureLimiter {
    redis: RedisHandle,
    namespace: Arc<str>,
    policy: LockoutPolicy,
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX).max(1)
}

impl FailureLimiter {
    /// `namespace` separates independent limiters, e.g. `login:account`.
    #[must_use]
    pub fn new(redis: RedisHandle, namespace: impl Into<Arc<str>>, policy: LockoutPolicy) -> Self {
        Self {
            redis,
            namespace: namespace.into(),
            policy,
        }
    }

    /// The configured policy.
    #[must_use]
    pub fn policy(&self) -> LockoutPolicy {
        self.policy
    }

    fn keys(&self, subject: &str) -> (String, String) {
        (
            format!("builtpanel:lockout:{}:fail:{subject}", self.namespace),
            format!("builtpanel:lockout:{}:lock:{subject}", self.namespace),
        )
    }

    /// Remaining lock time, or `None` when the subject is not locked.
    ///
    /// # Errors
    /// Connection or command failures.
    pub async fn locked_for(&self, subject: &str) -> Result<Option<Duration>, CacheError> {
        let (_, lock) = self.keys(subject);
        let mut conn = self.redis.connection().await?;
        let pttl: i64 = redis::cmd("PTTL").arg(&lock).query_async(&mut conn).await?;
        // -2: no key, -1: key without TTL (never written by us; treat as unlocked).
        Ok(u64::try_from(pttl)
            .ok()
            .filter(|ms| *ms > 0)
            .map(Duration::from_millis))
    }

    /// Records one failure and locks the subject when the threshold is reached.
    ///
    /// # Errors
    /// Connection or command failures.
    pub async fn record_failure(&self, subject: &str) -> Result<FailureRecorded, CacheError> {
        let (window, lock) = self.keys(subject);
        let mut conn = self.redis.connection().await?;
        let (failures, locked): (u32, u8) = redis::cmd("EVAL")
            .arg(RECORD_FAILURE)
            .arg(2)
            .arg(&window)
            .arg(&lock)
            .arg(millis(self.policy.window))
            .arg(self.policy.threshold.max(1))
            .arg(millis(self.policy.lockout))
            .query_async(&mut conn)
            .await?;
        Ok(FailureRecorded {
            failures,
            locked: locked == 1,
        })
    }

    /// Forgets recent failures (e.g. after a successful login). An active
    /// lock is left in place.
    ///
    /// # Errors
    /// Connection or command failures.
    pub async fn reset(&self, subject: &str) -> Result<(), CacheError> {
        let (window, _) = self.keys(subject);
        self.redis.del(&window).await.map(|_| ())
    }

    /// Lifts a lock and forgets failures (admin unlock).
    ///
    /// # Errors
    /// Connection or command failures.
    pub async fn unlock(&self, subject: &str) -> Result<(), CacheError> {
        let (window, lock) = self.keys(subject);
        let mut conn = self.redis.connection().await?;
        let _: u64 = redis::cmd("DEL")
            .arg(&window)
            .arg(&lock)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }
}
