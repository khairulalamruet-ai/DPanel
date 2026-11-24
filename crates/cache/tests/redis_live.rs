//! Integration tests against a real Redis server.
//!
//! Ignored by default. Run with:
//! `REDIS_URL=redis://:<password>@127.0.0.1:6379/15 cargo test -p builtpanel-cache -- --include-ignored`
//! Use a dedicated logical database (e.g. /15); keys are prefixed and cleaned up.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::time::Duration;

use builtpanel_cache::RedisHandle;
use secrecy::SecretString;

fn handle() -> RedisHandle {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set for --include-ignored runs");
    RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL")
}

fn key(name: &str) -> String {
    format!("builtpanel:test:{}:{name}", std::process::id())
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_URL and pass --include-ignored"]
async fn ping_succeeds() {
    handle().ping().await.expect("PONG");
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_URL and pass --include-ignored"]
async fn set_get_del_round_trip() {
    let h = handle();
    let k = key("roundtrip");
    h.set_ex(&k, "value-1", Duration::from_secs(30))
        .await
        .expect("set");
    assert_eq!(h.get(&k).await.expect("get").as_deref(), Some("value-1"));
    assert!(h.del(&k).await.expect("del"));
    assert_eq!(h.get(&k).await.expect("get after del"), None);
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_URL and pass --include-ignored"]
async fn ttl_expires_value() {
    let h = handle();
    let k = key("ttl");
    h.set_ex(&k, "short", Duration::from_secs(1))
        .await
        .expect("set");
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_eq!(h.get(&k).await.expect("get"), None);
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_URL and pass --include-ignored"]
async fn clones_share_one_connection() {
    let h = handle();
    h.ping().await.expect("first ping connects");
    let h2 = h.clone();
    h2.ping().await.expect("clone reuses the manager");
    assert!(format!("{h2:?}").contains("connected: true"));
}

#[tokio::test]
#[ignore = "needs Redis: set REDIS_URL and pass --include-ignored"]
async fn wrong_password_is_rejected() {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL");
    let bad = url.replacen("://:", "://:wrong-", 1);
    let h = RedisHandle::new(&SecretString::from(bad)).expect("valid url");
    let err = h.ping().await.expect_err("auth must fail");
    assert!(!err.to_string().contains("wrong-"), "{err}");
}
