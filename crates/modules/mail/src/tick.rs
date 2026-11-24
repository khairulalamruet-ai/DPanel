//! The mail tick (T13.02, T13.04, T13.07): once in a while it compares what the panel wants
//! with what each node's mail server has been told, sends the difference as operations, writes
//! the DNS records the mail server asks for into the domains' zones, and holds a domain whose
//! outgoing queue has grown past the limit.
//!
//! Every thing has a version counter that the API raises when it changes; the operation's
//! ledger key carries the version, so the tick can run as often as it likes: asking again
//! for the same version finds the same operation.

use builtpanel_database::{
    dns,
    mail::{self as store, MailDomain, Mailbox},
    notifications::{self, NewNotification},
    operations::{self, NewOperation, Status},
    plans,
    sqlx::PgPool,
    tenants,
};
use builtpanel_domain::{DnsRecord, DnsRecordType, check_dns_conflicts};
use builtpanel_node_protocol::{MailDnsRecord, MailForwarder, MailQueueItem, Operation, Output};
use builtpanel_settings::SettingsRegistry;
use serde_json::json;
use uuid::Uuid;

/// A domain or mailbox that the node refused is tried again after this long.
const RETRY_AFTER_SECS: i32 = 600;
/// Most queued messages removed in one hold.
const MAX_REMOVED: usize = 1000;
const OP_TTL_SECS: i32 = 6 * 3600;

/// What went wrong in a tick (the loop logs it and goes on).
#[derive(Debug, thiserror::Error)]
pub enum TickError {
    /// The database.
    #[error(transparent)]
    Database(#[from] builtpanel_database::sqlx::Error),
    /// The ledger refused an operation.
    #[error("cannot queue an operation: {0}")]
    Queue(String),
    /// The settings could not be read.
    #[error("cannot read the settings: {0}")]
    Settings(String),
}

/// How an operation stands.
enum Step {
    Waiting,
    Done(Option<Output>),
    Failed(String),
}

async fn step(
    db: &PgPool,
    node_id: Uuid,
    tenant_id: Option<Uuid>,
    op: &Operation,
    key: &str,
) -> Result<Step, TickError> {
    let payload = serde_json::to_value(op).map_err(|e| TickError::Queue(e.to_string()))?;
    let queued = operations::enqueue(
        db,
        &NewOperation {
            node_id,
            kind: op.type_name(),
            tenant_id,
            payload: &payload,
            idempotency_key: key,
            created_by: None,
            ttl_secs: OP_TTL_SECS,
            max_attempts: 5,
        },
    )
    .await
    .map_err(|e| TickError::Queue(e.to_string()))?;
    let row = queued.operation();
    Ok(match row.status {
        Status::Succeeded => Step::Done(
            row.output
                .clone()
                .and_then(|v| serde_json::from_value::<Output>(v).ok()),
        ),
        Status::Failed => Step::Failed(
            row.error
                .as_ref()
                .and_then(|e| e.get("message"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("the mail server refused the change")
                .chars()
                .take(300)
                .collect(),
        ),
        _ => Step::Waiting,
    })
}

fn short_hash(text: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// One pass over everything.
///
/// # Errors
/// The database, the ledger or the settings failing.
pub async fn tick(db: &PgPool, settings: &SettingsRegistry) -> Result<(), TickError> {
    store::retry_stale(db, RETRY_AFTER_SECS).await?;
    let rules = crate::rules(settings, db)
        .await
        .map_err(|e| TickError::Settings(e.to_string()))?;
    let domains = store::all_domains(db).await?;
    let mut nodes: Vec<Uuid> = domains.iter().map(|d| d.node_id).collect();
    nodes.sort();
    nodes.dedup();
    let mut ready = Vec::new();
    for node in nodes {
        if server_ready(db, node, &rules.hostname).await? {
            ready.push(node);
        }
    }
    for domain in domains.iter().filter(|d| ready.contains(&d.node_id)) {
        if let Err(error) = domain_round(db, domain).await {
            tracing::warn!(domain = %domain.fqdn, %error, "a mail domain could not finish a round");
        }
    }
    hold_abusers(db, rules.abuse_queue_threshold).await?;
    Ok(())
}

/// Makes sure the node's mail server has the name it should. True when it has.
async fn server_ready(db: &PgPool, node: Uuid, setting: &str) -> Result<bool, TickError> {
    let row = store::server(db, node).await?;
    let hostname = match (&row, setting.is_empty()) {
        (Some(r), _) => r.hostname.clone(),
        (None, false) => {
            store::want_hostname(db, node, setting).await?;
            setting.to_owned()
        }
        (None, true) => return Ok(false),
    };
    if row.as_ref().and_then(|r| r.applied_hostname.as_deref()) == Some(hostname.as_str()) {
        return Ok(true);
    }
    let op = Operation::EnsureMailServer {
        hostname: hostname.clone(),
        certificate_pem: None,
        private_key_pem: None,
    };
    match step(
        db,
        node,
        None,
        &op,
        &format!("mailserver:{node}:{:x}", short_hash(&hostname)),
    )
    .await?
    {
        Step::Done(_) => {
            store::server_applied(db, node, &hostname, None).await?;
            Ok(true)
        }
        Step::Failed(why) => {
            store::server_applied(db, node, &hostname, Some(&why)).await?;
            Ok(false)
        }
        Step::Waiting => Ok(false),
    }
}

async fn domain_round(db: &PgPool, row: &MailDomain) -> Result<(), TickError> {
    let mut current = row.clone();
    let node = row.node_id;
    let tenant = row.tenant_id;
    let domain = &row;
    if domain.state == "removing" {
        let op = Operation::RemoveMailDomain {
            domain: domain.fqdn.clone(),
        };
        match step(
            db,
            node,
            tenant,
            &op,
            &format!("maildomainrm:{}:{}", domain.id, domain.version),
        )
        .await?
        {
            Step::Done(_) => store::delete_domain(db, domain.id).await?,
            Step::Failed(why) => store::domain_failed(db, domain.id, &why).await?,
            Step::Waiting => {}
        }
        return Ok(());
    }
    // The plan may have changed: the outgoing limit follows it.
    if let Some(tenant_id) = domain.tenant_id
        && let Some(t) = tenants::get(db, tenant_id).await?
        && let Some(plan) = plans::get(db, t.plan_id).await?
    {
        let wanted = crate::per_hour_of(&plan);
        if wanted != domain.per_hour {
            store::set_per_hour(db, domain.id, wanted).await?;
            return Ok(());
        }
    }
    if domain.state == "error" {
        return Ok(());
    }
    let forwarders = store::forwarders_of_domain(db, domain.id).await?;
    if domain.version > domain.applied_version {
        let op = Operation::SetMailDomain {
            domain: domain.fqdn.clone(),
            catch_all: forwarders
                .iter()
                .find(|f| f.local_part == "*")
                .map(|f| f.destination.clone()),
            per_hour: domain.per_hour.and_then(|v| u32::try_from(v).ok()),
            held: domain.held,
        };
        match step(
            db,
            node,
            tenant,
            &op,
            &format!("maildomain:{}:{}", domain.id, domain.version),
        )
        .await?
        {
            Step::Done(Some(Output::MailDomainSet { records })) => {
                let records = serde_json::to_value(&records).unwrap_or_else(|_| json!([]));
                store::domain_applied(db, domain.id, domain.version, &records).await?;
            }
            Step::Done(_) => {
                store::domain_applied(db, domain.id, domain.version, &json!([])).await?;
            }
            Step::Failed(why) => {
                store::domain_failed(db, domain.id, &why).await?;
                return Ok(());
            }
            Step::Waiting => return Ok(()),
        }
        // The domain is on the node now: go on with the rest of it.
        match store::domain(db, domain.id).await? {
            Some(fresh) => current = fresh,
            None => return Ok(()),
        }
    }
    let domain = &current;
    if domain.applied_version == 0 {
        return Ok(());
    }
    if !domain.dns_applied {
        write_dns(db, domain).await?;
    }
    if domain.fwd_version > domain.fwd_applied {
        let op = Operation::SetMailForwarders {
            domain: domain.fqdn.clone(),
            forwarders: aggregate(&forwarders),
        };
        match step(
            db,
            node,
            tenant,
            &op,
            &format!("mailfwd:{}:{}", domain.id, domain.fwd_version),
        )
        .await?
        {
            Step::Done(_) => store::forwarders_applied(db, domain.id, domain.fwd_version).await?,
            Step::Failed(why) => store::domain_failed(db, domain.id, &why).await?,
            Step::Waiting => {}
        }
    }
    for mailbox in store::mailboxes_of_domain(db, domain.id).await? {
        if let Err(error) = mailbox_round(db, domain, &mailbox).await {
            tracing::warn!(domain = %domain.fqdn, mailbox = %mailbox.local_part, %error, "a mailbox could not finish a round");
        }
    }
    Ok(())
}

/// The forwarders as the node takes them: a list of destinations per name (the catch-all is
/// the domain's own setting).
fn aggregate(rows: &[store::Forwarder]) -> Vec<MailForwarder> {
    let mut out: Vec<MailForwarder> = Vec::new();
    for row in rows.iter().filter(|r| r.local_part != "*") {
        match out.iter_mut().find(|f| f.local == row.local_part) {
            Some(f) => f.to.push(row.destination.clone()),
            None => out.push(MailForwarder {
                local: row.local_part.clone(),
                to: vec![row.destination.clone()],
            }),
        }
    }
    out
}

async fn mailbox_round(db: &PgPool, domain: &MailDomain, m: &Mailbox) -> Result<(), TickError> {
    let node = domain.node_id;
    let tenant = domain.tenant_id;
    if m.state == "removing" {
        let op = Operation::DropMailbox {
            domain: domain.fqdn.clone(),
            local: m.local_part.clone(),
        };
        match step(
            db,
            node,
            tenant,
            &op,
            &format!("mailboxdrop:{}:{}", m.id, m.version),
        )
        .await?
        {
            Step::Done(_) => store::delete_mailbox(db, m.id).await?,
            Step::Failed(why) => store::mailbox_failed(db, m.id, &why).await?,
            Step::Waiting => {}
        }
        return Ok(());
    }
    if m.state == "error" {
        return Ok(());
    }
    if m.version > m.applied_version {
        // The password is put in by the dispatcher when the frame is sent.
        let op = Operation::UpsertMailbox {
            domain: domain.fqdn.clone(),
            local: m.local_part.clone(),
            password: None,
            quota_bytes: u64::try_from(m.quota_mb).unwrap_or(1) * 1024 * 1024,
            aliases: Vec::new(),
        };
        match step(
            db,
            node,
            tenant,
            &op,
            &format!("mailbox:{}:{}", m.id, m.version),
        )
        .await?
        {
            Step::Done(_) => store::mailbox_applied(db, m.id, m.version).await?,
            Step::Failed(why) => store::mailbox_failed(db, m.id, &why).await?,
            Step::Waiting => {}
        }
        return Ok(());
    }
    if m.ar_version > m.ar_applied {
        let enabled = m.ar_subject.is_some();
        let op = Operation::SetMailAutoresponder {
            domain: domain.fqdn.clone(),
            local: m.local_part.clone(),
            enabled,
            subject: m.ar_subject.clone().unwrap_or_default(),
            body: m.ar_body.clone().unwrap_or_default(),
            from: m.ar_from,
            until: m.ar_until,
        };
        match step(
            db,
            node,
            tenant,
            &op,
            &format!("mailar:{}:{}", m.id, m.ar_version),
        )
        .await?
        {
            Step::Done(_) => store::autoresponder_applied(db, m.id, m.ar_version).await?,
            Step::Failed(why) => store::mailbox_failed(db, m.id, &why).await?,
            Step::Waiting => {}
        }
    }
    Ok(())
}

// ------------------------------------------------------------------------------------ DNS

/// Puts the records the mail server asked for into the domain's zone. What the zone holds
/// already is left as it is, except a DKIM key, which follows the mail server's.
async fn write_dns(db: &PgPool, domain: &MailDomain) -> Result<(), TickError> {
    let wanted: Vec<MailDnsRecord> =
        serde_json::from_value(domain.records.clone()).unwrap_or_default();
    if wanted.is_empty() {
        return Ok(());
    }
    // A domain whose DNS is kept somewhere else has no zone here: the records are shown in
    // the admin's deliverability page instead.
    let Some(domain_id) = domain.domain_id else {
        return Ok(());
    };
    let Some(zone) = dns::zone_of_domain(db, domain_id).await? else {
        return Ok(());
    };
    let mut tx = db.begin().await?;
    let existing = dns::records_in(&mut tx, zone.id).await?;
    let mut changed = false;
    for w in &wanted {
        let record = match DnsRecord::parse(
            &w.name,
            &w.rtype,
            &w.content,
            None,
            w.priority.map(u32::from),
        ) {
            Ok(r) => r,
            Err(error) => {
                tracing::warn!(domain = %domain.fqdn, name = %w.name, %error, "a mail record is not valid");
                continue;
            }
        };
        let same_name_type = existing
            .iter()
            .filter(|e| e.record.name == record.name && e.record.rtype == record.rtype)
            .collect::<Vec<_>>();
        let is_dkim = record.rtype == DnsRecordType::Txt && record.name.ends_with("._domainkey");
        if is_dkim && let Some(old) = same_name_type.first() {
            if old.record.content != record.content {
                dns::change_record(&mut tx, zone.id, old.id, &record)
                    .await
                    .map_err(|e| TickError::Queue(e.to_string()))?;
                changed = true;
            }
            continue;
        }
        // The owner's own SPF, DMARC or MX is not replaced: the deliverability page says when
        // it is not what the mail server asks for.
        let owned = |prefix: &str| {
            record.content.starts_with(prefix)
                && same_name_type
                    .iter()
                    .any(|e| e.record.content.starts_with(prefix))
        };
        if owned("v=spf1")
            || owned("v=DMARC1")
            || (record.rtype == DnsRecordType::Mx && !same_name_type.is_empty())
        {
            continue;
        }
        if check_dns_conflicts(existing.iter().map(|e| &e.record), &record).is_err() {
            continue;
        }
        match dns::add_record(&mut tx, zone.id, &record).await {
            Ok(_) => changed = true,
            Err(dns::RecordWriteError::Duplicate) => {}
            Err(dns::RecordWriteError::Database(e)) => return Err(e.into()),
        }
    }
    if changed {
        dns::mark_changed(&mut tx, zone.id).await?;
    }
    tx.commit().await?;
    store::dns_done(db, domain.id).await?;
    Ok(())
}

// ----------------------------------------------------------------------------------- abuse

/// Holds the domains with too many messages waiting in the outgoing queue (T13.07): their mail
/// is limited to one a day, the waiting messages are removed, and the account and the
/// administrators are told.
async fn hold_abusers(db: &PgPool, threshold: i64) -> Result<(), TickError> {
    if threshold <= 0 {
        return Ok(());
    }
    let reports = store::queues(db).await?;
    if reports.is_empty() {
        return Ok(());
    }
    let domains = store::all_domains(db).await?;
    for (node, items, _) in reports {
        let items: Vec<MailQueueItem> = serde_json::from_value(items).unwrap_or_default();
        for domain in domains.iter().filter(|d| d.node_id == node && !d.held) {
            let suffix = format!("@{}", domain.fqdn);
            let mine: Vec<&MailQueueItem> = items
                .iter()
                .filter(|i| i.sender.to_ascii_lowercase().ends_with(&suffix))
                .collect();
            if i64::try_from(mine.len()).unwrap_or(i64::MAX) <= threshold {
                continue;
            }
            let reason = format!(
                "{} messages were waiting in the outgoing queue (the limit is {threshold}).",
                mine.len()
            );
            if !store::set_held(db, domain.id, true, Some(&reason)).await? {
                continue;
            }
            tracing::warn!(domain = %domain.fqdn, queued = mine.len(), "a mail domain was held for sending too much");
            let ids: Vec<String> = mine
                .iter()
                .take(MAX_REMOVED)
                .map(|i| i.id.clone())
                .collect();
            let op = Operation::MailQueueAction {
                action: "delete".to_owned(),
                ids,
            };
            step(
                db,
                node,
                None,
                &op,
                &format!("mailhold:{}:{}", domain.id, now() / 60),
            )
            .await?;
            tell(db, domain, &reason).await?;
        }
    }
    Ok(())
}

/// Puts the news of a hold in the inboxes of the account's owner and the administrators.
async fn tell(db: &PgPool, domain: &MailDomain, reason: &str) -> Result<(), TickError> {
    let mut recipients = notifications::admins(db).await?;
    if let Some(tenant_id) = domain.tenant_id
        && let Some(owner) = notifications::tenant_owner(db, tenant_id).await?
    {
        recipients.push(owner);
    }
    let subject = format!("Outgoing mail of {} is held", domain.fqdn);
    let body = format!(
        "Mail from {} was held: {reason} Messages that were waiting have been removed. \
         Outgoing mail is limited to one message a day until an administrator releases it.",
        domain.fqdn
    );
    let data = json!({ "mail_domain_id": domain.id, "domain": domain.fqdn });
    let mut conn = db.acquire().await?;
    for r in recipients {
        notifications::add(
            &mut conn,
            &NewNotification {
                account_id: r.account_id,
                event_type: "mail_domain_held",
                severity: "warning",
                subject: &subject,
                body: &body,
                data: &data,
                dedupe_key: &format!("mail-held:{}:{}", domain.id, now() / 3600),
            },
        )
        .await?;
    }
    Ok(())
}
