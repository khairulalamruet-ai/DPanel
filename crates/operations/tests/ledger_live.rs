//! The operations ledger against real PostgreSQL.
//! Ignored by default; run with DATABASE_URL (superuser) and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::time::Duration;

use builtpanel_node_protocol::Operation;
use builtpanel_operations::{Enqueued, Operations, OperationsError, Request, Status, envelope};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

async fn node(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO nodes (name) VALUES ($1) RETURNING id")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn ping(node: Uuid, key: &str, nonce: u64) -> Request {
    Request::new(node, Operation::Ping { nonce }, key)
}

async fn count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM operations")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_same_key_twice_gives_one_row(pool: PgPool) {
    let ops = Operations::new(pool.clone());
    let n = node(&pool, "node-a").await;

    let first = ops.enqueue(&ping(n, "k1", 7)).await.unwrap();
    let Enqueued::Created(row) = &first else {
        panic!("first call must create")
    };
    assert_eq!(
        (row.status, row.attempts, row.kind.as_str()),
        (Status::Pending, 0, "ping")
    );
    assert_eq!(row.payload, json!({"type": "ping", "nonce": 7}));

    let second = ops.enqueue(&ping(n, "k1", 7)).await.unwrap();
    assert!(matches!(&second, Enqueued::Existing(r) if r.id == row.id));
    assert_eq!(count(&pool).await, 1);

    // Same key, different request: refused, nothing swapped.
    let err = ops.enqueue(&ping(n, "k1", 8)).await.unwrap_err();
    assert!(matches!(err, OperationsError::KeyReused), "{err:?}");
    let err = ops
        .enqueue(&Request::new(n, Operation::ReportCapabilities, "k1"))
        .await
        .unwrap_err();
    assert!(matches!(err, OperationsError::KeyReused), "{err:?}");
    assert_eq!(count(&pool).await, 1);

    // Keys are per node.
    let other = node(&pool, "node-b").await;
    assert!(matches!(
        ops.enqueue(&ping(other, "k1", 7)).await.unwrap(),
        Enqueued::Created(_)
    ));
    assert_eq!(count(&pool).await, 2);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn concurrent_requests_with_one_key_still_make_one_row(pool: PgPool) {
    let ops = Operations::new(pool.clone());
    let n = node(&pool, "node-a").await;
    let mut tasks = Vec::new();
    for _ in 0..24 {
        let ops = ops.clone();
        tasks.push(tokio::spawn(async move {
            ops.enqueue(&ping(n, "same", 1)).await
        }));
    }
    let mut created = 0;
    let mut ids = std::collections::HashSet::new();
    for t in tasks {
        let e = t.await.unwrap().unwrap();
        created += usize::from(matches!(e, Enqueued::Created(_)));
        ids.insert(e.operation().id);
    }
    assert_eq!((created, ids.len()), (1, 1));
    assert_eq!(count(&pool).await, 1);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn input_is_validated_and_revoked_nodes_take_no_work(pool: PgPool) {
    let ops = Operations::new(pool.clone());
    let n = node(&pool, "node-a").await;
    for key in ["", "has space", &"k".repeat(129)] {
        assert!(matches!(
            ops.enqueue(&ping(n, key, 1)).await.unwrap_err(),
            OperationsError::InvalidKey
        ));
    }
    let mut r = ping(n, "ok", 1);
    r.ttl = Duration::ZERO;
    assert!(matches!(
        ops.enqueue(&r).await.unwrap_err(),
        OperationsError::InvalidTtl
    ));
    r.ttl = Duration::from_secs(3601);
    assert!(matches!(
        ops.enqueue(&r).await.unwrap_err(),
        OperationsError::InvalidTtl
    ));
    r.ttl = Duration::from_secs(60);
    r.max_attempts = 0;
    assert!(matches!(
        ops.enqueue(&r).await.unwrap_err(),
        OperationsError::InvalidAttempts
    ));
    r.max_attempts = 21;
    assert!(matches!(
        ops.enqueue(&r).await.unwrap_err(),
        OperationsError::InvalidAttempts
    ));
    assert!(matches!(
        ops.enqueue(&ping(Uuid::now_v7(), "ok", 1))
            .await
            .unwrap_err(),
        OperationsError::UnknownNode
    ));
    sqlx::query("UPDATE nodes SET status = 'revoked' WHERE id = $1")
        .bind(n)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        ops.enqueue(&ping(n, "ok", 1)).await.unwrap_err(),
        OperationsError::NodeRevoked
    ));
    assert_eq!(count(&pool).await, 0);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_operation_walks_pending_dispatched_succeeded_and_stays_there(pool: PgPool) {
    let ops = Operations::new(pool.clone());
    let n = node(&pool, "node-a").await;
    let id = ops.enqueue(&ping(n, "k", 1)).await.unwrap().operation().id;

    // Only claimable for the nodes asked about.
    assert!(
        ops.claim_due(&[Uuid::now_v7()], 10)
            .await
            .unwrap()
            .is_empty()
    );
    let claimed = ops.claim_due(&[n], 10).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(
        (claimed[0].status, claimed[0].attempts),
        (Status::Dispatched, 1)
    );
    assert!(claimed[0].dispatched_at.is_some());
    assert!(
        ops.claim_due(&[n], 10).await.unwrap().is_empty(),
        "not twice"
    );

    assert!(
        ops.succeed(id, &json!({"type": "pong", "nonce": 1}))
            .await
            .unwrap()
    );
    let row = ops.get(id).await.unwrap().unwrap();
    assert_eq!(row.status, Status::Succeeded);
    assert_eq!(row.output, Some(json!({"type": "pong", "nonce": 1})));
    assert!(row.finished_at.is_some() && row.error.is_none());

    // Final means final, for the service and for raw SQL.
    assert!(!ops.succeed(id, &json!({})).await.unwrap());
    assert_eq!(
        ops.fail(id, &json!({"code": "internal"}), None)
            .await
            .unwrap(),
        None
    );
    assert!(!ops.release_undelivered(id).await.unwrap());
    for sql in [
        "UPDATE operations SET status = 'pending', finished_at = NULL",
        "UPDATE operations SET status = 'failed'",
        "UPDATE operations SET output = '{}'",
        "DELETE FROM operations",
    ] {
        assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
    }
    assert_eq!(
        ops.get(id).await.unwrap().unwrap().status,
        Status::Succeeded
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_database_refuses_illegal_moves_and_identity_changes(pool: PgPool) {
    let ops = Operations::new(pool.clone());
    let n = node(&pool, "node-a").await;
    let id = ops.enqueue(&ping(n, "k", 1)).await.unwrap().operation().id;
    // pending cannot jump to succeeded or retryable.
    for to in ["succeeded", "retryable"] {
        let r = sqlx::query("UPDATE operations SET status = $1 WHERE id = $2")
            .bind(to)
            .bind(id)
            .execute(&pool)
            .await;
        assert!(r.is_err(), "pending -> {to}");
    }
    // What identifies the operation never changes.
    for sql in [
        "UPDATE operations SET kind = 'report_capabilities'",
        "UPDATE operations SET payload = '{\"type\":\"ping\",\"nonce\":2}'",
        "UPDATE operations SET idempotency_key = 'other'",
        "UPDATE operations SET max_attempts = 9",
        "UPDATE operations SET node_id = gen_random_uuid()",
    ] {
        assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
    }
    assert_eq!(ops.get(id).await.unwrap().unwrap().status, Status::Pending);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn retryable_failures_are_sent_again_until_the_attempts_run_out(pool: PgPool) {
    let ops = Operations::new(pool.clone());
    let n = node(&pool, "node-a").await;
    let mut r = ping(n, "k", 1);
    r.max_attempts = 2;
    let id = ops.enqueue(&r).await.unwrap().operation().id;
    let err = json!({"code": "internal", "message": "disk busy", "retryable": true});

    ops.claim_due(&[n], 10).await.unwrap();
    // Not retryable by the caller's decision: fails for good at once.
    // (Checked on a second operation below.) Retry after a minute: waits.
    assert_eq!(
        ops.fail(id, &err, Some(Duration::from_secs(60)))
            .await
            .unwrap(),
        Some(Status::Retryable)
    );
    assert!(
        ops.claim_due(&[n], 10).await.unwrap().is_empty(),
        "must wait"
    );
    sqlx::query("UPDATE operations SET next_attempt_at = now() WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let again = ops.claim_due(&[n], 10).await.expect("claim again");
    assert_eq!((again.len(), again[0].attempts), (1, 2));
    // Second and last attempt fails: no third.
    assert_eq!(
        ops.fail(id, &err, Some(Duration::ZERO)).await.unwrap(),
        Some(Status::Failed)
    );
    let row = ops.get(id).await.unwrap().unwrap();
    assert_eq!((row.status, row.attempts), (Status::Failed, 2));
    assert!(row.finished_at.is_some());
    assert_eq!(row.error, Some(err.clone()));

    // A failure the caller calls permanent is final immediately.
    let id2 = ops.enqueue(&ping(n, "k2", 2)).await.unwrap().operation().id;
    ops.claim_due(&[n], 10).await.unwrap();
    assert_eq!(
        ops.fail(id2, &err, None).await.unwrap(),
        Some(Status::Failed)
    );

    // A retry that would land after the expiry is not scheduled.
    let mut r = ping(n, "k3", 3);
    r.ttl = Duration::from_secs(30);
    let id3 = ops.enqueue(&r).await.unwrap().operation().id;
    ops.claim_due(&[n], 10).await.unwrap();
    assert_eq!(
        ops.fail(id3, &err, Some(Duration::from_secs(600)))
            .await
            .unwrap(),
        Some(Status::Failed)
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn an_undelivered_operation_goes_back_without_using_an_attempt(pool: PgPool) {
    let ops = Operations::new(pool.clone());
    let n = node(&pool, "node-a").await;
    let id = ops.enqueue(&ping(n, "k", 1)).await.unwrap().operation().id;
    ops.claim_due(&[n], 10).await.unwrap();
    assert!(ops.release_undelivered(id).await.unwrap());
    let row = ops.get(id).await.unwrap().unwrap();
    assert_eq!((row.status, row.attempts), (Status::Pending, 0));
    assert_eq!(ops.claim_due(&[n], 10).await.unwrap().len(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn expired_operations_are_not_sent_and_are_failed(pool: PgPool) {
    let ops = Operations::new(pool.clone());
    let n = node(&pool, "node-a").await;
    let mut r = ping(n, "k", 1);
    r.ttl = Duration::from_secs(1);
    let id = ops.enqueue(&r).await.unwrap().operation().id;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(ops.claim_due(&[n], 10).await.unwrap().is_empty());
    assert_eq!(ops.expire_overdue().await.unwrap(), 1);
    let row = ops.get(id).await.unwrap().unwrap();
    assert_eq!(row.status, Status::Failed);
    assert_eq!(row.error.unwrap()["code"], "expired");
    assert_eq!(ops.expire_overdue().await.unwrap(), 0);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn workers_claiming_at_once_never_share_an_operation(pool: PgPool) {
    let ops = Operations::new(pool.clone());
    let n = node(&pool, "node-a").await;
    for i in 0..40 {
        ops.enqueue(&ping(n, &format!("k{i}"), i)).await.unwrap();
    }
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let ops = ops.clone();
        tasks.push(tokio::spawn(async move {
            let mut mine = Vec::new();
            loop {
                let got = ops.claim_due(&[n], 5).await.unwrap();
                if got.is_empty() {
                    return mine;
                }
                mine.extend(got.into_iter().map(|o| o.id));
            }
        }));
    }
    let mut all = Vec::new();
    for t in tasks {
        all.extend(t.await.unwrap());
    }
    let unique: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!((all.len(), unique.len()), (40, 40));
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_wire_envelope_follows_the_row(pool: PgPool) {
    let ops = Operations::new(pool.clone());
    let n = node(&pool, "node-a").await;
    let mut r = ping(n, "k", 9);
    r.ttl = Duration::from_secs(120);
    let row = ops.enqueue(&r).await.unwrap().operation().clone();
    let env = envelope(&row, row.created_at + 5).unwrap();
    assert_eq!(env.op_id, row.id);
    assert_eq!(env.idempotency_key, "k");
    assert_eq!(env.op, Operation::Ping { nonce: 9 });
    assert_eq!(
        (env.issued_at, env.expires_at),
        (row.created_at + 5, row.expires_at)
    );
    env.check_fresh(row.created_at + 10).unwrap();
    assert!(env.check_fresh(row.expires_at + 1).is_err());
}
