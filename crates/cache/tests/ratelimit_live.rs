//! The request counter against a real Redis server.
//!
//! Ignored by default. Run with REDIS_URL set and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::time::Duration;

use builtpanel_cache::{RedisHandle, ratelimit::RateLimiter};
use secrecy::SecretString;

fn limiter(limit: u32, window_ms: u64) -> RateLimiter {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set for --include-ignored runs");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    RateLimiter::new(
        redis,
        format!("test-{}", std::process::id()),
        limit,
        Duration::from_millis(window_ms),
    )
}

#[tokio::test]
#[ignore = "needs Redis"]
async fn calls_up_to_the_limit_pass_then_stop_until_the_window_is_over() {
    let l = limiter(3, 1500);
    let who = format!("a-{}", std::process::id());
    for i in 1..=3 {
        assert!(l.hit(&who).await.unwrap(), "call {i} is within the limit");
    }
    assert!(!l.hit(&who).await.unwrap(), "the fourth is not");
    assert!(!l.hit(&who).await.unwrap());
    // Others are not affected.
    assert!(l.hit("someone-else").await.unwrap());
    // When the window is over the subject starts again.
    tokio::time::sleep(Duration::from_millis(1700)).await;
    assert!(l.hit(&who).await.unwrap());
}

#[tokio::test]
#[ignore = "needs Redis"]
async fn concurrent_calls_cannot_slip_past_the_limit() {
    let l = limiter(10, 5000);
    let who = format!("race-{}", std::process::id());
    let mut tasks = Vec::new();
    for _ in 0..50 {
        let (l, who) = (l.clone(), who.clone());
        tasks.push(tokio::spawn(async move { l.hit(&who).await.unwrap() }));
    }
    let mut allowed = 0;
    for t in tasks {
        if t.await.unwrap() {
            allowed += 1;
        }
    }
    assert_eq!(allowed, 10);
}
