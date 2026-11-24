//! `account_mfa` / `mfa_backup_codes` behaviour against real PostgreSQL.
//! Ignored by default; run with DATABASE_URL (superuser) and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use builtpanel_database::mfa;
use sqlx::{PgPool, types::Uuid};

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

async fn account(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', $1, $1 || '@example.com', $2) RETURNING id",
    )
    .bind(name)
    .bind(HASH)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn blob(byte: u8) -> Vec<u8> {
    vec![byte; 61]
}

fn hashes(n: u8) -> Vec<[u8; 32]> {
    (0..n).map(|i| [i; 32]).collect()
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn enrolment_is_pending_until_confirmed_and_can_be_restarted(pool: PgPool) {
    let acc = account(&pool, "mfa-a").await;
    assert!(mfa::find_totp(&pool, acc).await.unwrap().is_none());
    assert!(!mfa::totp_enabled(&pool, acc).await.unwrap());

    assert!(
        mfa::begin_totp_enrollment(&pool, acc, &blob(1))
            .await
            .unwrap()
    );
    assert!(
        !mfa::totp_enabled(&pool, acc).await.unwrap(),
        "pending is not enabled"
    );
    // Starting again replaces the pending secret.
    assert!(
        mfa::begin_totp_enrollment(&pool, acc, &blob(2))
            .await
            .unwrap()
    );
    let f = mfa::find_totp(&pool, acc).await.unwrap().unwrap();
    assert_eq!(
        (f.secret_enc, f.confirmed, f.last_used_step),
        (blob(2), false, None)
    );

    assert!(
        mfa::confirm_totp(&pool, acc, 100, &hashes(10))
            .await
            .unwrap()
    );
    assert!(mfa::totp_enabled(&pool, acc).await.unwrap());
    assert_eq!(mfa::unused_backup_codes(&pool, acc).await.unwrap(), 10);

    // A confirmed factor cannot be overwritten or confirmed twice.
    assert!(
        !mfa::begin_totp_enrollment(&pool, acc, &blob(3))
            .await
            .unwrap()
    );
    assert!(
        !mfa::confirm_totp(&pool, acc, 101, &hashes(3))
            .await
            .unwrap()
    );
    let f = mfa::find_totp(&pool, acc).await.unwrap().unwrap();
    assert_eq!((f.secret_enc, f.last_used_step), (blob(2), Some(100)));
    assert_eq!(mfa::unused_backup_codes(&pool, acc).await.unwrap(), 10);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn totp_steps_are_accepted_once_and_only_forward(pool: PgPool) {
    let acc = account(&pool, "mfa-b").await;
    mfa::begin_totp_enrollment(&pool, acc, &blob(1))
        .await
        .unwrap();
    // Not confirmed yet: nothing is accepted.
    assert!(!mfa::accept_totp_step(&pool, acc, 50).await.unwrap());
    mfa::confirm_totp(&pool, acc, 100, &hashes(1))
        .await
        .unwrap();

    assert!(
        !mfa::accept_totp_step(&pool, acc, 100).await.unwrap(),
        "confirming step is spent"
    );
    assert!(
        !mfa::accept_totp_step(&pool, acc, 99).await.unwrap(),
        "older step"
    );
    assert!(mfa::accept_totp_step(&pool, acc, 101).await.unwrap());
    assert!(
        !mfa::accept_totp_step(&pool, acc, 101).await.unwrap(),
        "replay"
    );

    // Twenty concurrent uses of the same new step: exactly one wins.
    let tasks: Vec<_> = (0..20)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move { mfa::accept_totp_step(&pool, acc, 102).await.unwrap() })
        })
        .collect();
    let mut wins = 0;
    for t in tasks {
        wins += u32::from(t.await.unwrap());
    }
    assert_eq!(wins, 1);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn backup_codes_are_single_use_and_per_account(pool: PgPool) {
    let a = account(&pool, "mfa-c").await;
    let b = account(&pool, "mfa-d").await;
    for acc in [a, b] {
        mfa::begin_totp_enrollment(&pool, acc, &blob(1))
            .await
            .unwrap();
    }
    mfa::confirm_totp(&pool, a, 1, &hashes(3)).await.unwrap();
    mfa::confirm_totp(&pool, b, 1, &[[9; 32]]).await.unwrap();

    assert!(mfa::consume_backup_code(&pool, a, &[1; 32]).await.unwrap());
    assert!(
        !mfa::consume_backup_code(&pool, a, &[1; 32]).await.unwrap(),
        "already used"
    );
    assert!(
        !mfa::consume_backup_code(&pool, a, &[9; 32]).await.unwrap(),
        "other account's code"
    );
    assert!(
        !mfa::consume_backup_code(&pool, a, &[77; 32]).await.unwrap(),
        "unknown"
    );
    assert_eq!(mfa::unused_backup_codes(&pool, a).await.unwrap(), 2);
    assert_eq!(mfa::unused_backup_codes(&pool, b).await.unwrap(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn removing_totp_or_the_account_deletes_factor_and_codes(pool: PgPool) {
    let a = account(&pool, "mfa-e").await;
    assert!(
        !mfa::remove_totp(&pool, a).await.unwrap(),
        "nothing to remove"
    );
    mfa::begin_totp_enrollment(&pool, a, &blob(1))
        .await
        .unwrap();
    mfa::confirm_totp(&pool, a, 1, &hashes(10)).await.unwrap();
    assert!(mfa::remove_totp(&pool, a).await.unwrap());
    assert!(!mfa::totp_enabled(&pool, a).await.unwrap());
    assert_eq!(mfa::unused_backup_codes(&pool, a).await.unwrap(), 0);

    let b = account(&pool, "mfa-f").await;
    mfa::begin_totp_enrollment(&pool, b, &blob(1))
        .await
        .unwrap();
    mfa::confirm_totp(&pool, b, 1, &hashes(2)).await.unwrap();
    sqlx::query("DELETE FROM accounts WHERE id = $1")
        .bind(b)
        .execute(&pool)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM account_mfa) + (SELECT count(*) FROM mfa_backup_codes)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test(migrations = "../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn constraints_reject_malformed_rows(pool: PgPool) {
    let a = account(&pool, "mfa-g").await;
    // Too short to be nonce + tag + secret.
    assert!(
        mfa::begin_totp_enrollment(&pool, a, &[1; 10])
            .await
            .is_err()
    );
    let bad_hash =
        sqlx::query("INSERT INTO mfa_backup_codes (account_id, code_sha256) VALUES ($1, $2)")
            .bind(a)
            .bind(vec![1u8; 31])
            .execute(&pool)
            .await;
    assert!(bad_hash.is_err());
    let bad_kind = sqlx::query(
        "INSERT INTO account_mfa (account_id, kind, secret_enc) VALUES ($1, 'sms', $2)",
    )
    .bind(a)
    .bind(blob(1))
    .execute(&pool)
    .await;
    assert!(bad_kind.is_err());
}
