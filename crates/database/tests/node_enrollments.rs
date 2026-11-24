//! Enrollment token semantics against real PostgreSQL (T02.05).
//! Ignored by default; see tests/migrations.rs for how to run.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use builtpanel_database::node_enrollments::{
    ConsumedToken, RevokeOutcome, TokenStatus, consume_token, insert_token, list_tokens,
    name_in_use, revoke_token,
};
use sqlx::{PgPool, types::Uuid};

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

fn token(n: u8) -> [u8; 32] {
    [n; 32]
}

async fn admin(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('admin', 'rooty', 'rooty@example.com', $1) RETURNING id",
    )
    .bind(HASH)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Moves a token's life into the past without breaking `expires_at > created_at`.
async fn age(pool: &PgPool, id: Uuid, ago_secs: i32) {
    sqlx::query(
        "UPDATE node_enrollment_tokens
         SET created_at = now() - make_interval(secs => $2 + 600), expires_at = now() - make_interval(secs => $2)
         WHERE id = $1",
    )
    .bind(id)
    .bind(ago_secs)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL: set DATABASE_URL and pass --include-ignored"]
async fn a_token_works_once_and_only_once(pool: PgPool) {
    let by = admin(&pool).await;
    let mut conn = pool.acquire().await.unwrap();
    let created = insert_token(&mut conn, "node-1", &token(1), by, 900)
        .await
        .unwrap();
    assert_eq!(created.status, TokenStatus::Pending);
    assert_eq!(created.created_by, Some(by));
    assert!((created.expires_at - created.created_at - 900).abs() <= 1);

    let first = consume_token(&mut conn, &token(1)).await.unwrap();
    assert_eq!(
        first,
        Some(ConsumedToken {
            id: created.id,
            name: "node-1".into()
        })
    );
    // The second use, and every later one, finds nothing.
    for _ in 0..3 {
        assert_eq!(consume_token(&mut conn, &token(1)).await.unwrap(), None);
    }
    let listed = list_tokens(&pool, 10).await.unwrap();
    assert_eq!(listed[0].status, TokenStatus::Consumed);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL: set DATABASE_URL and pass --include-ignored"]
async fn racing_agents_cannot_both_use_the_same_token(pool: PgPool) {
    let by = admin(&pool).await;
    insert_token(
        &mut pool.acquire().await.unwrap(),
        "node-1",
        &token(2),
        by,
        900,
    )
    .await
    .unwrap();
    let tasks: Vec<_> = (0..25)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move {
                let mut conn = pool.acquire().await.unwrap();
                consume_token(&mut conn, &token(2)).await.unwrap().is_some()
            })
        })
        .collect();
    let mut winners = 0;
    for t in tasks {
        winners += u32::from(t.await.unwrap());
    }
    assert_eq!(winners, 1, "exactly one of 25 concurrent uses succeeds");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL: set DATABASE_URL and pass --include-ignored"]
async fn unknown_expired_and_revoked_tokens_are_all_just_none(pool: PgPool) {
    let by = admin(&pool).await;
    let mut conn = pool.acquire().await.unwrap();
    assert_eq!(
        consume_token(&mut conn, &token(9)).await.unwrap(),
        None,
        "unknown"
    );

    let expired = insert_token(&mut conn, "node-old", &token(3), by, 900)
        .await
        .unwrap();
    age(&pool, expired.id, 5).await;
    assert_eq!(
        consume_token(&mut conn, &token(3)).await.unwrap(),
        None,
        "expired"
    );

    let revoked = insert_token(&mut conn, "node-gone", &token(4), by, 900)
        .await
        .unwrap();
    assert_eq!(
        revoke_token(&mut conn, revoked.id).await.unwrap(),
        RevokeOutcome::Revoked
    );
    assert_eq!(
        consume_token(&mut conn, &token(4)).await.unwrap(),
        None,
        "revoked"
    );

    // A token that is merely one second from its end still works.
    let fresh = insert_token(&mut conn, "node-new", &token(5), by, 2)
        .await
        .unwrap();
    assert_eq!(
        consume_token(&mut conn, &token(5))
            .await
            .unwrap()
            .map(|c| c.id),
        Some(fresh.id)
    );

    let statuses: Vec<(String, TokenStatus)> = list_tokens(&pool, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|t| (t.name, t.status))
        .collect();
    assert_eq!(
        statuses,
        [
            ("node-new".to_owned(), TokenStatus::Consumed),
            ("node-gone".to_owned(), TokenStatus::Revoked),
            ("node-old".to_owned(), TokenStatus::Expired),
        ],
        "newest first"
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL: set DATABASE_URL and pass --include-ignored"]
async fn revoking_only_touches_pending_tokens(pool: PgPool) {
    let by = admin(&pool).await;
    let mut conn = pool.acquire().await.unwrap();
    let pending = insert_token(&mut conn, "node-a", &token(1), by, 900)
        .await
        .unwrap();
    let used = insert_token(&mut conn, "node-b", &token(2), by, 900)
        .await
        .unwrap();
    let old = insert_token(&mut conn, "node-c", &token(3), by, 900)
        .await
        .unwrap();
    consume_token(&mut conn, &token(2)).await.unwrap().unwrap();
    age(&pool, old.id, 5).await;

    assert_eq!(
        revoke_token(&mut conn, pending.id).await.unwrap(),
        RevokeOutcome::Revoked
    );
    assert_eq!(
        revoke_token(&mut conn, pending.id).await.unwrap(),
        RevokeOutcome::NotPending
    );
    assert_eq!(
        revoke_token(&mut conn, used.id).await.unwrap(),
        RevokeOutcome::NotPending
    );
    assert_eq!(
        revoke_token(&mut conn, old.id).await.unwrap(),
        RevokeOutcome::NotPending
    );
    assert_eq!(
        revoke_token(&mut conn, Uuid::now_v7()).await.unwrap(),
        RevokeOutcome::Unknown
    );
    // The used token stays "consumed": revoking did not rewrite its history.
    let status: String = sqlx::query_scalar(
        "SELECT CASE WHEN consumed_at IS NOT NULL THEN 'consumed' ELSE 'other' END FROM node_enrollment_tokens WHERE id = $1",
    )
    .bind(used.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "consumed");
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL: set DATABASE_URL and pass --include-ignored"]
async fn a_name_is_taken_by_nodes_and_by_valid_tokens_only(pool: PgPool) {
    let by = admin(&pool).await;
    let mut conn = pool.acquire().await.unwrap();
    assert!(!name_in_use(&mut conn, "node-1").await.unwrap());

    let t = insert_token(&mut conn, "node-1", &token(1), by, 900)
        .await
        .unwrap();
    assert!(
        name_in_use(&mut conn, "node-1").await.unwrap(),
        "pending token"
    );
    assert!(!name_in_use(&mut conn, "node-2").await.unwrap());

    revoke_token(&mut conn, t.id).await.unwrap();
    assert!(
        !name_in_use(&mut conn, "node-1").await.unwrap(),
        "revoked token frees the name"
    );

    let t2 = insert_token(&mut conn, "node-1", &token(2), by, 900)
        .await
        .unwrap();
    age(&pool, t2.id, 5).await;
    assert!(
        !name_in_use(&mut conn, "node-1").await.unwrap(),
        "expired token frees the name"
    );

    sqlx::query("INSERT INTO nodes (name) VALUES ('node-3')")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        name_in_use(&mut conn, "node-3").await.unwrap(),
        "an enrolled node"
    );
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL: set DATABASE_URL and pass --include-ignored"]
async fn the_table_itself_refuses_nonsense(pool: PgPool) {
    let by = admin(&pool).await;
    let mut conn = pool.acquire().await.unwrap();
    // Two tokens can never share a hash.
    insert_token(&mut conn, "node-a", &token(1), by, 900)
        .await
        .unwrap();
    let err = insert_token(&mut conn, "node-b", &token(1), by, 900)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|e| e.constraint()),
        Some("node_enrollment_tokens_hash_key")
    );
    // Bad names and hash lengths.
    let bad_name = insert_token(&mut conn, "Bad Name", &token(2), by, 900)
        .await
        .unwrap_err();
    assert_eq!(
        bad_name.as_database_error().and_then(|e| e.constraint()),
        Some("node_enrollment_tokens_name_format")
    );
    let bad_hash = sqlx::query("INSERT INTO node_enrollment_tokens (name, token_sha256, expires_at) VALUES ('x1', '\\x0102', now() + interval '1 hour')")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        bad_hash.as_database_error().and_then(|e| e.constraint()),
        Some("node_enrollment_tokens_hash_len")
    );
    // A token cannot be both used and revoked, nor expire before it exists.
    let id = insert_token(&mut conn, "node-c", &token(3), by, 900)
        .await
        .unwrap()
        .id;
    let both = sqlx::query(
        "UPDATE node_enrollment_tokens SET consumed_at = now(), revoked_at = now() WHERE id = $1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(
        both.as_database_error().and_then(|e| e.constraint()),
        Some("node_enrollment_tokens_one_ending")
    );
    let backwards =
        sqlx::query("UPDATE node_enrollment_tokens SET expires_at = created_at WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap_err();
    assert_eq!(
        backwards.as_database_error().and_then(|e| e.constraint()),
        Some("node_enrollment_tokens_expiry")
    );
    // Deleting the creating account keeps the token (history), without the link.
    sqlx::query("DELETE FROM accounts WHERE id = $1")
        .bind(by)
        .execute(&pool)
        .await
        .unwrap();
    let created_by: Option<Uuid> =
        sqlx::query_scalar("SELECT created_by FROM node_enrollment_tokens WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(created_by, None);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL: set DATABASE_URL and pass --include-ignored"]
async fn built_in_roles_get_read_access_to_nodes(pool: PgPool) {
    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT r.name FROM role_permissions rp JOIN roles r ON r.id = rp.role_id
         WHERE rp.permission = 'admin.nodes.read' ORDER BY r.name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(roles, ["admin", "support"]);
    let manage: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM role_permissions WHERE permission = 'admin.nodes.manage'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        manage, 0,
        "managing nodes is only granted through super_admin's `*`"
    );
}
