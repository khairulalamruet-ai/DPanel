//! Failure injection against the operations ledger (T19.06): while workers keep claiming
//! operations, nodes are unreachable, answers get lost, nodes vanish after taking a job and
//! answer late, and the janitor times jobs out. Whatever happens, no operation is lost and
//! every one ends up done exactly once in the ledger.
//! Ignored by default; run with DATABASE_URL (superuser) and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use builtpanel_database::operations as ledger;
use builtpanel_node_protocol::Operation;
use builtpanel_operations::{Operations, Request};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// A small deterministic generator, so a failing run can be repeated.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn no_operation_is_lost_when_nodes_and_answers_go_missing(pool: PgPool) {
    let mut nodes = Vec::new();
    for name in ["chaos-a", "chaos-b"] {
        let id: Uuid = sqlx::query_scalar("INSERT INTO nodes (name) VALUES ($1) RETURNING id")
            .bind(name)
            .fetch_one(&pool)
            .await
            .unwrap();
        nodes.push(id);
    }
    let ops = Operations::new(pool.clone());
    let total = 200u64;
    for i in 0..total {
        let mut request = Request::new(
            nodes[(i % 2) as usize],
            Operation::Ping { nonce: i },
            format!("chaos-{i}"),
        );
        // The most deliveries a job may have. The janitor here times every sent job out at
        // once, so on a busy machine one unlucky job can use them all up: that is a visible
        // failure (checked below), not a lost job.
        request.max_attempts = 20;
        ops.enqueue(&request).await.unwrap();
    }

    // What the nodes did (a job may run more than once: the agent's jobs are idempotent) and
    // how often the ledger accepted an answer for each job.
    let ran: Arc<Mutex<HashMap<Uuid, u32>>> = Arc::default();
    let accepted: Arc<Mutex<HashMap<Uuid, u32>>> = Arc::default();

    // Chaos: four workers, a janitor, and a fate for every job they claim.
    let mut workers = Vec::new();
    for seed in 1..=4u64 {
        let (pool, nodes) = (pool.clone(), nodes.clone());
        let (ran, accepted) = (ran.clone(), accepted.clone());
        workers.push(tokio::spawn(async move {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            for round in 0..40 {
                for op in ledger::claim_due(&pool, &nodes, 5).await.unwrap() {
                    match rng.next() % 6 {
                        // The node is unreachable: nothing was sent.
                        0 => {
                            ledger::release_undelivered(&pool, op.id).await.unwrap();
                        }
                        // Sent, and the node died before it did anything.
                        1 => {}
                        // The node did it and the answer arrived.
                        2 => {
                            *ran.lock().unwrap().entry(op.id).or_default() += 1;
                            if ledger::succeed(&pool, op.id, &json!({"ok": true}))
                                .await
                                .unwrap()
                            {
                                *accepted.lock().unwrap().entry(op.id).or_default() += 1;
                            }
                        }
                        // The node did it, and the answer was lost on the way.
                        3 => *ran.lock().unwrap().entry(op.id).or_default() += 1,
                        // The node tried and said "not now".
                        4 => {
                            let error =
                                json!({"code": "busy", "message": "later", "retryable": true});
                            ledger::fail(&pool, op.id, &error, Some(0)).await.unwrap();
                        }
                        // The node did it, but the answer came after the janitor gave up.
                        _ => {
                            *ran.lock().unwrap().entry(op.id).or_default() += 1;
                            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                            if ledger::succeed(&pool, op.id, &json!({"ok": true, "late": true}))
                                .await
                                .unwrap()
                            {
                                *accepted.lock().unwrap().entry(op.id).or_default() += 1;
                            }
                        }
                    }
                }
                if round % 3 == 0 {
                    // Dispatched and never answered: back in the queue.
                    ledger::time_out_dispatched(&pool, 0, 0).await.unwrap();
                }
            }
        }));
    }
    for worker in workers {
        worker.await.unwrap();
    }

    // Calm: the nodes are back and answer everything; the janitor still runs.
    // Bounded by time, not by rounds: on a busy machine a round takes far longer.
    let calm_until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < calm_until {
        let open: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM operations WHERE status NOT IN ('succeeded', 'failed')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        if open == 0 {
            break;
        }
        for op in ledger::claim_due(&pool, &nodes, 50).await.unwrap() {
            *ran.lock().unwrap().entry(op.id).or_default() += 1;
            if ledger::succeed(&pool, op.id, &json!({"ok": true}))
                .await
                .unwrap()
            {
                *accepted.lock().unwrap().entry(op.id).or_default() += 1;
            }
        }
        ledger::time_out_dispatched(&pool, 0, 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // Nothing was lost: every job is finished, none is still waiting or running. A job that
    // used up all its deliveries is `failed` for everybody to see (never silently gone); every
    // other job succeeded.
    let still_open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE status NOT IN ('succeeded', 'failed')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(still_open, 0, "jobs still open");
    let failed: Vec<(i32, i32, Option<serde_json::Value>)> = sqlx::query_as(
        "SELECT attempts, max_attempts, error FROM operations WHERE status = 'failed'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        failed
            .iter()
            .all(|(attempts, max, error)| attempts == max && error.is_some()),
        "a job failed without using up its deliveries: {failed:?}"
    );
    let succeeded: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operations WHERE status = 'succeeded'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(succeeded + failed.len() as i64, total as i64);
    // Each finished job was accepted exactly once, however many times it ran or answered.
    let accepted = accepted.lock().unwrap();
    let ran = ran.lock().unwrap();
    assert_eq!(accepted.len() as i64, succeeded);
    assert!(accepted.values().all(|n| *n == 1), "{accepted:?}");
    // And every accepted job really was run by a node.
    assert!(accepted.keys().all(|id| ran.contains_key(id)));
    let reruns = ran.values().filter(|n| **n > 1).count();
    eprintln!("{reruns} of {total} jobs ran more than once (at-least-once delivery)");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn a_job_nobody_can_deliver_in_time_fails_visibly_instead_of_hanging(pool: PgPool) {
    let node: Uuid = sqlx::query_scalar("INSERT INTO nodes (name) VALUES ('chaos-c') RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    let ops = Operations::new(pool.clone());
    let mut request = Request::new(node, Operation::Ping { nonce: 1 }, "doomed");
    request.ttl = std::time::Duration::from_secs(1);
    ops.enqueue(&request).await.unwrap();
    // The node stays away until the job is past its time.
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    assert!(
        ledger::claim_due(&pool, &[node], 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(ledger::expire_overdue(&pool).await.unwrap(), 1);
    let (status, error): (String, serde_json::Value) =
        sqlx::query_as("SELECT status::text, error FROM operations")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "failed");
    assert_eq!(error["code"], "expired");
}
