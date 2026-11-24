//! Plan-based gating (T03.07).
//!
//! The third gate of the module system: after "the module is on" and "the role
//! has the permission", a request needs "the tenant's plan includes the
//! feature" and, for creating things, "one more is within the limit".
//!
//! ```ignore
//! gate.require_feature(tenant, "redirects").await?;        // 403 feature_disabled
//! gate.check_limit(tenant, "max_domains", current + 1).await?; // 409 limit_reached
//! ```
//!
//! * A [`PlanSnapshot`] (the feature keys and limits of the tenant's plan) comes
//!   from a [`PlanSource`], which phase 04 backs with the plans tables.
//! * Snapshots are cached in Redis for a few minutes. Whoever changes a plan,
//!   or moves a tenant to another plan, calls [`FeatureGate::invalidate`]; the
//!   time limit only bounds how long a missed invalidation can matter.
//! * Unknown is closed: a feature the plan does not list is off, a limit it
//!   does not list is zero, and a tenant without a plan is refused.
//! * A limit check is a decision, not a reservation. To stay correct under
//!   concurrent requests, call it inside the transaction that counts and
//!   inserts (phase 04 serialises creations per tenant).

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use builtpanel_cache::RedisHandle;
use builtpanel_http_kit::AppError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A boxed future, so the traits can be used as `dyn`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// How long a snapshot may be served from the cache.
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// A numeric limit of a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Limit {
    /// No limit.
    Unlimited,
    /// At most this many (zero or more).
    Max(i64),
}

impl Limit {
    /// From the stored number: `-1` is unlimited, zero or more is a maximum.
    /// Anything else is treated as zero, the safe reading of a corrupt value.
    #[must_use]
    pub fn from_stored(value: i64) -> Self {
        match value {
            -1 => Self::Unlimited,
            n if n >= 0 => Self::Max(n),
            _ => Self::Max(0),
        }
    }

    /// Whether `wanted` (the count after the action) is allowed.
    #[must_use]
    pub fn allows(self, wanted: i64) -> bool {
        match self {
            Self::Unlimited => true,
            Self::Max(max) => wanted <= max,
        }
    }
}

/// What a tenant's plan includes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSnapshot {
    /// Feature keys that are on.
    pub features: BTreeSet<String>,
    /// Limits by key.
    pub limits: BTreeMap<String, Limit>,
}

/// Why a check refused (or could not be made).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GateError {
    /// The plan does not include the feature.
    #[error("the plan does not include the feature `{0}`")]
    FeatureDisabled(String),
    /// The action would go over a limit.
    #[error("the limit `{key}` ({limit}) would be exceeded")]
    LimitReached {
        /// The limit's key.
        key: String,
        /// The maximum (zero when the plan lists none).
        limit: i64,
        /// The count the action would reach.
        wanted: i64,
    },
    /// The tenant has no plan.
    #[error("the tenant has no plan")]
    NoPlan,
    /// The plan could not be read or the cache failed in a way that matters.
    #[error("plan lookup failed: {0}")]
    Backend(String),
}

impl From<GateError> for AppError {
    fn from(err: GateError) -> Self {
        match err {
            GateError::FeatureDisabled(_) => AppError::forbidden(
                "feature_disabled",
                "This feature is not included in the plan.",
            ),
            GateError::LimitReached { limit, .. } => AppError::conflict(
                "limit_reached",
                format!("The plan allows at most {limit} of these."),
            ),
            GateError::NoPlan => AppError::forbidden("no_plan", "No plan is assigned."),
            GateError::Backend(message) => AppError::internal(std::io::Error::other(message)),
        }
    }
}

/// Where plans come from.
pub trait PlanSource: Send + Sync + 'static {
    /// The plan of a tenant; `None` if it has none.
    fn load(&self, tenant: Uuid) -> BoxFuture<'_, Result<Option<PlanSnapshot>, GateError>>;
}

/// A place to keep snapshots between requests.
pub trait SnapshotCache: Send + Sync + 'static {
    /// The cached snapshot, if there is a fresh one.
    fn get(&self, tenant: Uuid) -> BoxFuture<'_, Result<Option<PlanSnapshot>, String>>;
    /// Keeps a snapshot for `ttl`.
    fn put(
        &self,
        tenant: Uuid,
        snapshot: PlanSnapshot,
        ttl: Duration,
    ) -> BoxFuture<'_, Result<(), String>>;
    /// Forgets a tenant's snapshot.
    fn forget(&self, tenant: Uuid) -> BoxFuture<'_, Result<(), String>>;
}

/// Snapshots in Redis, under `bp:plan:<tenant>`.
#[derive(Clone)]
pub struct RedisCache {
    redis: RedisHandle,
}

impl RedisCache {
    /// A cache on this connection.
    #[must_use]
    pub fn new(redis: RedisHandle) -> Self {
        Self { redis }
    }

    fn key(tenant: Uuid) -> String {
        format!("bp:plan:{tenant}")
    }
}

impl SnapshotCache for RedisCache {
    fn get(&self, tenant: Uuid) -> BoxFuture<'_, Result<Option<PlanSnapshot>, String>> {
        Box::pin(async move {
            let text = self
                .redis
                .get(&Self::key(tenant))
                .await
                .map_err(|e| e.to_string())?;
            // A damaged entry is a miss: the plan is loaded again and replaces it.
            Ok(text.and_then(|t| serde_json::from_str(&t).ok()))
        })
    }

    fn put(
        &self,
        tenant: Uuid,
        snapshot: PlanSnapshot,
        ttl: Duration,
    ) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let text = serde_json::to_string(&snapshot).map_err(|e| e.to_string())?;
            self.redis
                .set_ex(&Self::key(tenant), &text, ttl)
                .await
                .map_err(|e| e.to_string())
        })
    }

    fn forget(&self, tenant: Uuid) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.redis
                .del(&Self::key(tenant))
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
    }
}

/// Snapshots in memory (tests, and single-process fallbacks).
#[derive(Default)]
pub struct MemoryCache {
    entries: Mutex<BTreeMap<Uuid, PlanSnapshot>>,
}

impl SnapshotCache for MemoryCache {
    fn get(&self, tenant: Uuid) -> BoxFuture<'_, Result<Option<PlanSnapshot>, String>> {
        Box::pin(async move {
            Ok(self
                .entries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&tenant)
                .cloned())
        })
    }

    fn put(
        &self,
        tenant: Uuid,
        snapshot: PlanSnapshot,
        _ttl: Duration,
    ) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.entries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(tenant, snapshot);
            Ok(())
        })
    }

    fn forget(&self, tenant: Uuid) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.entries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&tenant);
            Ok(())
        })
    }
}

/// The gate. Cheap to clone.
#[derive(Clone)]
pub struct FeatureGate {
    source: Arc<dyn PlanSource>,
    cache: Arc<dyn SnapshotCache>,
    ttl: Duration,
}

impl FeatureGate {
    /// A gate over a plan source and a cache.
    #[must_use]
    pub fn new(source: Arc<dyn PlanSource>, cache: Arc<dyn SnapshotCache>, ttl: Duration) -> Self {
        Self { source, cache, ttl }
    }

    /// The tenant's plan: from the cache, else from the source (and cached).
    /// A cache that fails is skipped, never fatal: the plan is then read from
    /// its source, which is always right.
    ///
    /// # Errors
    /// [`GateError::NoPlan`] or [`GateError::Backend`].
    pub async fn snapshot(&self, tenant: Uuid) -> Result<PlanSnapshot, GateError> {
        match self.cache.get(tenant).await {
            Ok(Some(snapshot)) => return Ok(snapshot),
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(%tenant, error = %err, "plan cache unavailable: reading the plan directly")
            }
        }
        let snapshot = self.source.load(tenant).await?.ok_or(GateError::NoPlan)?;
        if let Err(err) = self.cache.put(tenant, snapshot.clone(), self.ttl).await {
            tracing::warn!(%tenant, error = %err, "could not cache the plan");
        }
        Ok(snapshot)
    }

    /// Succeeds if the tenant's plan includes the feature.
    ///
    /// # Errors
    /// [`GateError::FeatureDisabled`] (403 `feature_disabled`), or lookup errors.
    pub async fn require_feature(&self, tenant: Uuid, key: &str) -> Result<(), GateError> {
        if self.snapshot(tenant).await?.features.contains(key) {
            Ok(())
        } else {
            Err(GateError::FeatureDisabled(key.to_owned()))
        }
    }

    /// Succeeds if `wanted` (the count the action would reach, so
    /// `current + 1` when creating one thing) is within the limit.
    ///
    /// # Errors
    /// [`GateError::LimitReached`] (409 `limit_reached`), or lookup errors.
    pub async fn check_limit(&self, tenant: Uuid, key: &str, wanted: i64) -> Result<(), GateError> {
        let snapshot = self.snapshot(tenant).await?;
        // A plan without the limit allows none.
        let limit = snapshot.limits.get(key).copied().unwrap_or(Limit::Max(0));
        if limit.allows(wanted) {
            Ok(())
        } else {
            Err(GateError::LimitReached {
                key: key.to_owned(),
                limit: match limit {
                    Limit::Max(n) => n,
                    Limit::Unlimited => -1,
                },
                wanted,
            })
        }
    }

    /// Forgets the tenant's cached plan. Call it whenever the plan, or the
    /// plan the tenant is on, changes.
    ///
    /// # Errors
    /// [`GateError::Backend`] if the cache could not be cleared: the caller
    /// should report it, because the old plan may be served until the time
    /// limit passes.
    pub async fn invalidate(&self, tenant: Uuid) -> Result<(), GateError> {
        self.cache.forget(tenant).await.map_err(GateError::Backend)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A plan source that counts loads and whose plan can be changed.
    struct Fake {
        plan: Mutex<Option<PlanSnapshot>>,
        loads: AtomicUsize,
    }

    impl Fake {
        fn new(plan: Option<PlanSnapshot>) -> Arc<Self> {
            Arc::new(Self {
                plan: Mutex::new(plan),
                loads: AtomicUsize::new(0),
            })
        }
    }

    impl PlanSource for Fake {
        fn load(&self, _tenant: Uuid) -> BoxFuture<'_, Result<Option<PlanSnapshot>, GateError>> {
            Box::pin(async move {
                self.loads.fetch_add(1, Ordering::SeqCst);
                Ok(self.plan.lock().unwrap().clone())
            })
        }
    }

    fn plan(features: &[&str], limits: &[(&str, Limit)]) -> PlanSnapshot {
        PlanSnapshot {
            features: features.iter().map(|f| (*f).to_owned()).collect(),
            limits: limits.iter().map(|(k, l)| ((*k).to_owned(), *l)).collect(),
        }
    }

    fn gate(source: &Arc<Fake>) -> FeatureGate {
        FeatureGate::new(
            source.clone(),
            Arc::new(MemoryCache::default()),
            DEFAULT_TTL,
        )
    }

    #[tokio::test]
    async fn a_feature_that_is_off_is_refused_with_feature_disabled() {
        let source = Fake::new(Some(plan(&["domains"], &[])));
        let gate = gate(&source);
        let tenant = Uuid::now_v7();
        assert_eq!(gate.require_feature(tenant, "domains").await, Ok(()));
        let err = gate.require_feature(tenant, "redirects").await.unwrap_err();
        assert_eq!(err, GateError::FeatureDisabled("redirects".into()));
        match AppError::from(err) {
            AppError::Forbidden { code, .. } => assert_eq!(code, "feature_disabled"),
            other => panic!("not a 403: {other:?}"),
        }
    }

    #[tokio::test]
    async fn going_over_a_limit_is_refused_with_limit_reached() {
        let source = Fake::new(Some(plan(
            &[],
            &[
                ("max_domains", Limit::Max(3)),
                ("max_mail", Limit::Unlimited),
                ("max_zero", Limit::Max(0)),
            ],
        )));
        let gate = gate(&source);
        let t = Uuid::now_v7();
        // current 2 -> creating one more makes 3: allowed. Current 3 -> 4: refused.
        assert_eq!(gate.check_limit(t, "max_domains", 3).await, Ok(()));
        let err = gate.check_limit(t, "max_domains", 4).await.unwrap_err();
        assert_eq!(
            err,
            GateError::LimitReached {
                key: "max_domains".into(),
                limit: 3,
                wanted: 4
            }
        );
        match AppError::from(err) {
            AppError::Conflict { code, detail } => {
                assert_eq!(code, "limit_reached");
                assert!(detail.contains('3'));
            }
            other => panic!("not a 409: {other:?}"),
        }
        assert_eq!(gate.check_limit(t, "max_mail", i64::MAX).await, Ok(()));
        assert!(gate.check_limit(t, "max_zero", 1).await.is_err());
        // Counting nothing is always fine, even at zero.
        assert_eq!(gate.check_limit(t, "max_zero", 0).await, Ok(()));
        // A limit the plan does not list is zero: unknown is closed.
        assert!(gate.check_limit(t, "max_unlisted", 1).await.is_err());
    }

    #[tokio::test]
    async fn a_tenant_without_a_plan_gets_nothing() {
        let source = Fake::new(None);
        let gate = gate(&source);
        let t = Uuid::now_v7();
        assert_eq!(
            gate.require_feature(t, "domains").await,
            Err(GateError::NoPlan)
        );
        assert_eq!(
            gate.check_limit(t, "max_domains", 1).await,
            Err(GateError::NoPlan)
        );
        match AppError::from(GateError::NoPlan) {
            AppError::Forbidden { code, .. } => assert_eq!(code, "no_plan"),
            other => panic!("{other:?}"),
        }
        // A missing plan is not cached: it is looked up again.
        assert_eq!(source.loads.load(Ordering::SeqCst), 2);
        *source.plan.lock().unwrap() = Some(plan(&["domains"], &[]));
        assert_eq!(gate.require_feature(t, "domains").await, Ok(()));
    }

    #[tokio::test]
    async fn the_plan_is_read_once_until_it_is_invalidated() {
        let source = Fake::new(Some(plan(&["domains"], &[("max_domains", Limit::Max(1))])));
        let gate = gate(&source);
        let t = Uuid::now_v7();
        for _ in 0..5 {
            gate.require_feature(t, "domains").await.unwrap();
            gate.check_limit(t, "max_domains", 1).await.unwrap();
        }
        assert_eq!(
            source.loads.load(Ordering::SeqCst),
            1,
            "served from the cache"
        );

        // The plan changes. Until the cache is told, the old plan is served ...
        *source.plan.lock().unwrap() = Some(plan(
            &["domains", "redirects"],
            &[("max_domains", Limit::Max(10))],
        ));
        assert!(gate.require_feature(t, "redirects").await.is_err());
        assert!(gate.check_limit(t, "max_domains", 2).await.is_err());
        // ... and after invalidation the new one is.
        gate.invalidate(t).await.unwrap();
        assert_eq!(gate.require_feature(t, "redirects").await, Ok(()));
        assert_eq!(gate.check_limit(t, "max_domains", 10).await, Ok(()));
        assert_eq!(source.loads.load(Ordering::SeqCst), 2);

        // Other tenants are unaffected.
        let other = Uuid::now_v7();
        gate.require_feature(other, "redirects").await.unwrap();
        assert_eq!(source.loads.load(Ordering::SeqCst), 3);
        gate.invalidate(t).await.unwrap();
        gate.require_feature(t, "redirects").await.unwrap();
        assert_eq!(source.loads.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn stored_numbers_become_limits() {
        assert_eq!(Limit::from_stored(-1), Limit::Unlimited);
        assert_eq!(Limit::from_stored(0), Limit::Max(0));
        assert_eq!(Limit::from_stored(25), Limit::Max(25));
        assert_eq!(
            Limit::from_stored(-7),
            Limit::Max(0),
            "corrupt values close the gate"
        );
        assert!(Limit::Max(5).allows(5) && !Limit::Max(5).allows(6));
        assert!(Limit::Unlimited.allows(i64::MAX));
    }

    /// A cache that always fails, to prove the gate falls back to the source.
    struct Broken;

    impl SnapshotCache for Broken {
        fn get(&self, _: Uuid) -> BoxFuture<'_, Result<Option<PlanSnapshot>, String>> {
            Box::pin(async { Err("redis is down".to_owned()) })
        }
        fn put(&self, _: Uuid, _: PlanSnapshot, _: Duration) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(async { Err("redis is down".to_owned()) })
        }
        fn forget(&self, _: Uuid) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(async { Err("redis is down".to_owned()) })
        }
    }

    #[tokio::test]
    async fn a_failing_cache_costs_speed_not_correctness() {
        let source = Fake::new(Some(plan(&["domains"], &[])));
        let gate = FeatureGate::new(source.clone(), Arc::new(Broken), DEFAULT_TTL);
        let t = Uuid::now_v7();
        assert_eq!(gate.require_feature(t, "domains").await, Ok(()));
        assert!(gate.require_feature(t, "mail").await.is_err());
        assert_eq!(
            source.loads.load(Ordering::SeqCst),
            2,
            "no cache, so every check reads the plan"
        );
        // But a failed invalidation is reported: the caller must know.
        assert_eq!(
            gate.invalidate(t).await,
            Err(GateError::Backend("redis is down".into()))
        );
    }
}
