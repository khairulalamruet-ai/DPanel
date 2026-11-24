//! Notifications against real PostgreSQL and Redis (T18.03, T18.04). Ignored by default; run with
//! DATABASE_URL, REDIS_URL and `--include-ignored`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, OnceLock};

use builtpanel_auth_core::secretbox::MasterKey;
use builtpanel_cache::RedisHandle;
use builtpanel_database::notifications as store;
use builtpanel_kernel::Event;
use builtpanel_mod_notifications::{NotificationsState, deliver_due, handle_event};
use builtpanel_session::{SessionConfig, SessionStore};
use secrecy::SecretString;
use sqlx::PgPool;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
    net::TcpListener,
};
use uuid::Uuid;

const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaA";

fn state(pool: &PgPool) -> NotificationsState {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL must be set");
    let redis = RedisHandle::new(&SecretString::from(url)).expect("valid REDIS_URL");
    NotificationsState {
        db: pool.clone(),
        sessions: SessionStore::new(redis, pool.clone(), SessionConfig::default()),
        settings: Arc::new(OnceLock::new()),
        master_key: MasterKey::from_bytes(&[9u8; 32]),
    }
}

async fn account(pool: &PgPool, name: &str, kind: &str, locale: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO accounts (kind, username, email, password_hash, locale)
         VALUES ($1::account_kind, $2, $2 || '@example.com', $3, $4) RETURNING id",
    )
    .bind(kind)
    .bind(name)
    .bind(HASH)
    .bind(locale)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A hosting account for `owner` on a node; returns (tenant, node).
async fn tenant(pool: &PgPool, owner: Uuid, name: &str, uid: i32) -> (Uuid, Uuid) {
    let node: Uuid = sqlx::query_scalar(
        "INSERT INTO nodes (name, status) VALUES ('node1', 'online')
         ON CONFLICT (name) DO UPDATE SET name = 'node1' RETURNING id",
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
    .bind(owner)
    .bind(node)
    .bind(plan)
    .bind(name)
    .bind(uid)
    .fetch_one(pool)
    .await
    .unwrap();
    (tenant, node)
}

async fn inbox(pool: &PgPool, account: Uuid) -> Vec<store::Notification> {
    store::list(pool, account, false, 50).await.unwrap()
}

/// A mail server that takes whatever it is given: returns its port and what it received.
async fn fake_smtp() -> (u16, Arc<tokio::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let got: Arc<tokio::sync::Mutex<Vec<String>>> = Arc::default();
    let sink = got.clone();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            let sink = sink.clone();
            tokio::spawn(async move {
                let (read, mut write) = socket.into_split();
                let mut lines = BufReader::new(read).lines();
                let _ = write.write_all(b"220 fake ESMTP\r\n").await;
                let (mut data, mut message) = (false, String::new());
                while let Ok(Some(line)) = lines.next_line().await {
                    if data {
                        if line == "." {
                            data = false;
                            sink.lock().await.push(std::mem::take(&mut message));
                            let _ = write.write_all(b"250 queued\r\n").await;
                        } else {
                            message.push_str(&line);
                            message.push('\n');
                        }
                        continue;
                    }
                    let upper = line.to_ascii_uppercase();
                    let answer: &[u8] = if upper.starts_with("EHLO") {
                        b"250-fake\r\n250 8BITMIME\r\n"
                    } else if upper.starts_with("DATA") {
                        data = true;
                        b"354 go on\r\n"
                    } else if upper.starts_with("QUIT") {
                        let _ = write.write_all(b"221 bye\r\n").await;
                        return;
                    } else {
                        b"250 ok\r\n"
                    };
                    let _ = write.write_all(answer).await;
                }
            });
        }
    });
    (port, got)
}

async fn set_smtp(state: &NotificationsState, port: u16) {
    store::set_smtp(
        &state.db,
        &store::Smtp {
            host: "127.0.0.1".into(),
            port: i32::from(port),
            security: "none".into(),
            username: None,
            sealed_password: None,
            from_address: "panel@example.com".into(),
            from_name: Some("Acme Host".into()),
            enabled: true,
        },
    )
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL and Redis"]
async fn a_certificate_a_failed_backup_and_an_offline_node_each_make_a_notification(pool: PgPool) {
    let st = state(&pool);
    let admin = account(&pool, "root1", "admin", "en").await;
    let owner = account(&pool, "alice", "client", "bn").await;
    let other = account(&pool, "bob", "client", "en").await;
    let (tenant_id, node_id) = tenant(&pool, owner, "alice", 10_001).await;

    let certificate = Event::CertificateExpiring {
        tenant_id,
        domain_id: Uuid::new_v4(),
        name: "alice.example".into(),
        not_after: 4_000_000_000,
    };
    let failed_backup = Event::BackupFailed {
        backup_id: Uuid::new_v4(),
        tenant_id: Some(tenant_id),
        error: "the disk is full".into(),
    };
    let offline = Event::NodeOffline {
        node_id,
        name: "node1".into(),
    };
    for event in [&certificate, &failed_backup, &offline] {
        handle_event(&st, event).await.unwrap();
        // Events are delivered at least once: the second time changes nothing.
        handle_event(&st, event).await.unwrap();
    }

    // The account's owner hears about the certificate and the backup, in Bangla; nobody else does.
    let mine = inbox(&pool, owner).await;
    assert_eq!(mine.len(), 2, "{mine:?}");
    assert!(mine.iter().all(|n| n.subject.starts_with('[')), "{mine:?}");
    assert!(
        mine.iter()
            .any(|n| n.event_type == "certificate_expiring" && n.subject.contains("সার্টিফিকেট")),
        "{mine:?}"
    );
    assert!(
        mine.iter()
            .any(|n| n.event_type == "backup_failed" && n.severity == "critical"),
        "{mine:?}"
    );
    assert!(inbox(&pool, other).await.is_empty());
    // The administrator hears about the node (in English), not about somebody's backup.
    let theirs = inbox(&pool, admin).await;
    assert_eq!(theirs.len(), 1, "{theirs:?}");
    assert_eq!(theirs[0].event_type, "node_offline");
    assert!(theirs[0].subject.contains("node1") && theirs[0].subject.contains("offline"));

    // Reading.
    assert_eq!(store::unread_count(&pool, owner).await.unwrap(), 2);
    assert_eq!(
        store::mark_read(&pool, owner, Some(&[mine[0].id]))
            .await
            .unwrap(),
        1
    );
    assert_eq!(store::unread_count(&pool, owner).await.unwrap(), 1);
    assert_eq!(store::mark_read(&pool, owner, None).await.unwrap(), 1);
    assert_eq!(store::unread_count(&pool, owner).await.unwrap(), 0);
    // And someone else's notification cannot be marked.
    assert_eq!(
        store::mark_read(&pool, other, Some(&[theirs[0].id]))
            .await
            .unwrap(),
        0
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL and Redis"]
async fn the_database_announces_a_failed_backup_and_an_offline_node_itself(pool: PgPool) {
    let owner = account(&pool, "carol", "client", "en").await;
    let (tenant_id, node_id) = tenant(&pool, owner, "carol", 10_002).await;
    let destination: Uuid = sqlx::query_scalar(
        "INSERT INTO backup_destinations (name, kind, settings, is_default)
         VALUES ('Disk', 'local', '{\"path\": \"/var/backups\"}', true) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let backup: Uuid = sqlx::query_scalar(
        "INSERT INTO backups (tenant_id, account_name, kind, trigger, destination_id, status)
         VALUES ($1, 'carol', 'tenant', 'manual', $2, 'running') RETURNING id",
    )
    .bind(tenant_id)
    .bind(destination)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE backups SET status = 'failed', error = 'the destination refused' WHERE id = $1",
    )
    .bind(backup)
    .execute(&pool)
    .await
    .unwrap();
    // Failing again (a retry that fails once more) does not announce twice.
    sqlx::query("UPDATE backups SET error = 'still refused' WHERE id = $1")
        .bind(backup)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE nodes SET status = 'offline' WHERE id = $1")
        .bind(node_id)
        .execute(&pool)
        .await
        .unwrap();
    let rows: Vec<(String, serde_json::Value)> =
        sqlx::query_as("SELECT event_type, payload FROM outbox ORDER BY created_at, id")
            .fetch_all(&pool)
            .await
            .unwrap();
    let types: Vec<&str> = rows.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(types, ["backup_failed", "node_offline"], "{rows:?}");
    // What the triggers write is what the program reads back.
    let event: Event = serde_json::from_value(rows[0].1.clone()).unwrap();
    assert_eq!(
        event,
        Event::BackupFailed {
            backup_id: backup,
            tenant_id: Some(tenant_id),
            error: "the destination refused".into()
        }
    );
    let event: Event = serde_json::from_value(rows[1].1.clone()).unwrap();
    assert_eq!(
        event,
        Event::NodeOffline {
            node_id,
            name: "node1".into()
        }
    );
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL and Redis"]
async fn a_channel_gets_its_events_by_mail_and_a_pause_or_snooze_stops_it(pool: PgPool) {
    let st = state(&pool);
    let (port, mails) = fake_smtp().await;
    set_smtp(&st, port).await;
    let owner = account(&pool, "dave", "client", "en").await;
    let (tenant_id, _) = tenant(&pool, owner, "dave", 10_003).await;
    let channel = store::add_channel(&pool, owner, "email", "me", "dave@example.org", None)
        .await
        .unwrap();
    assert!(
        store::update_channel(
            &pool,
            owner,
            channel,
            None,
            Some(&["backup_failed".to_owned()])
        )
        .await
        .unwrap()
    );
    let backup_failed = |why: &str| Event::BackupFailed {
        backup_id: Uuid::new_v4(),
        tenant_id: Some(tenant_id),
        error: why.into(),
    };

    // A certificate is not on the channel's list: inbox only. A failed backup goes out as a mail.
    handle_event(
        &st,
        &Event::CertificateExpiring {
            tenant_id,
            domain_id: Uuid::new_v4(),
            name: "dave.example".into(),
            not_after: 4_000_000_000,
        },
    )
    .await
    .unwrap();
    handle_event(&st, &backup_failed("no space")).await.unwrap();
    assert_eq!(inbox(&pool, owner).await.len(), 2);
    assert_eq!(deliver_due(&st).await, 1);
    let sent = mails.lock().await.clone();
    assert_eq!(sent.len(), 1, "{sent:?}");
    let mail = sent[0].to_ascii_lowercase();
    assert!(mail.contains("to: dave@example.org"), "{}", sent[0]);
    assert!(
        mail.contains("a backup failed") && mail.contains("no space"),
        "{}",
        sent[0]
    );
    assert!(
        sent[0].contains("Built Panel"),
        "the panel's name is in the mail: {}",
        sent[0]
    );
    let status: String = sqlx::query_scalar("SELECT status FROM notification_deliveries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "sent");
    assert_eq!(deliver_due(&st).await, 0, "nothing is sent twice");

    // Paused: the inbox still gets it, the channel does not.
    store::set_quiet(
        &pool,
        owner,
        store::Quiet {
            paused: true,
            snoozed_until: None,
        },
    )
    .await
    .unwrap();
    handle_event(&st, &backup_failed("paused")).await.unwrap();
    assert_eq!(inbox(&pool, owner).await.len(), 3);
    assert_eq!(deliver_due(&st).await, 0);
    // Snoozed: the same, until the time is up.
    let later = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
        + 3600;
    store::set_quiet(
        &pool,
        owner,
        store::Quiet {
            paused: false,
            snoozed_until: Some(later),
        },
    )
    .await
    .unwrap();
    handle_event(&st, &backup_failed("snoozed")).await.unwrap();
    assert_eq!(deliver_due(&st).await, 0);
    // Back on: the next one goes out.
    store::set_quiet(&pool, owner, store::Quiet::default())
        .await
        .unwrap();
    handle_event(&st, &backup_failed("back on")).await.unwrap();
    assert_eq!(deliver_due(&st).await, 1);
    assert_eq!(mails.lock().await.len(), 2);
}

#[sqlx::test(migrations = "../../../migrations")]
#[ignore = "needs PostgreSQL and Redis"]
async fn a_mail_that_cannot_be_sent_is_tried_again_and_then_given_up(pool: PgPool) {
    let st = state(&pool);
    // A port nobody listens on.
    let free = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = free.local_addr().unwrap().port();
    drop(free);
    set_smtp(&st, dead).await;
    let owner = account(&pool, "erin", "client", "en").await;
    let (tenant_id, _) = tenant(&pool, owner, "erin", 10_004).await;
    let channel = store::add_channel(&pool, owner, "email", "me", "erin@example.org", None)
        .await
        .unwrap();
    store::update_channel(
        &pool,
        owner,
        channel,
        None,
        Some(&["backup_failed".to_owned()]),
    )
    .await
    .unwrap();
    handle_event(
        &st,
        &Event::BackupFailed {
            backup_id: Uuid::new_v4(),
            tenant_id: Some(tenant_id),
            error: "x".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(deliver_due(&st).await, 1);
    let (status, attempts, error): (String, i32, Option<String>) =
        sqlx::query_as("SELECT status, attempts, last_error FROM notification_deliveries")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((status.as_str(), attempts), ("pending", 1));
    assert!(error.is_some(), "the reason is kept");
    // Not tried again at once (it waits a minute), and given up after the last try.
    assert_eq!(deliver_due(&st).await, 0);
    for _ in 1..builtpanel_mod_notifications::MAX_ATTEMPTS {
        sqlx::query("UPDATE notification_deliveries SET next_attempt_at = now()")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(deliver_due(&st).await, 1);
    }
    let (status, attempts): (String, i32) =
        sqlx::query_as("SELECT status, attempts FROM notification_deliveries")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), attempts),
        ("failed", builtpanel_mod_notifications::MAX_ATTEMPTS)
    );
}
