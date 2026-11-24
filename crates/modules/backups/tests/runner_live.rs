//! The backup runner against real PostgreSQL (T09.07). Ignored by default; run with
//! DATABASE_URL (superuser) and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use builtpanel_auth_core::secretbox::MasterKey;
use builtpanel_mod_backups::runner::{self, tick};
use builtpanel_node_protocol::Operation;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";
const NOW: i64 = 1_710_000_000; // 2024-03-09

async fn tenant(pool: &PgPool, name: &str, uid: i32) -> (Uuid, Uuid) {
    let account: Uuid = sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash)
         VALUES ('client', $1, $1 || '@example.com', $2) RETURNING id",
    )
    .bind(name)
    .bind(HASH)
    .fetch_one(pool)
    .await
    .unwrap();
    let node: Uuid = sqlx::query_scalar(
        "INSERT INTO nodes (name) VALUES ('node1') ON CONFLICT (name) DO UPDATE SET name = 'node1' RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let plan: Uuid = sqlx::query_scalar("SELECT id FROM plans WHERE is_default")
        .fetch_one(pool)
        .await
        .unwrap();
    let tenant: Uuid = sqlx::query_scalar(
        "INSERT INTO tenants (owner_account_id, node_id, plan_id, username, uid, status)
         VALUES ($1, $2, $3, $4, $5, 'active') RETURNING id",
    )
    .bind(account)
    .bind(node)
    .bind(plan)
    .bind(name)
    .bind(uid)
    .fetch_one(pool)
    .await
    .unwrap();
    (tenant, node)
}

async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn schedules_make_backups_and_retention_removes_the_old_ones(pool: PgPool) {
    let key = MasterKey::from_bytes(&[7u8; 32]);
    let (t1, node) = tenant(&pool, "alpha", 10_001).await;
    let (t2, _) = tenant(&pool, "beta", 10_002).await;
    let (t3, _) = tenant(&pool, "gamma", 10_003).await;
    let dest: Uuid = sqlx::query_scalar(
        "INSERT INTO backup_destinations (name, kind, settings, is_default)
         VALUES ('Disk', 'local', '{\"path\": \"/var/backups\"}', true) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for t in [t1, t2, t3] {
        // Due a minute ago; keep the newest of 2 days.
        builtpanel_database::backups::upsert_schedule(
            &pool,
            t,
            "30 2 * * *",
            2,
            0,
            None,
            true,
            Some(NOW - 60),
        )
        .await
        .unwrap();
    }
    // Alpha has a domain with a zone: its records go into the backup (T09.07).
    let domain: Uuid = sqlx::query_scalar(
        "INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, 'alpha.example', 'primary') RETURNING id",
    )
    .bind(t1)
    .fetch_one(&pool)
    .await
    .unwrap();
    let zone: Uuid = sqlx::query_scalar(
        "INSERT INTO dns_zones (domain_id, name) VALUES ($1, 'alpha.example') RETURNING id",
    )
    .bind(domain)
    .fetch_one(&pool)
    .await
    .unwrap();
    for (name, rtype, content, priority) in [
        ("@", "A", "203.0.113.7", None),
        ("@", "MX", "mail.alpha.example", Some(10)),
    ] {
        sqlx::query("INSERT INTO dns_records (zone_id, name, rtype, content, ttl, priority) VALUES ($1, $2, $3, $4, 300, $5)")
            .bind(zone)
            .bind(name)
            .bind(rtype)
            .bind(content)
            .bind(priority)
            .execute(&pool)
            .await
            .unwrap();
    }
    tick(&pool, &key, NOW).await.unwrap();
    let alpha_dns: Value = sqlx::query_scalar(
        "SELECT payload -> 'dns' FROM operations WHERE kind = 'create_backup' AND tenant_id = $1",
    )
    .bind(t1)
    .fetch_optional(&pool)
    .await
    .unwrap()
    .unwrap_or(Value::Null);
    let records = alpha_dns.as_array().cloned().unwrap_or_default();
    assert_eq!(records.len(), 2, "{alpha_dns}");
    assert_eq!(records[0]["zone"], "alpha.example");
    assert_eq!(records[0]["type"], "A");
    assert_eq!(records[1]["priority"], 10);
    // Two at a time on one node: the third waits, still due.
    let queued: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, status FROM backups WHERE trigger = 'schedule' ORDER BY created_at",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(queued.len(), 2, "{queued:?}");
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM backup_schedules WHERE next_run_at = to_timestamp(1709999940)"
        )
        .await,
        1
    );
    // Each has its operation, without a key in the ledger, and a sealed key of its own.
    let (backup, _) = queued[0].clone();
    let op: (String, Value, String) = sqlx::query_as(
        "SELECT kind, payload, idempotency_key FROM operations WHERE kind = 'create_backup' ORDER BY created_at LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(op.0, "create_backup");
    assert!(op.2.starts_with("backup:"), "{}", op.2);
    assert!(
        op.1.get("key").is_none() && op.1.get("credentials").is_none(),
        "{}",
        op.1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(DISTINCT sealed_key) FROM backups WHERE sealed_key IS NOT NULL"
        )
        .await,
        2
    );
    // The same tick again changes nothing.
    tick(&pool, &key, NOW).await.unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM operations WHERE kind = 'create_backup'"
        )
        .await,
        2
    );

    // The key and the credentials are put in only on the way out.
    let payload: Operation = serde_json::from_value(op.1.clone()).unwrap();
    let filled = runner::fill(&pool, &key, payload).await.unwrap();
    let Operation::CreateBackup { key: Some(k), .. } = filled else {
        panic!("no key")
    };
    assert_eq!(k.0.len(), 64);

    // The node starts, then finishes.
    let id: Uuid = sqlx::query_scalar("SELECT id FROM operations WHERE idempotency_key = $1")
        .bind(format!("backup:{backup}"))
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1, dispatched_at = now() WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let s: String = sqlx::query_scalar("SELECT status FROM backups WHERE id = $1")
        .bind(backup)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(s, "running");
    sqlx::query("UPDATE operations SET status = 'succeeded', output = $2, finished_at = now() WHERE id = $1")
        .bind(id)
        .bind(json!({"type": "backup_created", "manifest_sha256": "a".repeat(64), "size_bytes": 1234,
                     "parts": [{"name": "files.tar.zst.enc", "kind": "files", "stored_bytes": 1000, "sha256": "b".repeat(64)}]}))
        .execute(&pool)
        .await
        .unwrap();
    let (status, size, parts): (String, i64, Value) =
        sqlx::query_as("SELECT status, size_bytes, parts FROM backups WHERE id = $1")
            .bind(backup)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), size, parts.as_array().unwrap().len()),
        ("ready", 1234, 1)
    );

    // Retention: five ready scheduled backups of one account on five different days; the
    // newest two days are kept (the one made above is the newest of its day).
    for days_ago in 1..=5 {
        sqlx::query(
            "INSERT INTO backups (tenant_id, account_name, kind, trigger, destination_id, status, created_at)
             SELECT tenant_id, 'alpha', 'tenant', 'schedule', $2, 'ready', now() - make_interval(days => $3)
             FROM backups WHERE id = $1",
        )
        .bind(backup)
        .bind(dest)
        .bind(days_ago)
        .execute(&pool)
        .await
        .unwrap();
    }
    // A manual one is never removed by retention.
    sqlx::query(
        "INSERT INTO backups (tenant_id, account_name, kind, trigger, destination_id, status, created_at)
         SELECT tenant_id, 'alpha', 'tenant', 'manual', $2, 'ready', now() - interval '90 days'
         FROM backups WHERE id = $1",
    )
    .bind(backup)
    .bind(dest)
    .execute(&pool)
    .await
    .unwrap();
    tick(&pool, &key, NOW).await.unwrap();
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT trigger, status FROM backups WHERE tenant_id = (SELECT tenant_id FROM backups WHERE id = $1) ORDER BY created_at DESC",
    )
    .bind(backup)
    .fetch_all(&pool)
    .await
    .unwrap();
    let deleting = rows.iter().filter(|r| r.1 == "deleting").count();
    assert_eq!(deleting, 4, "{rows:?}");
    assert!(rows.iter().any(|r| r.0 == "manual" && r.1 == "ready"));
    // Their removal is queued; the node does it and the rows are marked deleted.
    let removals: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, idempotency_key FROM operations WHERE kind = 'delete_backup'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(removals.len(), 4);
    for (op, _) in &removals[..3] {
        sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1, dispatched_at = now() WHERE id = $1")
            .bind(op)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE operations SET status = 'succeeded', output = '{\"type\": \"backup_deleted\", \"removed\": 5}', finished_at = now() WHERE id = $1")
            .bind(op)
            .execute(&pool)
            .await
            .unwrap();
    }
    // One removal fails: the backup is ready again and the next round tries again, with a new key.
    let (failing, key1) = removals[3].clone();
    sqlx::query("UPDATE operations SET status = 'dispatched', attempts = 1, dispatched_at = now() WHERE id = $1")
        .bind(failing)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE operations SET status = 'failed', error = '{\"message\": \"destination down\"}', finished_at = now() WHERE id = $1")
        .bind(failing)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM backups WHERE status = 'deleted'"
        )
        .await,
        3
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM backups WHERE status = 'deleting'"
        )
        .await,
        0
    );
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    tick(&pool, &key, NOW + 3600).await.unwrap();
    let again: Vec<String> = sqlx::query_scalar(
        "SELECT idempotency_key FROM operations WHERE kind = 'delete_backup' AND status = 'pending'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(again.len(), 1, "{again:?}");
    assert_ne!(again[0], key1);
    let _ = node;
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_panels_own_schedule_queues_system_backups_and_retention_removes_old_ones(
    pool: PgPool,
) {
    let key = MasterKey::from_bytes(&[7u8; 32]);
    let dest: Uuid = sqlx::query_scalar(
        "INSERT INTO backup_destinations (name, kind, settings, is_default)
         VALUES ('Disk', 'local', '{\"path\": \"/var/backups\"}', true) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    builtpanel_database::backups::upsert_system_schedule(
        &pool,
        "0 3 * * *",
        2,
        0,
        None,
        true,
        Some(NOW - 60),
    )
    .await
    .unwrap();
    tick(&pool, &key, NOW).await.unwrap();
    let rows: Vec<(String, String, Option<Uuid>)> =
        sqlx::query_as("SELECT kind, status, tenant_id FROM backups")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows, [("system".to_owned(), "queued".to_owned(), None)]);
    // The executor takes it, and nobody else gets it.
    let claimed = builtpanel_database::backups::claim_system_backups(&pool)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert!(
        builtpanel_database::backups::claim_system_backups(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    // Running: the schedule does not pile up another.
    sqlx::query("UPDATE backup_schedules SET next_run_at = to_timestamp($1)")
        .bind(NOW - 30)
        .execute(&pool)
        .await
        .unwrap();
    tick(&pool, &key, NOW).await.unwrap();
    assert_eq!(
        count(&pool, "SELECT count(*) FROM backups WHERE kind = 'system'").await,
        1
    );
    builtpanel_database::backups::finish_system_backup(
        &pool,
        claimed[0].id,
        10,
        &json!([]),
        &"a".repeat(64),
    )
    .await
    .unwrap();
    // Four ready scheduled ones on older days; two days are kept (the new one and one more).
    for days_ago in 1..=4 {
        sqlx::query(
            "INSERT INTO backups (kind, account_name, trigger, destination_id, status, created_at)
             VALUES ('system', 'panel', 'schedule', $1, 'ready', now() - make_interval(days => $2))",
        )
        .bind(dest)
        .bind(days_ago)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("UPDATE backups SET trigger = 'schedule' WHERE kind = 'system'")
        .execute(&pool)
        .await
        .unwrap();
    tick(&pool, &key, NOW).await.unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM backups WHERE status = 'deleting'"
        )
        .await,
        3
    );
    // No node operation is made for a system backup: the control plane does it itself.
    assert_eq!(count(&pool, "SELECT count(*) FROM operations").await, 0);
    let gone = builtpanel_database::backups::deleting_system(&pool)
        .await
        .unwrap();
    assert_eq!(gone.len(), 3);
    builtpanel_database::backups::finish_system_removal(&pool, gone[0].id, None)
        .await
        .unwrap();
    builtpanel_database::backups::finish_system_removal(
        &pool,
        gone[1].id,
        Some("destination down"),
    )
    .await
    .unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM backups WHERE status = 'deleted'"
        )
        .await,
        1
    );
    // The failed removal is ready again, and retention takes it up once more.
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM backups WHERE status = 'ready' AND error IS NOT NULL"
        )
        .await,
        1
    );
    tick(&pool, &key, NOW).await.unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM backups WHERE status = 'deleting'"
        )
        .await,
        2
    );
}

/// The DNS records of a backup come back through the panel: the zone's present records are
/// replaced by the backup's, an invalid or clashing record and a zone that is gone are skipped,
/// the old records are kept in the summary, and a second round changes nothing (T09.07 follow-up).
#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL"]
async fn the_dns_records_of_a_backup_are_put_back_once_and_the_old_ones_are_kept(pool: PgPool) {
    let key = MasterKey::from_bytes(&[7u8; 32]);
    let (tenant_id, _) = tenant(&pool, "delta", 10_004).await;
    let (other_tenant, _) = tenant(&pool, "epsilon", 10_005).await;
    let destination: Uuid = sqlx::query_scalar(
        "INSERT INTO backup_destinations (name, kind, settings, is_default)
         VALUES ('Disk', 'local', '{\"path\": \"/var/backups\"}', true) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut zones = Vec::new();
    for (owner, name) in [
        (tenant_id, "delta.example"),
        (other_tenant, "epsilon.example"),
    ] {
        let domain: Uuid = sqlx::query_scalar(
            "INSERT INTO domains (tenant_id, fqdn, kind) VALUES ($1, $2, 'primary') RETURNING id",
        )
        .bind(owner)
        .bind(name)
        .fetch_one(&pool)
        .await
        .unwrap();
        let zone: Uuid = sqlx::query_scalar(
            "INSERT INTO dns_zones (domain_id, name) VALUES ($1, $2) RETURNING id",
        )
        .bind(domain)
        .bind(name)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO dns_records (zone_id, name, rtype, content, ttl) VALUES ($1, '@', 'TXT', 'today', 300)")
            .bind(zone)
            .execute(&pool)
            .await
            .unwrap();
        zones.push(zone);
    }
    // Both zones are in sync with the DNS server; a restore must make its zone pending again.
    sqlx::query("UPDATE dns_zones SET status = 'active'")
        .execute(&pool)
        .await
        .unwrap();
    let backup: Uuid = sqlx::query_scalar(
        "INSERT INTO backups (tenant_id, account_name, kind, trigger, destination_id, status)
         VALUES ($1, 'delta', 'tenant', 'manual', $2, 'ready') RETURNING id",
    )
    .bind(tenant_id)
    .bind(destination)
    .fetch_one(&pool)
    .await
    .unwrap();
    let restore: Uuid = sqlx::query_scalar(
        "INSERT INTO backup_restores (backup_id, tenant_id, dns, status)
         VALUES ($1, $2, true, 'running') RETURNING id",
    )
    .bind(backup)
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    // The node's answer: records of this tenant's zone, of a zone that is gone, of a zone that
    // is another account's, and two that cannot be taken (a bad address, a clash).
    let answer = json!({
        "verified_parts": 4, "files_restored": false, "databases": [], "snapshot": [],
        "dns": [
            {"zone": "delta.example", "name": "@", "type": "A", "content": "203.0.113.7", "ttl": 300, "priority": null},
            {"zone": "delta.example", "name": "@", "type": "MX", "content": "mail.delta.example", "ttl": 300, "priority": 10},
            {"zone": "delta.example", "name": "www", "type": "CNAME", "content": "delta.example", "ttl": 300, "priority": null},
            {"zone": "delta.example", "name": "www", "type": "A", "content": "203.0.113.8", "ttl": 300, "priority": null},
            {"zone": "delta.example", "name": "bad", "type": "A", "content": "not-an-address", "ttl": 300, "priority": null},
            {"zone": "gone.example", "name": "@", "type": "A", "content": "203.0.113.9", "ttl": 300, "priority": null},
            {"zone": "epsilon.example", "name": "@", "type": "A", "content": "203.0.113.10", "ttl": 300, "priority": null}
        ],
    });
    // Not applied before the node has answered.
    tick(&pool, &key, NOW).await.unwrap();
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM dns_records WHERE content = 'today'"
        )
        .await,
        2
    );
    sqlx::query("UPDATE backup_restores SET status = 'done', result = $2 WHERE id = $1")
        .bind(restore)
        .bind(&answer)
        .execute(&pool)
        .await
        .unwrap();
    tick(&pool, &key, NOW).await.unwrap();
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT name, rtype, content FROM dns_records WHERE zone_id = $1 ORDER BY name, rtype",
    )
    .bind(zones[0])
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            ("@".into(), "A".into(), "203.0.113.7".into()),
            ("@".into(), "MX".into(), "mail.delta.example.".into()),
            ("www".into(), "CNAME".into(), "delta.example.".into()),
        ],
        "the A next to the CNAME and the bad address are skipped"
    );
    // The other account's zone is not touched, whatever a backup says.
    let other: Vec<String> =
        sqlx::query_scalar("SELECT content FROM dns_records WHERE zone_id = $1")
            .bind(zones[1])
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(other, vec!["today".to_owned()]);
    let (summary, applied, result): (Value, bool, Value) = sqlx::query_as(
        "SELECT dns_summary, dns_applied_at IS NOT NULL, result FROM backup_restores WHERE id = $1",
    )
    .bind(restore)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(applied);
    assert_eq!(summary["zones_restored"], 1, "{summary}");
    // A zone that is gone and one that is another account's look the same: not this account's.
    assert_eq!(
        summary["zones_missing"],
        json!(["epsilon.example", "gone.example"])
    );
    assert_eq!(summary["records_added"], 3);
    assert_eq!(summary["records_removed"], 1);
    assert_eq!(summary["records_skipped"], 2, "{summary}");
    assert_eq!(summary["replaced"][0]["previous"][0]["content"], "today");
    assert!(result.get("dns").is_none(), "the records leave the answer");
    // The zone is marked to be sent again.
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM dns_zones WHERE status = 'pending'"
        )
        .await,
        1
    );
    // A second round changes nothing.
    tick(&pool, &key, NOW + 60).await.unwrap();
    assert_eq!(
        count(&pool, "SELECT count(*) FROM dns_records WHERE zone_id IN (SELECT id FROM dns_zones WHERE name = 'delta.example')").await,
        3
    );
}
