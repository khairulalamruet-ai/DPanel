//! Running a planned import (T15.04).
//!
//! The executor does what the plan lists, in its order, by using the panel's own API as the
//! people who would do it by hand: the account is made as the administrator who started the
//! transfer, everything else as the new account (a session "on behalf of" that administrator,
//! like "login as user"). So the plan limits, the checks, the audit trail and the work sent to the
//! node are exactly those of any other request, and nothing here talks to a node itself.
//!
//! Every step is recorded as it starts and ends. A step that fails ends the transfer; running
//! it again goes on from that step (what an earlier step made is looked up, not made twice, and a
//! name that is already there because of an earlier try counts as done).

use std::{
    io::Write as _,
    path::{Path as FsPath, PathBuf},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use builtpanel_database::migration_transfers::{self as store, Progress, Transfer};
use builtpanel_import::{Action, DomainKind, ImportModel, Plan};
use builtpanel_kernel::ModuleMount;
use builtpanel_session::{CSRF_HEADER, NewSession, RevokeReason, SessionStore, set_cookie};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::PgPool;
use tower::ServiceExt as _;
use uuid::Uuid;

use crate::MigrationState;

/// How long the node may take to set up a new account.
const ACCOUNT_WAIT: Duration = Duration::from_secs(15 * 60);
/// How long a database, a user or a transfer to the node may take.
const NODE_WAIT: Duration = Duration::from_secs(60 * 60);
/// How often something that the node does is looked at.
const POLL: Duration = Duration::from_secs(2);
/// The size of one uploaded piece (the API takes at most 8 MiB).
const CHUNK: usize = 8 * 1024 * 1024;
/// Where the packed home folder waits in the account's files while it is unpacked.
const HOLDING: &str = ".bp-import/home.tar.gz";

/// How the executor calls the panel's own routes. The mount is set when the panel starts.
#[derive(Clone, Default)]
pub struct Loopback(pub Arc<OnceLock<ModuleMount>>);

/// A session of the executor.
struct Login {
    cookie: String,
    value: String,
    csrf: String,
}

type Answer = (StatusCode, Value);

impl Loopback {
    async fn call(
        &self,
        who: &Login,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> Result<Answer, String> {
        let (content, body) = match body {
            Some(v) => (Some("application/json"), Body::from(v.to_string())),
            None => (None, Body::empty()),
        };
        self.send(who, method, uri, content, body).await
    }

    async fn send(
        &self,
        who: &Login,
        method: &str,
        uri: &str,
        content_type: Option<&str>,
        body: Body,
    ) -> Result<Answer, String> {
        let mount = self
            .0
            .get()
            .ok_or_else(|| "the panel is not ready to run imports".to_owned())?;
        let mut req = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::COOKIE, &who.cookie)
            .header(CSRF_HEADER, &who.csrf);
        if let Some(c) = content_type {
            req = req.header(header::CONTENT_TYPE, c);
        }
        let mut req = req.body(body).map_err(|e| e.to_string())?;
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                0,
            ))));
        let res = tokio::time::timeout(Duration::from_secs(120), mount.service().oneshot(req))
            .await
            .map_err(|_| format!("{method} {uri} took too long"))?
            .unwrap_or_else(|never| match never {});
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
            .await
            .map_err(|e| e.to_string())?;
        Ok((
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        ))
    }
}

async fn login(
    sessions: &SessionStore,
    account: Uuid,
    on_behalf_of: Option<Uuid>,
) -> Result<Login, String> {
    let new = NewSession {
        account_id: account,
        ip: None,
        user_agent: Some("builtpanel-import".to_owned()),
    };
    let (token, data) = match on_behalf_of {
        Some(admin) => sessions.create_impersonated(new, admin).await,
        None => sessions.create(new).await,
    }
    .map_err(|e| format!("a session could not be opened: {e}"))?;
    let cookie = set_cookie(&token)
        .split(';')
        .next()
        .unwrap_or("")
        .to_owned();
    let value = cookie.split_once('=').map_or("", |(_, v)| v).to_owned();
    Ok(Login {
        cookie,
        value,
        csrf: data.csrf_token.expose().to_owned(),
    })
}

/// What the error of a call says, short.
fn why(answer: &Answer) -> String {
    let (status, body) = answer;
    let code = body["code"].as_str().unwrap_or("");
    let detail = body["detail"]
        .as_str()
        .or_else(|| body["title"].as_str())
        .unwrap_or("");
    let fields = body["errors"].to_string();
    let mut text = format!("{status} {code} {detail}");
    if fields != "null" {
        text.push(' ');
        text.push_str(&fields);
    }
    text.chars().take(400).collect()
}

fn code_of(answer: &Answer) -> &str {
    answer.1["code"].as_str().unwrap_or("")
}

/// A password nobody knows: the new account's owner sets their own with "forgot password".
fn throwaway_password() -> String {
    format!("Aa1!{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

struct Run<'a> {
    state: &'a MigrationState,
    transfer: Transfer,
    plan: Plan,
    admin: Uuid,
    steps: Vec<Value>,
    client: Option<Login>,
    cron_seen: usize,
}

/// Runs the plan of a transfer. Meant to be spawned: it ends when the transfer has.
pub async fn run(state: MigrationState, id: Uuid) {
    let Ok(Some(transfer)) = store::get(&state.db, id).await else {
        return;
    };
    let Some(plan) = transfer
        .plan
        .clone()
        .and_then(|p| serde_json::from_value::<Plan>(p).ok())
    else {
        fail(&state, &transfer, "the transfer has no plan").await;
        return;
    };
    let Some(admin) = transfer.created_by else {
        fail(
            &state,
            &transfer,
            "the administrator who started it is gone",
        )
        .await;
        return;
    };
    let steps = transfer.steps.as_array().cloned().unwrap_or_default();
    let mut run = Run {
        state: &state,
        transfer,
        plan,
        admin,
        steps,
        client: None,
        cron_seen: 0,
    };
    let result = run.all().await;
    if let Some(client) = run.client.take() {
        let _ = state
            .sessions
            .revoke(&client.value, RevokeReason::Security)
            .await;
    }
    if let Err(message) = result {
        let t = run.transfer.clone();
        fail(&state, &t, &message).await;
    }
}

/// Ends a transfer as failed, with the reason.
pub async fn fail_transfer(state: &MigrationState, t: &Transfer, message: &str) {
    fail(state, t, message).await;
}

async fn fail(state: &MigrationState, t: &Transfer, message: &str) {
    let _ = store::append_log(&state.db, t.id, &[format!("Failed: {message}")]).await;
    let _ = store::set_progress(
        &state.db,
        t.id,
        &Progress {
            status: "failed",
            progress_percent: t.progress_percent,
            current_step: &t.current_step,
            size_bytes: None,
            steps: &t.steps,
            plan: None,
            error_message: Some(&message.chars().take(900).collect::<String>()),
        },
    )
    .await;
}

impl Run<'_> {
    async fn log(&self, line: impl Into<String>) {
        let _ = store::append_log(&self.state.db, self.transfer.id, &[line.into()]).await;
    }

    async fn save(&mut self, status: &str, current: &str, error: Option<&str>) {
        let total = self.steps.len().max(1);
        let done = self
            .steps
            .iter()
            .filter(|s| s["status"] == "completed")
            .count();
        let percent = i32::try_from(done * 100 / total).unwrap_or(100);
        let steps = Value::Array(self.steps.clone());
        self.transfer.steps = steps.clone();
        self.transfer.current_step = current.to_owned();
        self.transfer.progress_percent = percent;
        let _ = store::set_progress(
            &self.state.db,
            self.transfer.id,
            &Progress {
                status,
                progress_percent: percent,
                current_step: current,
                size_bytes: None,
                steps: &steps,
                plan: None,
                error_message: error,
            },
        )
        .await;
    }

    async fn cancelled(&self) -> bool {
        matches!(
            store::get(&self.state.db, self.transfer.id).await,
            Ok(Some(t)) if t.status == "cancelled"
        )
    }

    async fn all(&mut self) -> Result<(), String> {
        let list = self.plan.steps.clone();
        for (i, step) in list.iter().enumerate() {
            if self
                .steps
                .get(i)
                .is_some_and(|s| s["status"] == "completed")
            {
                if step.action == Action::AddCronJob {
                    self.cron_seen += 1;
                }
                continue;
            }
            if self.cancelled().await {
                self.log("Cancelled.").await;
                return Ok(());
            }
            let name = self.steps[i]["name"].as_str().unwrap_or("").to_owned();
            self.steps[i]["status"] = json!("running");
            self.save("running", &name, None).await;
            self.log(format!("Start: {name}")).await;
            match self.step(step).await {
                Ok(detail) => {
                    self.steps[i]["status"] = json!("completed");
                    if let Some(d) = &detail {
                        self.steps[i]["details"] = json!(d);
                        self.log(format!("Done: {name}. {d}")).await;
                    } else {
                        self.log(format!("Done: {name}.")).await;
                    }
                    self.save("running", &name, None).await;
                }
                Err(message) => {
                    self.steps[i]["status"] = json!("failed");
                    self.steps[i]["details"] = json!(message);
                    self.save("running", &name, None).await;
                    return Err(format!("{name}: {message}"));
                }
            }
        }
        self.log("The import has finished.").await;
        self.save("completed", "Import finished", None).await;
        Ok(())
    }

    // ------------------------------------------------------------------ the steps

    async fn step(&mut self, step: &builtpanel_import::Step) -> Result<Option<String>, String> {
        match step.action {
            Action::CreateAccount => self.create_account().await,
            Action::AddDomain => self.add_domain(&step.subject).await,
            Action::RestoreFiles => self.restore_files().await,
            Action::CreateDatabase => self.create_database(&step.subject).await,
            Action::ImportDump => self.import_dump(&step.subject).await,
            Action::CreateDbUser => self.create_db_user(&step.subject).await,
            Action::AddCronJob => {
                let n = self.cron_seen;
                self.cron_seen += 1;
                self.add_cron(n).await
            }
            Action::CreateDnsZone => self.import_dns(&step.subject).await,
            Action::CreateMailbox | Action::CreateForwarder => {
                Ok(Some("Skipped: mail is not imported yet.".to_owned()))
            }
        }
    }

    /// Adds the records of a zone to the domain's zone here. Records that point at the old
    /// server are left out (the zone here already points the domain at this one), and so is
    /// whatever the API refuses: a record that is a duplicate of one that is there, or that
    /// breaks a rule of this panel's DNS. None of that stops the import.
    async fn import_dns(&mut self, zone: &str) -> Result<Option<String>, String> {
        let Some(z) = self
            .model()
            .dns_zones
            .iter()
            .find(|z| z.name == zone)
            .cloned()
        else {
            return Err("the zone is not in the plan's model".to_owned());
        };
        let Some(domain) = self.domain_id(zone).await? else {
            return Ok(Some("The domain is not on this panel.".to_owned()));
        };
        let old_server: Vec<&str> = z
            .records
            .iter()
            .filter(|r| r.name == "@" && matches!(r.rtype.as_str(), "A" | "AAAA"))
            .map(|r| r.content.as_str())
            .collect();
        // A zone may have only one SPF record: when the zone here has one (from its template),
        // the old one is not added next to it.
        let listed = self
            .call(
                "GET",
                &format!("/api/v1/client/domains/{domain}/dns/records"),
                None,
            )
            .await?;
        let has_spf = listed.1["items"].as_array().is_some_and(|items| {
            items.iter().any(|r| {
                r["type"] == "TXT"
                    && r["name"] == "@"
                    && r["content"]
                        .as_str()
                        .is_some_and(|c| c.trim_matches('"').starts_with("v=spf1"))
            })
        });
        let (mut added, mut same, mut old, mut refused) = (0, 0, 0, Vec::new());
        for r in &z.records {
            if matches!(r.rtype.as_str(), "A" | "AAAA") && old_server.contains(&r.content.as_str())
            {
                old += 1;
                continue;
            }
            if r.rtype == "NS" && r.name == "@" {
                continue;
            }
            if has_spf
                && r.rtype == "TXT"
                && r.name == "@"
                && r.content.trim_matches('"').starts_with("v=spf1")
            {
                refused.push("TXT @: the zone here has an SPF record already".to_owned());
                continue;
            }
            let host = |t: &str| t.trim_end_matches('.').to_owned();
            let content = match r.rtype.as_str() {
                "CNAME" | "MX" | "NS" => host(&r.content),
                // `weight port target`
                "SRV" => {
                    let mut parts: Vec<String> =
                        r.content.split_whitespace().map(str::to_owned).collect();
                    if let Some(last) = parts.last_mut() {
                        *last = host(last);
                    }
                    parts.join(" ")
                }
                _ => r.content.clone(),
            };
            let mut body = json!({ "name": r.name, "type": r.rtype, "content": content });
            if let Some(ttl) = r.ttl {
                body["ttl"] = json!(ttl.clamp(60, 604_800));
            }
            if matches!(r.rtype.as_str(), "MX" | "SRV") {
                body["priority"] = json!(r.priority.unwrap_or(10));
            }
            let answer = self
                .call(
                    "POST",
                    &format!("/api/v1/client/domains/{domain}/dns/records"),
                    Some(body),
                )
                .await?;
            if answer.0 == StatusCode::CREATED {
                added += 1;
            } else if answer.0 == StatusCode::CONFLICT && code_of(&answer) == "duplicate_record" {
                same += 1;
            } else if refused.len() < 3 {
                refused.push(format!("{} {}: {}", r.rtype, r.name, why(&answer)));
            } else {
                refused.push(String::new());
            }
        }
        for line in refused.iter().filter(|l| !l.is_empty()) {
            self.log(format!("A DNS record was left out: {line}")).await;
        }
        let refused_count = refused.len();
        Ok(Some(format!(
            "{added} record(s) added, {same} already there, {old} that point at the old server \
             left out, {refused_count} refused. Check the SPF record and any address in it: it may \
             still name the old server."
        )))
    }

    fn model(&self) -> &ImportModel {
        &self.plan.model
    }

    async fn account_id(&self) -> Result<Uuid, String> {
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM accounts WHERE username = $1")
            .bind(&self.model().account.username)
            .fetch_optional(&self.state.db)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "the account is not there".to_owned())
    }

    async fn client(&mut self) -> Result<&Login, String> {
        if self.client.is_none() {
            let account = self.account_id().await?;
            self.client = Some(login(&self.state.sessions, account, Some(self.admin)).await?);
        }
        self.client.as_ref().ok_or_else(|| "no session".to_owned())
    }

    async fn call(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> Result<Answer, String> {
        let lb = self.state.loopback.clone();
        let who = self.client().await?;
        lb.call(who, method, uri, body).await
    }

    async fn create_account(&mut self) -> Result<Option<String>, String> {
        let username = self.model().account.username.clone();
        let email = self
            .model()
            .account
            .email
            .clone()
            .unwrap_or_else(|| format!("{username}@import.invalid"));
        let exists: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM accounts WHERE username = $1")
                .bind(&username)
                .fetch_optional(&self.state.db)
                .await
                .map_err(|e| e.to_string())?;
        let account = match exists {
            // An earlier try made it (the plan was checked when the transfer started).
            Some(id) => id,
            None => {
                let who = login(&self.state.sessions, self.admin, None).await?;
                let mut body = json!({
                    "username": username, "email": email, "password": throwaway_password()
                });
                if let Some(locale) = self.model().account.locale.as_deref()
                    && matches!(locale, "en" | "bn")
                {
                    body["locale"] = json!(locale);
                }
                let answer = self
                    .state
                    .loopback
                    .call(&who, "POST", "/api/v1/admin/users", Some(body))
                    .await?;
                let _ = self
                    .state
                    .sessions
                    .revoke(&who.value, RevokeReason::Security)
                    .await;
                if answer.0 != StatusCode::CREATED {
                    return Err(why(&answer));
                }
                answer.1["user"]["id"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| "the answer has no account id".to_owned())?
            }
        };
        let started = Instant::now();
        loop {
            let status: Option<String> =
                sqlx::query_scalar("SELECT status FROM tenants WHERE owner_account_id = $1")
                    .bind(account)
                    .fetch_optional(&self.state.db)
                    .await
                    .map_err(|e| e.to_string())?;
            match status.as_deref() {
                Some("active") => break,
                Some("failed") => return Err("the node could not set the account up".to_owned()),
                Some("provisioning") | None => {}
                Some(other) => return Err(format!("the account is {other}")),
            }
            if started.elapsed() > ACCOUNT_WAIT {
                return Err("the node did not set the account up in time".to_owned());
            }
            if self.cancelled().await {
                return Ok(Some("Cancelled".to_owned()));
            }
            tokio::time::sleep(POLL).await;
        }
        Ok(Some(
            "The account has no password anyone knows: its owner uses \"forgot password\"."
                .to_owned(),
        ))
    }

    async fn domain_id(&self, fqdn: &str) -> Result<Option<Uuid>, String> {
        sqlx::query_scalar("SELECT id FROM domains WHERE fqdn = $1")
            .bind(fqdn)
            .fetch_optional(&self.state.db)
            .await
            .map_err(|e| e.to_string())
    }

    /// The folder a domain's files are in, below the web directory: `public_html` is the web
    /// directory itself.
    fn docroot(raw: Option<&str>) -> Option<String> {
        let raw = raw?.trim_matches('/');
        let rel = raw
            .strip_prefix("public_html/")
            .or_else(|| (raw != "public_html").then_some(raw))?;
        (!rel.is_empty()).then(|| rel.to_owned())
    }

    async fn add_domain(&mut self, fqdn: &str) -> Result<Option<String>, String> {
        let Some(domain) = self
            .model()
            .domains
            .iter()
            .find(|d| d.name == fqdn)
            .cloned()
        else {
            return Err("the domain is not in the plan's model".to_owned());
        };
        let mut body = json!({ "fqdn": fqdn });
        match domain.kind {
            DomainKind::Main => body["kind"] = json!("primary"),
            DomainKind::Addon => body["kind"] = json!("addon"),
            DomainKind::Sub | DomainKind::Alias => {
                // The longest of the account's domains this name ends with, or the main one.
                let mut names: Vec<&str> = self
                    .model()
                    .domains
                    .iter()
                    .filter(|d| d.name != fqdn)
                    .map(|d| d.name.as_str())
                    .collect();
                names.sort_by_key(|n| std::cmp::Reverse(n.len()));
                let parent = names
                    .iter()
                    .find(|n| fqdn.ends_with(&format!(".{n}")))
                    .copied()
                    .or_else(|| {
                        self.model()
                            .domains
                            .iter()
                            .find(|d| d.kind == DomainKind::Main)
                            .map(|d| d.name.as_str())
                    })
                    .map(str::to_owned);
                let parent_id = match parent {
                    Some(p) => self.domain_id(&p).await?,
                    None => None,
                };
                body["kind"] = json!(if domain.kind == DomainKind::Sub {
                    "sub"
                } else {
                    "alias"
                });
                if let Some(id) = parent_id {
                    body["parent_domain_id"] = json!(id);
                }
            }
        }
        if let Some(root) = Self::docroot(domain.docroot.as_deref()) {
            body["docroot"] = json!(root);
        }
        let answer = self
            .call("POST", "/api/v1/client/domains", Some(body))
            .await?;
        if answer.0 == StatusCode::CREATED {
            return Ok(None);
        }
        if answer.0 == StatusCode::CONFLICT && self.domain_id(fqdn).await?.is_some() {
            return Ok(Some("Already there.".to_owned()));
        }
        Err(why(&answer))
    }

    async fn wait_ready(&self, table: &str, name: &str) -> Result<(), String> {
        let sql = format!("SELECT status FROM {table} WHERE name = $1");
        let started = Instant::now();
        loop {
            let status: Option<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.clone()))
                .bind(name)
                .fetch_optional(&self.state.db)
                .await
                .map_err(|e| e.to_string())?;
            match status.as_deref() {
                Some("ready") => return Ok(()),
                Some("failed") => return Err(format!("`{name}` could not be made on the node")),
                _ => {}
            }
            if started.elapsed() > NODE_WAIT {
                return Err(format!("`{name}` was not ready in time"));
            }
            if self.cancelled().await {
                return Err("cancelled".to_owned());
            }
            tokio::time::sleep(POLL).await;
        }
    }

    async fn create_database(&mut self, name: &str) -> Result<Option<String>, String> {
        let answer = self
            .call(
                "POST",
                "/api/v1/client/databases",
                Some(json!({ "name": name })),
            )
            .await?;
        let made = answer.0 == StatusCode::CREATED;
        let again = answer.0 == StatusCode::CONFLICT && code_of(&answer) == "name_taken";
        if !made && !again {
            return Err(why(&answer));
        }
        self.wait_ready("databases", name).await?;
        Ok(again.then(|| "Already there.".to_owned()))
    }

    async fn db_id(&self, table: &str, name: &str) -> Result<Uuid, String> {
        let sql = format!("SELECT id FROM {table} WHERE name = $1");
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .bind(name)
            .fetch_optional(&self.state.db)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("`{name}` is not there"))
    }

    fn temp_file(&self, what: &str) -> PathBuf {
        self.state
            .spool_dir
            .join("migration")
            .join(format!("run-{}-{what}", self.transfer.id.simple()))
    }

    fn backup_path(&self) -> PathBuf {
        self.state
            .spool_dir
            .join("migration")
            .join(&self.transfer.backup_file)
    }

    async fn import_dump(&mut self, db: &str) -> Result<Option<String>, String> {
        let Some(path) = self
            .model()
            .databases
            .iter()
            .find(|d| d.name == db)
            .and_then(|d| d.dump.clone())
        else {
            return Ok(Some("The backup has no dump of it.".to_owned()));
        };
        let tmp = self.temp_file("dump.sql");
        let (tmp2, backup, wanted) = (tmp.clone(), self.backup_path(), path.as_str().to_owned());
        let (size, sha) = tokio::task::spawn_blocking(move || -> Result<(u64, String), String> {
            let file = std::fs::File::open(&backup).map_err(|e| e.to_string())?;
            let out = std::fs::File::create(&tmp2).map_err(|e| e.to_string())?;
            let mut out = HashingWriter::new(out);
            let n = builtpanel_import::copy_file(std::io::BufReader::new(file), &wanted, &mut out)
                .map_err(|e| e.to_string())?;
            out.flush().map_err(|e| e.to_string())?;
            Ok((n, out.finish()))
        })
        .await
        .map_err(|e| e.to_string())??;
        let result = async {
            if size == 0 {
                return Ok(Some("The dump is empty.".to_owned()));
            }
            let id = self.db_id("databases", db).await?;
            let answer = self
                .call(
                    "POST",
                    &format!("/api/v1/client/databases/{id}/imports"),
                    Some(json!({ "size_bytes": size, "sha256": sha, "gzip": false })),
                )
                .await?;
            if answer.0 != StatusCode::CREATED {
                return Err(why(&answer));
            }
            let import = answer.1["id"].as_str().unwrap_or_default().to_owned();
            self.send_chunks(&tmp, &format!("/api/v1/client/imports/{import}"))
                .await?;
            self.wait_done(&format!("/api/v1/client/imports/{import}"))
                .await?;
            Ok(None)
        }
        .await;
        let _ = std::fs::remove_file(&tmp);
        result
    }

    /// A call to the files routes that work on the node: answered at once, or with `202` and
    /// the operation to wait for.
    async fn file_op(
        &mut self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> Result<(), String> {
        let answer = self.call(method, uri, body).await?;
        if answer.0 == StatusCode::OK {
            return Ok(());
        }
        if answer.0 != StatusCode::ACCEPTED {
            return Err(why(&answer));
        }
        let op = answer.1["operation_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let started = Instant::now();
        loop {
            let polled = self
                .call(
                    "GET",
                    &format!("/api/v1/client/files/operations/{op}"),
                    None,
                )
                .await?;
            if polled.0 == StatusCode::OK {
                return Ok(());
            }
            if polled.0 != StatusCode::ACCEPTED {
                return Err(why(&polled));
            }
            if started.elapsed() > NODE_WAIT {
                return Err("the node did not finish in time".to_owned());
            }
            if self.cancelled().await {
                return Err("cancelled".to_owned());
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// Sends a file as chunks, then says it is whole.
    async fn send_chunks(&mut self, file: &FsPath, base: &str) -> Result<(), String> {
        let lb = self.state.loopback.clone();
        let mut data = std::fs::File::open(file).map_err(|e| e.to_string())?;
        let mut n = 0u32;
        loop {
            let mut buf = vec![0u8; CHUNK];
            let mut filled = 0;
            while filled < CHUNK {
                use std::io::Read as _;
                let r = data.read(&mut buf[filled..]).map_err(|e| e.to_string())?;
                if r == 0 {
                    break;
                }
                filled += r;
            }
            if filled == 0 {
                break;
            }
            buf.truncate(filled);
            let who = self.client().await?;
            let answer = lb
                .send(
                    who,
                    "PUT",
                    &format!("{base}/chunks/{n}"),
                    Some("application/octet-stream"),
                    Body::from(buf),
                )
                .await?;
            if !answer.0.is_success() {
                return Err(why(&answer));
            }
            n += 1;
        }
        let answer = self.call("POST", &format!("{base}/complete"), None).await?;
        if !answer.0.is_success() {
            return Err(why(&answer));
        }
        Ok(())
    }

    /// Waits until the node has finished with an upload or an import.
    async fn wait_done(&mut self, uri: &str) -> Result<(), String> {
        let started = Instant::now();
        loop {
            let answer = self.call("GET", uri, None).await?;
            if !answer.0.is_success() {
                return Err(why(&answer));
            }
            match answer.1["status"].as_str() {
                Some("done") => return Ok(()),
                Some("failed" | "cancelled" | "expired") => {
                    return Err(format!(
                        "the node ended it: {}",
                        answer.1["error"].as_str().unwrap_or("failed")
                    ));
                }
                _ => {}
            }
            if started.elapsed() > NODE_WAIT {
                return Err("the node did not finish in time".to_owned());
            }
            if self.cancelled().await {
                return Err("cancelled".to_owned());
            }
            tokio::time::sleep(POLL).await;
        }
    }

    async fn create_db_user(&mut self, name: &str) -> Result<Option<String>, String> {
        let Some(user) = self
            .model()
            .db_users
            .iter()
            .find(|u| u.name == name)
            .cloned()
        else {
            return Err("the user is not in the plan's model".to_owned());
        };
        let answer = self
            .call(
                "POST",
                "/api/v1/client/db-users",
                Some(json!({ "name": name, "password": throwaway_password() })),
            )
            .await?;
        let made = answer.0 == StatusCode::CREATED;
        let again = answer.0 == StatusCode::CONFLICT;
        if !made && !again {
            return Err(why(&answer));
        }
        self.wait_ready("db_users", name).await?;
        let user_id = self.db_id("db_users", name).await?;
        let mut granted = 0;
        for grant in &user.grants {
            let Ok(db_id) = self.db_id("databases", &grant.database).await else {
                continue;
            };
            let read_only = !grant.all_privileges
                && !grant.privileges.is_empty()
                && grant.privileges.iter().all(|p| p == "SELECT");
            let access = if grant.all_privileges {
                "all"
            } else if read_only {
                "read_only"
            } else {
                "read_write"
            };
            let answer = self
                .call(
                    "PUT",
                    &format!("/api/v1/client/databases/{db_id}/grants/{user_id}"),
                    Some(json!({ "access": access })),
                )
                .await?;
            if !answer.0.is_success() {
                return Err(why(&answer));
            }
            granted += 1;
        }
        Ok(Some(format!(
            "New password set (the old one is not carried over); {granted} grant(s). \
             The application's configuration needs the new password."
        )))
    }

    async fn add_cron(&mut self, n: usize) -> Result<Option<String>, String> {
        let Some(job) = self.model().cron_jobs.get(n).cloned() else {
            return Ok(Some("Not in the plan's model.".to_owned()));
        };
        let answer = self
            .call(
                "POST",
                "/api/v1/client/cron-jobs",
                Some(json!({
                    "name": format!("imported-{}", n + 1),
                    "schedule": job.schedule,
                    "command": job.command,
                })),
            )
            .await?;
        if answer.0 == StatusCode::CREATED {
            return Ok(Some(
                "Check the command: paths of the old server may not exist here.".to_owned(),
            ));
        }
        if answer.0 == StatusCode::CONFLICT && code_of(&answer) != "limit_reached" {
            return Ok(Some("Already there.".to_owned()));
        }
        // A schedule or command the rules do not allow is left out, not a reason to stop.
        if answer.0 == StatusCode::UNPROCESSABLE_ENTITY || code_of(&answer) == "limit_reached" {
            return Ok(Some(format!("Left out: {}", why(&answer))));
        }
        Err(why(&answer))
    }

    async fn restore_files(&mut self) -> Result<Option<String>, String> {
        let Some(files) = self.model().files.clone() else {
            return Ok(Some("The backup has no files.".to_owned()));
        };
        let home = files.root.as_str().to_owned();
        // The web directory gets what is in `public_html`; the folders of other domains keep
        // their names. Mail, logs and the panel's own folders stay behind.
        let keep: Vec<String> = self
            .model()
            .domains
            .iter()
            .filter_map(|d| Self::docroot(d.docroot.as_deref()))
            .collect();
        let tmp = self.temp_file("home.tar.gz");
        let (tmp2, backup) = (tmp.clone(), self.backup_path());
        let packed = tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(&backup).map_err(|e| e.to_string())?;
            let out = std::fs::File::create(&tmp2).map_err(|e| e.to_string())?;
            let prefix = format!("{home}/");
            builtpanel_import::repack(
                std::io::BufReader::new(file),
                |p| {
                    let rel = p.strip_prefix(&prefix)?;
                    if let Some(rest) = rel.strip_prefix("public_html/") {
                        return Some(rest.to_owned());
                    }
                    keep.iter()
                        .any(|k| rel == k || rel.starts_with(&format!("{k}/")))
                        .then(|| rel.to_owned())
                },
                std::io::BufWriter::new(out),
            )
            .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;
        let result = self.upload_home(&tmp, packed).await;
        let _ = std::fs::remove_file(&tmp);
        result
    }

    async fn upload_home(
        &mut self,
        tmp: &FsPath,
        packed: builtpanel_import::Repacked,
    ) -> Result<Option<String>, String> {
        if packed.entries == 0 {
            return Ok(Some("There was nothing to restore.".to_owned()));
        }
        let size = std::fs::metadata(tmp).map_err(|e| e.to_string())?.len();
        let answer = self
            .call(
                "POST",
                "/api/v1/client/files/uploads",
                Some(json!({ "path": HOLDING, "size_bytes": size, "overwrite": true })),
            )
            .await?;
        if answer.0 != StatusCode::CREATED {
            return Err(why(&answer));
        }
        let id = answer.1["id"].as_str().unwrap_or_default().to_owned();
        self.send_chunks(tmp, &format!("/api/v1/client/files/uploads/{id}"))
            .await?;
        self.wait_done(&format!("/api/v1/client/files/transfers/{id}"))
            .await?;
        let unpacked = self
            .file_op(
                "POST",
                "/api/v1/client/files/extract",
                Some(json!({ "path": HOLDING, "dest": "." })),
            )
            .await;
        // The packed copy is removed whatever happened (it is in the web directory's reach).
        let removed = self
            .file_op(
                "DELETE",
                &format!("/api/v1/client/files?path={HOLDING}"),
                None,
            )
            .await;
        let _ = self
            .file_op("DELETE", "/api/v1/client/files/trash", None)
            .await;
        unpacked?;
        if let Err(e) = removed {
            self.log(format!("The packed copy could not be removed: {e}"))
                .await;
        }
        Ok(Some(format!(
            "{} file(s) and folder(s), {} byte(s); {} link(s) and special file(s) left out.",
            packed.entries, packed.bytes, packed.skipped
        )))
    }
}

/// Writes to a file and hashes what it writes.
struct HashingWriter {
    inner: std::fs::File,
    hash: Sha256,
}

impl HashingWriter {
    fn new(inner: std::fs::File) -> Self {
        Self {
            inner,
            hash: Sha256::new(),
        }
    }

    fn finish(self) -> String {
        self.hash
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

impl std::io::Write for HashingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hash.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// A transfer that was running when the panel stopped cannot go on (what it was waiting for is
/// gone): it is ended, and running it again goes on from its step.
///
/// # Errors
/// Database errors.
pub async fn end_interrupted(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE migration_transfers SET status = 'failed', \
         error_message = 'The panel stopped while this was running; run it again.', \
         updated_at = now() WHERE status = 'running' AND NOT dry_run",
    )
    .execute(pool)
    .await?;
    Ok(())
}
