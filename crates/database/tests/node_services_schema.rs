//! What a node's service reading means (T10.13): a service that stops is news once, and so is
//! its recovery. Ignored by default; see tests/migrations.rs.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use builtpanel_database::node_services::{self, Change};
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL; see tests/migrations.rs"]
async fn a_service_that_stops_is_announced_once_and_so_is_its_recovery(pool: PgPool) {
    let node: sqlx::types::Uuid =
        sqlx::query_scalar("INSERT INTO nodes (name) VALUES ('n1') RETURNING id")
            .fetch_one(&pool)
            .await
            .unwrap();
    let see = |name: &'static str, state: &'static str| {
        let pool = pool.clone();
        async move {
            let mut c = pool.acquire().await.unwrap();
            node_services::record(&mut c, node, name, state, "x")
                .await
                .unwrap()
        }
    };
    // Running, running: nothing to say. Stops: said once. Still stopped, then unreachable
    // (still down): nothing more. Runs again: said once. Not installed is not a failure.
    assert_eq!(see("database", "running").await, Change::None);
    assert_eq!(see("database", "running").await, Change::None);
    assert_eq!(see("database", "stopped").await, Change::Down);
    assert_eq!(see("database", "stopped").await, Change::None);
    assert_eq!(see("database", "unreachable").await, Change::None);
    assert_eq!(see("database", "running").await, Change::Recovered);
    assert_eq!(see("database", "running").await, Change::None);
    assert_eq!(see("sftp", "not_installed").await, Change::None);
    // The first reading of a service that is already down is news.
    assert_eq!(see("caddy", "unreachable").await, Change::Down);
    // Down, then not installed (taken away on purpose): no recovery announced, and no repeat.
    assert_eq!(see("caddy", "not_installed").await, Change::None);
    assert_eq!(see("caddy", "stopped").await, Change::Down);

    let listed = node_services::list(&pool, node).await.unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|s| (s.name.as_str(), s.state.as_str()))
            .collect::<Vec<_>>(),
        [
            ("caddy", "stopped"),
            ("database", "running"),
            ("sftp", "not_installed")
        ]
    );
    // `since` moves only when the state changes.
    let (since_before,): (i64,) = sqlx::query_as(
        "SELECT extract(epoch FROM since)::bigint FROM node_services WHERE name = 'database'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE node_services SET since = since - interval '1 hour' WHERE name = 'database'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let mut c = pool.acquire().await.unwrap();
    node_services::record(&mut c, node, "database", "running", "still")
        .await
        .unwrap();
    let (since_after,): (i64,) = sqlx::query_as(
        "SELECT extract(epoch FROM since)::bigint FROM node_services WHERE name = 'database'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(since_after, since_before - 3600);
}
