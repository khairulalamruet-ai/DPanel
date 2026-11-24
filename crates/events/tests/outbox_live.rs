//! The outbox and the relay against real PostgreSQL.
//! Ignored by default; run with DATABASE_URL (superuser) and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use builtpanel_events::{PassStats, Relay, Settings, publish};
use builtpanel_kernel::{
    Event, Manifest, Module,
    module::{BoxFuture, EventError},
};
use sqlx::PgPool;
use uuid::Uuid;

/// A module that records what it is told and can be made to fail.
struct Listener {
    manifest: Manifest,
    seen: Mutex<Vec<Event>>,
    fail_first: AtomicUsize,
    slow: bool,
}

impl Module for Listener {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn on_event<'a>(&'a self, event: &'a Event) -> BoxFuture<'a, Result<(), EventError>> {
        Box::pin(async move {
            if self.slow {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
            if self
                .fail_first
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                return Err(EventError("not now".into()));
            }
            self.seen.lock().unwrap().push(event.clone());
            Ok(())
        })
    }
}

fn listener(name: &str, fail_first: usize, slow: bool) -> Arc<Listener> {
    let text = format!(
        "[module]\nname = \"{name}\"\nversion = \"1.0.0\"\ntitle = {{ en = \"{name}\" }}\ncategory = \"web\"\n"
    );
    Arc::new(Listener {
        manifest: Manifest::parse(name, &text).unwrap(),
        seen: Mutex::new(Vec::new()),
        fail_first: AtomicUsize::new(fail_first),
        slow,
    })
}

fn settings() -> Settings {
    Settings {
        poll: Duration::from_millis(20),
        batch: 50,
        max_attempts: 3,
        retry_base: Duration::ZERO,
        retry_cap: Duration::ZERO,
        handler_timeout: Duration::from_millis(300),
    }
}

fn relay(pool: &PgPool, modules: Vec<Arc<Listener>>, settings: Settings) -> Relay {
    let modules: Vec<Arc<dyn Module>> = modules.into_iter().map(|m| m as Arc<dyn Module>).collect();
    Relay::new(pool.clone(), Arc::new(move || modules.clone()), settings)
}

fn tenant_created() -> (Uuid, Event) {
    let id = Uuid::now_v7();
    (id, Event::TenantCreated { tenant_id: id })
}

async fn publish_committed(pool: &PgPool, event: &Event) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    let id = publish(&mut tx, event).await.unwrap();
    tx.commit().await.unwrap();
    id
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_event_of_a_rolled_back_transaction_is_never_delivered(pool: PgPool) {
    let a = listener("alpha", 0, false);
    let relay = relay(&pool, vec![a.clone()], settings());

    // The change fails after the event was written: both are gone.
    let (_, lost) = tenant_created();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO nodes (name) VALUES ('rolled-back')")
        .execute(&mut *tx)
        .await
        .unwrap();
    publish(&mut tx, &lost).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(relay.run_once().await.unwrap(), PassStats::default());
    assert!(a.seen.lock().unwrap().is_empty());
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);

    // Not delivered before the commit either: an open transaction is invisible.
    let (_, kept) = tenant_created();
    let mut tx = pool.begin().await.unwrap();
    publish(&mut tx, &kept).await.unwrap();
    assert_eq!(relay.run_once().await.unwrap().delivered, 0);
    assert!(a.seen.lock().unwrap().is_empty());
    tx.commit().await.unwrap();

    // Committed: delivered, exactly once.
    assert_eq!(relay.run_once().await.unwrap().delivered, 1);
    assert_eq!(*a.seen.lock().unwrap(), vec![kept]);
    assert_eq!(relay.run_once().await.unwrap(), PassStats::default());
    assert_eq!(a.seen.lock().unwrap().len(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn every_active_module_gets_every_event_in_order(pool: PgPool) {
    let (a, b) = (listener("alpha", 0, false), listener("beta", 0, false));
    let relay = relay(&pool, vec![a.clone(), b.clone()], settings());
    let mut sent = Vec::new();
    for _ in 0..5 {
        let (_, event) = tenant_created();
        publish_committed(&pool, &event).await;
        sent.push(event);
    }
    let name = Event::ModuleDisabled { name: "dns".into() };
    publish_committed(&pool, &name).await;
    sent.push(name);

    assert_eq!(relay.run_once().await.unwrap().delivered, 6);
    assert_eq!(*a.seen.lock().unwrap(), sent);
    assert_eq!(*b.seen.lock().unwrap(), sent);
    let (pending, dead) = builtpanel_database::outbox::counts(&pool).await.unwrap();
    assert_eq!((pending, dead), (0, 0));
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_failing_module_is_retried_and_a_hopeless_event_is_set_aside(pool: PgPool) {
    // beta fails twice, then works: the event arrives on the third pass.
    let (a, b) = (listener("alpha", 0, false), listener("beta", 2, false));
    let r = relay(&pool, vec![a.clone(), b.clone()], settings());
    let (_, event) = tenant_created();
    let id = publish_committed(&pool, &event).await;
    assert_eq!(
        r.run_once().await.unwrap(),
        PassStats {
            delivered: 0,
            failed: 1,
            dead: 0
        }
    );
    let error: String = sqlx::query_scalar("SELECT last_error FROM outbox WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(error, "module `beta`: not now");
    assert_eq!(r.run_once().await.unwrap().failed, 1);
    assert_eq!(r.run_once().await.unwrap().delivered, 1);
    assert_eq!(b.seen.lock().unwrap().len(), 1);
    // alpha was asked each time: handlers must be safe to repeat.
    assert_eq!(a.seen.lock().unwrap().len(), 3);

    // A module that never works: after max_attempts the event is dead, and the
    // next event still gets through.
    let never = listener("gamma", usize::MAX, false);
    let r = relay(&pool, vec![never.clone()], settings());
    let (_, doomed) = tenant_created();
    let doomed_id = publish_committed(&pool, &doomed).await;
    assert_eq!(r.run_once().await.unwrap().failed, 1);
    assert_eq!(r.run_once().await.unwrap().failed, 1);
    assert_eq!(
        r.run_once().await.unwrap(),
        PassStats {
            delivered: 0,
            failed: 0,
            dead: 1
        }
    );
    assert_eq!(
        r.run_once().await.unwrap(),
        PassStats::default(),
        "dead events are left alone"
    );
    let (attempts, is_dead): (i32, bool) =
        sqlx::query_as("SELECT attempts, dead_at IS NOT NULL FROM outbox WHERE id = $1")
            .bind(doomed_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((attempts, is_dead), (3, true));
    assert_eq!(
        builtpanel_database::outbox::counts(&pool).await.unwrap(),
        (0, 1)
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_stuck_module_and_an_unknown_event_do_not_block_the_relay(pool: PgPool) {
    let stuck = listener("slow", 0, true);
    let r = relay(&pool, vec![stuck], settings());
    let (_, event) = tenant_created();
    let id = publish_committed(&pool, &event).await;
    let started = std::time::Instant::now();
    assert_eq!(r.run_once().await.unwrap().failed, 1);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the time limit applies"
    );
    let error: String = sqlx::query_scalar("SELECT last_error FROM outbox WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(error, "module `slow` did not finish in time");

    // An event written by a newer build.
    let a = listener("alpha", 0, false);
    let r = relay(&pool, vec![a.clone()], settings());
    sqlx::query("UPDATE outbox SET dead_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO outbox (event_type, payload) VALUES ('moon_landed', '{\"type\":\"moon_landed\"}')")
        .execute(&pool)
        .await
        .unwrap();
    let (_, known) = tenant_created();
    publish_committed(&pool, &known).await;
    let stats = r.run_once().await.unwrap();
    assert_eq!(
        (stats.delivered, stats.failed),
        (1, 1),
        "the known event is not held up"
    );
    assert_eq!(*a.seen.lock().unwrap(), vec![known]);
    let error: String =
        sqlx::query_scalar("SELECT last_error FROM outbox WHERE event_type = 'moon_landed'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(error, "this build does not understand `moon_landed` events");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn several_relays_deliver_each_event_once(pool: PgPool) {
    let a = listener("alpha", 0, false);
    for _ in 0..60 {
        let (_, event) = tenant_created();
        publish_committed(&pool, &event).await;
    }
    let mut small = settings();
    small.batch = 7;
    let mut tasks = Vec::new();
    for _ in 0..5 {
        let r = relay(&pool, vec![a.clone()], small.clone());
        let pool = pool.clone();
        tasks.push(tokio::spawn(async move {
            let mut delivered = 0;
            loop {
                let stats = r.run_once().await.unwrap();
                if stats == PassStats::default() {
                    // Another relay may still hold rows; look once more before leaving.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    if builtpanel_database::outbox::counts(&pool).await.unwrap().0 == 0 {
                        return delivered;
                    }
                }
                delivered += stats.delivered;
            }
        }));
    }
    let mut total = 0;
    for task in tasks {
        total += task.await.unwrap();
    }
    assert_eq!(total, 60);
    let seen = a.seen.lock().unwrap();
    let unique: std::collections::HashSet<_> = seen.iter().map(|e| format!("{e:?}")).collect();
    assert_eq!((seen.len(), unique.len()), (60, 60));
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_running_relay_delivers_and_stops_when_told(pool: PgPool) {
    let a = listener("alpha", 0, false);
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(relay(&pool, vec![a.clone()], settings()).run(stopped));
    let (_, event) = tenant_created();
    publish_committed(&pool, &event).await;
    for _ in 0..100 {
        if !a.seen.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(*a.seen.lock().unwrap(), vec![event]);
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
}
