//! Integration tests against a real PostgreSQL 18 server.
//!
//! Ignored by default so `cargo test` works without a database. Run with:
//! `DATABASE_URL=postgres://<superuser>@127.0.0.1:5432/postgres cargo test -p builtpanel-database -- --include-ignored`
//! `#[sqlx::test]` creates (and drops) a fresh database per test, applies all
//! migrations first, so the role in DATABASE_URL needs CREATEDB.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use builtpanel_database::{MIGRATOR, in_transaction, migration_status, ping};
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL: set DATABASE_URL and pass --include-ignored"]
async fn migrations_create_schema_info_row(pool: PgPool) {
    ping(&pool).await.expect("ping");
    let (product, rows): (String, i64) =
        sqlx::query_as("SELECT product, count(*) OVER () FROM schema_info")
            .fetch_one(&pool)
            .await
            .expect("schema_info row");
    assert_eq!(product, "builtpanel");
    assert_eq!(rows, 1);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL: set DATABASE_URL and pass --include-ignored"]
async fn schema_info_is_single_row(pool: PgPool) {
    let err = sqlx::query("INSERT INTO schema_info (id) VALUES (2)")
        .execute(&pool)
        .await
        .expect_err("CHECK (id = 1) must reject a second row");
    assert!(err.to_string().contains("schema_info"), "{err}");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL: set DATABASE_URL and pass --include-ignored"]
async fn uuidv7_is_available(pool: PgPool) {
    let version: i32 = sqlx::query_scalar("SELECT uuid_extract_version(uuidv7())::int4")
        .fetch_one(&pool)
        .await
        .expect("uuidv7");
    assert_eq!(version, 7);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL: set DATABASE_URL and pass --include-ignored"]
async fn status_reports_all_migrations_applied(pool: PgPool) {
    let status = migration_status(&pool).await.expect("status");
    assert!(!status.is_empty());
    assert!(status.iter().all(|m| m.applied), "{status:?}");
}

#[sqlx::test(migrations = false)]
#[ignore = "needs PostgreSQL: set DATABASE_URL and pass --include-ignored"]
async fn migrate_up_down_up_round_trips(pool: PgPool) {
    MIGRATOR.run(&pool).await.expect("up");
    MIGRATOR.undo(&pool, 0).await.expect("down to 0");
    let exists: bool = sqlx::query_scalar("SELECT to_regclass('schema_info') IS NOT NULL")
        .fetch_one(&pool)
        .await
        .expect("query");
    assert!(!exists, "down migration must drop schema_info");
    MIGRATOR.run(&pool).await.expect("up again");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL: set DATABASE_URL and pass --include-ignored"]
async fn transaction_commits_on_ok_and_rolls_back_on_err(pool: PgPool) {
    sqlx::query("CREATE TABLE t (v int)")
        .execute(&pool)
        .await
        .expect("create");

    in_transaction::<_, sqlx::Error, _>(&pool, |conn| {
        Box::pin(async move {
            sqlx::query("INSERT INTO t VALUES (1)")
                .execute(&mut *conn)
                .await?;
            Ok(())
        })
    })
    .await
    .expect("commit");

    let result = in_transaction::<(), sqlx::Error, _>(&pool, |conn| {
        Box::pin(async move {
            sqlx::query("INSERT INTO t VALUES (2)")
                .execute(&mut *conn)
                .await?;
            Err(sqlx::Error::RowNotFound)
        })
    })
    .await;
    assert!(result.is_err());

    let values: Vec<i32> = sqlx::query_scalar("SELECT v FROM t ORDER BY v")
        .fetch_all(&pool)
        .await
        .expect("select");
    assert_eq!(values, vec![1], "second insert must be rolled back");
}
