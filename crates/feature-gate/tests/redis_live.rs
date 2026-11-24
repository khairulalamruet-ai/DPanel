//! The Redis snapshot cache against a real Redis.
//! Ignored by default; run with REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use builtpanel_cache::RedisHandle;
use builtpanel_feature_gate::{
    BoxFuture, FeatureGate, GateError, Limit, PlanSnapshot, PlanSource, RedisCache, SnapshotCache,
};
use secrecy::SecretString;
use uuid::Uuid;

struct Counting(AtomicUsize);

impl PlanSource for Counting {
    fn load(&self, _: Uuid) -> BoxFuture<'_, Result<Option<PlanSnapshot>, GateError>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Some(PlanSnapshot {
                features: ["domains".to_owned()].into(),
                limits: [("max_domains".to_owned(), Limit::Max(5))].into(),
            }))
        })
    }
}

fn redis() -> RedisHandle {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    RedisHandle::new(&SecretString::from(url)).unwrap()
}

#[tokio::test]
#[ignore = "needs Redis"]
async fn snapshots_are_shared_through_redis_and_forgotten_on_invalidate() {
    let source = Arc::new(Counting(AtomicUsize::new(0)));
    let tenant = Uuid::now_v7();
    // Two gates, as two control plane processes would be.
    let a = FeatureGate::new(
        source.clone(),
        Arc::new(RedisCache::new(redis())),
        Duration::from_secs(60),
    );
    let b = FeatureGate::new(
        source.clone(),
        Arc::new(RedisCache::new(redis())),
        Duration::from_secs(60),
    );
    a.require_feature(tenant, "domains").await.unwrap();
    b.check_limit(tenant, "max_domains", 5).await.unwrap();
    assert!(b.check_limit(tenant, "max_domains", 6).await.is_err());
    assert_eq!(
        source.0.load(Ordering::SeqCst),
        1,
        "the second process used the first one's read"
    );

    // One process invalidates; the other sees it at once.
    a.invalidate(tenant).await.unwrap();
    b.require_feature(tenant, "domains").await.unwrap();
    assert_eq!(source.0.load(Ordering::SeqCst), 2);

    // A damaged entry is a miss, not an error.
    let cache = RedisCache::new(redis());
    redis()
        .set_ex(
            &format!("bp:plan:{tenant}"),
            "{not json",
            Duration::from_secs(30),
        )
        .await
        .unwrap();
    assert_eq!(cache.get(tenant).await.unwrap(), None);
    a.require_feature(tenant, "domains").await.unwrap();
    assert_eq!(source.0.load(Ordering::SeqCst), 3);
    a.invalidate(tenant).await.unwrap();
}
