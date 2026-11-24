//! Failure limiter against a real Redis server.
//!
//! Ignored by default. Run with REDIS_URL set and `--include-ignored`.
//! Subjects are unique per test run, so tests do not interfere.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::time::Duration;

use builtpanel_cache::{
    RedisHandle,
    lockout::{FailureLimiter, LockoutPolicy},
};
use secrecy::SecretString;

fn limiter(threshold: u32, window_ms: u64, lockout_ms: u64) -> FailureLimiter {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set for --include-ignored runs");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    FailureLimiter::new(
        redis,
        "test",
        LockoutPolicy {
            threshold,
            window: Duration::from_millis(window_ms),
            lockout: Duration::from_millis(lockout_ms),
        },
    )
}

fn subject(name: &str) -> String {
    format!("{}-{}-{name}", std::process::id(), uuid_like())
}

fn uuid_like() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_URL and pass --include-ignored"]
async fn threshold_failure_locks_and_lock_expires() {
    let l = limiter(3, 60_000, 1_500);
    let s = subject("expire");
    assert_eq!(l.locked_for(&s).await.unwrap(), None);
    for n in 1..=2 {
        let r = l.record_failure(&s).await.unwrap();
        assert_eq!((r.failures, r.locked), (n, false));
        assert_eq!(l.locked_for(&s).await.unwrap(), None);
    }
    let r = l.record_failure(&s).await.unwrap();
    assert_eq!((r.failures, r.locked), (3, true));
    let left = l.locked_for(&s).await.unwrap().expect("locked");
    assert!(left <= Duration::from_millis(1_500) && left > Duration::ZERO);

    tokio::time::sleep(Duration::from_millis(1_700)).await;
    assert_eq!(l.locked_for(&s).await.unwrap(), None);
    // The window was cleared when the lock started: a fresh count begins.
    assert_eq!(l.record_failure(&s).await.unwrap().failures, 1);
    l.unlock(&s).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_URL and pass --include-ignored"]
async fn old_failures_slide_out_of_the_window() {
    let l = limiter(3, 1_000, 60_000);
    let s = subject("slide");
    l.record_failure(&s).await.unwrap();
    l.record_failure(&s).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    let r = l.record_failure(&s).await.unwrap();
    assert_eq!((r.failures, r.locked), (1, false));
    l.unlock(&s).await.unwrap();
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_URL and pass --include-ignored"]
async fn reset_forgets_failures_and_unlock_lifts_lock() {
    let l = limiter(2, 60_000, 60_000);
    let s = subject("reset");
    l.record_failure(&s).await.unwrap();
    l.reset(&s).await.unwrap();
    assert_eq!(l.record_failure(&s).await.unwrap().failures, 1);
    assert!(l.record_failure(&s).await.unwrap().locked);
    assert!(l.locked_for(&s).await.unwrap().is_some());
    l.unlock(&s).await.unwrap();
    assert_eq!(l.locked_for(&s).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_URL and pass --include-ignored"]
async fn concurrent_failures_never_overshoot_the_threshold() {
    let l = limiter(5, 60_000, 60_000);
    let s = subject("race");
    let tasks: Vec<_> = (0..20)
        .map(|_| {
            let l = l.clone();
            let s = s.clone();
            tokio::spawn(async move { l.record_failure(&s).await.unwrap() })
        })
        .collect();
    let mut locks = 0;
    for t in tasks {
        if t.await.unwrap().locked {
            locks += 1;
        }
    }
    // 20 failures with threshold 5 and the window cleared on each lock: exactly 4 lock events,
    // and every failure was counted (no member collisions).
    assert_eq!(locks, 4);
    assert!(l.locked_for(&s).await.unwrap().is_some());
    l.unlock(&s).await.unwrap();
}
