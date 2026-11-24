//! The node's mail server (T13.02-T13.08): Stalwart in a container, managed through its JMAP
//! management API (`/jmap/`, the `urn:stalwart:jmap` capability).
//!
//! One server per node, `bp-mail`, with its configuration and data in two volumes. The first
//! start is Stalwart's *bootstrap mode*: the agent starts it with a recovery administrator of its
//! own (a random password kept in `<state>/secrets/stalwart-recovery`, 0600), completes the
//! bootstrap through the API, keeps the permanent administrator the server makes
//! (`<state>/secrets/stalwart-admin`) and restarts it. Nothing here ever shows a password to
//! anybody: the administrator's secrets stay on the node, and a mailbox's password is sent to the
//! server and forgotten (the control plane keeps it sealed only until the node has it).
//!
//! What the control plane asks is typed: a domain, a mailbox, a list of forwarders, an automatic
//! reply, a limit; the agent turns each into the server's own objects (a Domain, an Account, a
//! MailingList, a VacationResponse, an MtaOutboundThrottle). Names that reach the server are
//! checked first (domain names, local parts), because some of them end up in expressions.

use std::{
    os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use builtpanel_docker_manager::{
    Capability, ContainerSpec, DockerError, Network, PublishedPort, Restart, Tmpfs, VolumeMount,
};
use builtpanel_domain::Fqdn;
use builtpanel_node_protocol::{
    AgentError, ErrorCode, MAX_MAIL_QUEUE_ITEMS, MailDnsRecord, MailForwarder, MailQueueItem,
    MailUsage, Operation, Output,
};
use builtpanel_tenant_stack::{Allowlist, Role};
use serde_json::{Value, json};

use super::{
    db::{name, secret_file},
    tenant::{TenantContext, docker_error, pull_with_fallback},
};

/// The mail server's container.
pub const CONTAINER: &str = "bp-mail";
pub(super) const NETWORK: &str = "bp-mail-net";
const ETC_VOLUME: &str = "bp-mail-etc";
const DATA_VOLUME: &str = "bp-mail-data";
/// The loopback port of the management API unless `BUILTPANEL_AGENT_MAIL_API_PORT` says another.
pub const DEFAULT_API_PORT: u16 = 8089;
const READY_TIMEOUT: Duration = Duration::from_secs(180);
const JMAP: [&str; 2] = ["urn:ietf:params:jmap:core", "urn:stalwart:jmap"];
const VACATION: &str = "urn:ietf:params:jmap:vacationresponse";
const MIB: u64 = 1024 * 1024;

fn invalid(message: &str) -> AgentError {
    AgentError::new(ErrorCode::InvalidArguments, message.to_owned())
}

fn failed(message: impl Into<String>) -> AgentError {
    AgentError::new(ErrorCode::Internal, message.into())
}

fn env_port(var: &str, default: u16) -> u16 {
    std::env::var(var)
        .ok()
        .and_then(|p| p.parse().ok())
        .filter(|p| *p >= 1)
        .unwrap_or(default)
}

/// The loopback port of the management API.
#[must_use]
pub fn api_port() -> u16 {
    env_port("BUILTPANEL_AGENT_MAIL_API_PORT", DEFAULT_API_PORT)
}

/// The ports the mail server is published on for the world: SMTP (25), submission over TLS (465)
/// and IMAP over TLS (993), or what `BUILTPANEL_AGENT_MAIL_SMTP_PORT`, `..._SUBMISSION_PORT` and
/// `..._IMAP_PORT` say (a node whose Docker cannot bind the low ports, and tests).
#[must_use]
pub fn public_ports() -> [(u16, u16); 3] {
    [
        (25, env_port("BUILTPANEL_AGENT_MAIL_SMTP_PORT", 25)),
        (465, env_port("BUILTPANEL_AGENT_MAIL_SUBMISSION_PORT", 465)),
        (993, env_port("BUILTPANEL_AGENT_MAIL_IMAP_PORT", 993)),
    ]
}

// ------------------------------------------------------------------ the management API

/// A login at the management API.
#[derive(Clone)]
struct Jmap {
    port: u16,
    auth: String,
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(TABLE[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

impl Jmap {
    fn new(user: &str, secret: &str) -> Self {
        Self {
            port: api_port(),
            auth: format!("Basic {}", base64(format!("{user}:{secret}").as_bytes())),
        }
    }

    /// One request of method calls; the answers, in order.
    async fn call(&self, calls: Vec<Value>) -> Result<Vec<Value>, AgentError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let body = json!({
                "using": [JMAP[0], JMAP[1], "urn:ietf:params:jmap:mail", VACATION],
                "methodCalls": calls,
            });
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(60)))
                .build()
                .into();
            let mut response = agent
                .post(&format!("http://127.0.0.1:{}/jmap/", this.port))
                .header("Authorization", &this.auth)
                .send_json(&body)
                .map_err(|_| failed("the mail server is not reachable").retryable())?;
            if response.status().as_u16() == 401 {
                return Err(failed("the mail server refused the agent's login"));
            }
            if !response.status().is_success() {
                return Err(failed("the mail server did not answer").retryable());
            }
            let answer: Value = response
                .body_mut()
                .read_json()
                .map_err(|_| failed("the mail server's answer is not understood"))?;
            Ok(answer["methodResponses"]
                .as_array()
                .cloned()
                .unwrap_or_default())
        })
        .await
        .map_err(|_| failed("the call was cancelled").retryable())?
    }

    /// One method call; its arguments, or the server's error as an [`AgentError`].
    async fn one(&self, method: &str, args: Value) -> Result<Value, AgentError> {
        let answers = self.call(vec![json!([method, args, "c1"])]).await?;
        let first = answers
            .into_iter()
            .next()
            .ok_or_else(|| failed("the mail server gave no answer"))?;
        let name = first[0].as_str().unwrap_or_default();
        if name == "error" {
            let kind = first[1]["type"].as_str().unwrap_or("error");
            return Err(failed(format!("the mail server refused {method} ({kind})")));
        }
        Ok(first[1].clone())
    }

    /// The same, for a `set`: what it did not make, change or remove is an error that names the
    /// server's reason (a short, fixed phrase of the server, never a value we sent).
    async fn set(&self, method: &str, args: Value) -> Result<Value, AgentError> {
        let answer = self.one(method, args).await?;
        for key in ["notCreated", "notUpdated", "notDestroyed"] {
            if let Some((_, why)) = answer[key].as_object().and_then(|o| o.iter().next()) {
                let kind = why["type"].as_str().unwrap_or("error");
                let what = why["description"].as_str().unwrap_or_default();
                let properties = why["properties"]
                    .as_array()
                    .map(|p| {
                        p.iter()
                            .filter_map(Value::as_str)
                            .take(3)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                return Err(failed(format!(
                    "the mail server refused the change ({kind}: {} {properties})",
                    what.chars().take(80).collect::<String>()
                )));
            }
        }
        Ok(answer)
    }

    /// The ids of the objects of a type that match a filter.
    async fn ids(&self, object: &str, filter: Value) -> Result<Vec<String>, AgentError> {
        let answer = self
            .one(
                &format!("x:{object}/query"),
                json!({ "filter": filter, "limit": 1000 }),
            )
            .await?;
        Ok(answer["ids"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// The objects with these ids.
    async fn get(&self, object: &str, ids: &[String]) -> Result<Vec<Value>, AgentError> {
        let mut out = Vec::new();
        for chunk in ids.chunks(200) {
            let answer = self
                .one(&format!("x:{object}/get"), json!({ "ids": chunk }))
                .await?;
            out.extend(answer["list"].as_array().cloned().unwrap_or_default());
        }
        Ok(out)
    }

    /// The id of a domain, by its exact name.
    async fn domain_id(&self, domain: &str) -> Result<Option<String>, AgentError> {
        let ids = self.ids("Domain", json!({ "name": domain })).await?;
        Ok(self
            .get("Domain", &ids)
            .await?
            .into_iter()
            .find(|d| d["name"] == domain)
            .and_then(|d| d["id"].as_str().map(str::to_owned)))
    }

    /// The id of a mailbox, by its local part and its domain's id.
    async fn account_id(&self, local: &str, domain_id: &str) -> Result<Option<String>, AgentError> {
        let ids = self
            .ids("Account", json!({ "name": local, "domainId": domain_id }))
            .await?;
        Ok(self
            .get("Account", &ids)
            .await?
            .into_iter()
            .find(|a| a["name"] == local && a["domainId"] == domain_id)
            .and_then(|a| a["id"].as_str().map(str::to_owned)))
    }
}

// ------------------------------------------------------------------ names

fn domain_of(text: &str) -> Result<String, AgentError> {
    Fqdn::parse(text)
        .map(|d| d.as_str().to_owned())
        .map_err(|e| invalid(&e.to_string()))
}

/// The local part of an address: letters, digits and `._-`, as the control plane allows.
fn local_of(text: &str) -> Result<String, AgentError> {
    let ok = !text.is_empty()
        && text.len() <= 64
        && text.starts_with(|c: char| c.is_ascii_alphanumeric())
        && text.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        });
    if ok {
        Ok(text.to_owned())
    } else {
        Err(invalid(
            "a mailbox name is lower-case letters, digits and . _ -",
        ))
    }
}

pub(super) fn address_of(text: &str) -> Result<String, AgentError> {
    let (local, domain) = text
        .split_once('@')
        .ok_or_else(|| invalid("an address needs an @"))?;
    Ok(format!("{}@{}", local_of(local)?, domain_of(domain)?))
}

// ------------------------------------------------------------------ the server

fn mail_hostname_ok(text: &str) -> Result<String, AgentError> {
    domain_of(text)
}

/// The administrator's name and secret, from the secret store: what a webmail single sign-on
/// signs in with, as `address%name`. `None` when the mail server was never set up here.
pub(super) fn admin_credentials(context: &TenantContext) -> Option<(String, String)> {
    let file = super::db::secrets_dir(context).join("stalwart-admin");
    let text = std::fs::read_to_string(file).ok()?;
    text.trim()
        .split_once(':')
        .map(|(user, secret)| (user.to_owned(), secret.to_owned()))
}

/// The administrator's login, from the secret store.
fn admin_login(context: &TenantContext) -> Result<Option<Jmap>, AgentError> {
    let file = super::db::secrets_dir(context).join("stalwart-admin");
    let Ok(text) = std::fs::read_to_string(file) else {
        return Ok(None);
    };
    Ok(text
        .trim()
        .split_once(':')
        .map(|(user, secret)| Jmap::new(user, secret)))
}

fn keep_admin(context: &TenantContext, user: &str, secret: &str) -> Result<(), AgentError> {
    let dir = super::db::secrets_dir(context);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(|e| failed(format!("cannot prepare the secret store: {}", e.kind())))?;
    let tmp = dir.join(".stalwart-admin.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| failed(format!("cannot keep the mail administrator: {}", e.kind())))?;
    std::io::Write::write_all(&mut out, format!("{user}:{secret}").as_bytes())
        .and_then(|()| out.sync_all())
        .map_err(|e| failed(format!("cannot keep the mail administrator: {}", e.kind())))?;
    std::fs::rename(&tmp, dir.join("stalwart-admin"))
        .map_err(|e| failed(format!("cannot keep the mail administrator: {}", e.kind())))
}

async fn wait_ready(jmap: &Jmap) -> Result<(), AgentError> {
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    loop {
        match jmap.one("x:Domain/query", json!({"limit": 1})).await {
            Ok(_) => return Ok(()),
            // Bootstrap mode answers every other object with an error; that is "up" too.
            Err(e) if e.message.contains("refused") => return Ok(()),
            Err(e) if !e.retryable && !e.message.contains("not understood") => return Err(e),
            Err(_) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(failed("the mail server did not start in time").retryable());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn restart(context: &TenantContext) -> Result<(), AgentError> {
    let container = name(CONTAINER)?;
    context
        .docker
        .stop(&container, Duration::from_secs(30))
        .await
        .map_err(|e| docker_error(&e))?;
    context
        .docker
        .start(&container)
        .await
        .map_err(|e| docker_error(&e))
}

/// A self-signed certificate for a name, until there is a real one.
fn self_signed(hostname: &str) -> Result<(String, String), AgentError> {
    let make = || -> Result<(String, String), rcgen::Error> {
        let mut params = rcgen::CertificateParams::new(vec![hostname.to_owned()])?;
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, hostname);
        let key = rcgen::KeyPair::generate()?;
        let cert = params.self_signed(&key)?;
        Ok((cert.pem(), key.serialize_pem()))
    };
    make().map_err(|_| failed("cannot make a certificate"))
}

/// Puts a certificate (and its key) on the server, replacing the ones it had.
async fn install_certificate(jmap: &Jmap, cert: &str, key: &str) -> Result<(), AgentError> {
    let old = jmap.ids("Certificate", json!({})).await?;
    if !old.is_empty() {
        jmap.set("x:Certificate/set", json!({ "destroy": old }))
            .await?;
    }
    jmap.set(
        "x:Certificate/set",
        json!({ "create": { "k": {
            "certificate": { "@type": "Text", "value": cert },
            "privateKey": { "@type": "Text", "secret": key },
        } } }),
    )
    .await?;
    Ok(())
}

/// The standard test of a spam filter: a message with this line in it must be called spam.
const GTUBE: &str = "XJS*C4JDBQADN1.NSBN3*2IDNEN*GTUBE-STANDARD-ANTI-UBE-TEST-EMAIL*C.34X";
const GTUBE_RULE: &str = "BUILTPANEL_GTUBE";

/// The spam filter knows the GTUBE test message (the server's own rule set does not have it): a
/// rule that gives it the tag, and a score for the tag that settles it.
async fn ensure_spam_rules(jmap: &Jmap) -> Result<(), AgentError> {
    let ids = jmap.ids("SpamRule", json!({})).await?;
    let have = jmap
        .get("SpamRule", &ids)
        .await?
        .into_iter()
        .any(|r| r["name"] == GTUBE_RULE);
    if !have {
        jmap.set(
            "x:SpamRule/set",
            json!({ "create": { "r": {
                "@type": "Any",
                "name": GTUBE_RULE,
                "enable": true,
                "priority": 1,
                "condition": { "match": { "0": {
                    "if": format!("contains(body.text, '{GTUBE}')"),
                    "then": "'GTUBE'",
                } }, "else": "false" },
            } } }),
        )
        .await?;
    }
    let tags = jmap.ids("SpamTag", json!({})).await?;
    let scored = jmap
        .get("SpamTag", &tags)
        .await?
        .into_iter()
        .any(|t| t["tag"] == "GTUBE");
    if !scored {
        jmap.set(
            "x:SpamTag/set",
            json!({ "create": { "t": { "@type": "Score", "tag": "GTUBE", "score": 100.0 } } }),
        )
        .await?;
    }
    Ok(())
}

/// Makes sure the server runs, is set up, and has a certificate. Returns whether this call made
/// the container.
///
/// # Errors
/// Docker, the disk or the server failing (retryable), a bad name (not).
pub async fn ensure_server(
    context: &TenantContext,
    hostname: &str,
    certificate: Option<(&str, &str)>,
) -> Result<bool, AgentError> {
    let hostname = mail_hostname_ok(hostname)?;
    let docker = &context.docker;
    let container = name(CONTAINER)?;
    let recovery = secret_file(&super::db::secrets_dir(context), "stalwart-recovery")?;
    let created = match docker.inspect(&container).await {
        Ok(_) => false,
        Err(DockerError::NotFound) => {
            let allowlist = Allowlist::embedded()
                .map_err(|e| failed(format!("the image list is broken: {e}")))?;
            let image = allowlist
                .current(Role::Mail)
                .map_err(|e| failed(e.to_string()))?
                .clone();
            let network = name(NETWORK)?;
            docker
                .ensure_network(&network, false)
                .await
                .map_err(|e| docker_error(&e))?;
            let (etc, data) = (name(ETC_VOLUME)?, name(DATA_VOLUME)?);
            for v in [&etc, &data] {
                docker
                    .create_volume(v)
                    .await
                    .map_err(|e| docker_error(&e))?;
            }
            let image = pull_with_fallback(docker, &allowlist, &image).await?;
            // The server runs as its own user (2000), and a new volume belongs to root: made
            // over once, by a short job of the same image.
            let mut prepare = ContainerSpec::new(name("bp-mail-prepare")?, image.clone());
            prepare.entrypoint = vec![
                "chown".into(),
                "-R".into(),
                "2000:2000".into(),
                "/etc/stalwart".into(),
                "/var/lib/stalwart".into(),
            ];
            prepare.user = Some((0, 0));
            prepare.cap_add = vec![
                Capability::Chown,
                Capability::DacOverride,
                Capability::Fowner,
            ];
            prepare.volumes = vec![
                VolumeMount {
                    volume: name(ETC_VOLUME)?,
                    target: "/etc/stalwart".into(),
                    read_only: false,
                },
                VolumeMount {
                    volume: name(DATA_VOLUME)?,
                    target: "/var/lib/stalwart".into(),
                    read_only: false,
                },
            ];
            let _ = docker.remove(&name("bp-mail-prepare")?, true).await;
            let done = docker
                .run_to_end(&prepare, Duration::from_secs(60))
                .await
                .map_err(|e| docker_error(&e))?;
            if done.exit_code != 0 {
                return Err(failed("cannot give the mail server's folders to its user"));
            }
            let mut spec = ContainerSpec::new(container.clone(), image);
            spec.env.insert(
                "STALWART_RECOVERY_ADMIN".into(),
                format!("recovery:{recovery}"),
            );
            // The server's program carries the right to bind low ports as a file capability: the
            // container has to hold that capability or the program cannot start.
            spec.cap_add = vec![Capability::NetBindService];
            spec.memory_bytes = 1024 * MIB;
            spec.cpu_millis = 2000;
            spec.pids_limit = 512;
            spec.tmpfs = vec![Tmpfs {
                target: "/tmp".into(),
                size_bytes: 256 * MIB,
            }];
            spec.volumes = vec![
                VolumeMount {
                    volume: etc,
                    target: "/etc/stalwart".into(),
                    read_only: false,
                },
                VolumeMount {
                    volume: data,
                    target: "/var/lib/stalwart".into(),
                    read_only: false,
                },
            ];
            spec.network = Network::Named(network);
            let bad = || failed("bad address");
            let mut ports = vec![PublishedPort {
                container_port: 8080,
                host_port: api_port(),
                host_ip: "127.0.0.1".parse().map_err(|_| bad())?,
            }];
            for (container_port, host_port) in public_ports() {
                ports.push(PublishedPort {
                    container_port,
                    host_port,
                    host_ip: "0.0.0.0".parse().map_err(|_| bad())?,
                });
            }
            spec.ports = ports;
            spec.restart = Restart::UnlessStopped;
            match docker.create_container(&spec).await {
                Ok(_) | Err(DockerError::Conflict(_)) => {}
                Err(e) => return Err(docker_error(&e)),
            }
            true
        }
        Err(e) => return Err(docker_error(&e)),
    };
    docker
        .start(&container)
        .await
        .map_err(|e| docker_error(&e))?;
    // First time: the server is in bootstrap mode; the agent finishes it and keeps the
    // administrator the server makes.
    let admin = if let Some(admin) = admin_login(context)? {
        wait_ready(&admin).await?;
        admin
    } else {
        let boot = Jmap::new("recovery", &recovery);
        wait_ready(&boot).await?;
        let domain = hostname
            .split_once('.')
            .map_or(hostname.as_str(), |(_, rest)| rest);
        let done = boot
            .set(
                "x:Bootstrap/set",
                json!({ "update": { "singleton": {
                    "defaultHostname": hostname,
                    "defaultDomain": domain,
                    "requestTlsCertificate": false,
                    "generateDkimKeys": true,
                    "tracer": { "@type": "Stdout" },
                } } }),
            )
            .await?;
        let made = &done["updated"]["singleton"];
        let (user, secret) = (
            made["username"]
                .as_str()
                .ok_or_else(|| failed("the bootstrap gave no administrator"))?,
            made["secret"]
                .as_str()
                .ok_or_else(|| failed("the bootstrap gave no administrator"))?,
        );
        keep_admin(context, user, secret)?;
        restart(context).await?;
        let admin = Jmap::new(user, secret);
        wait_ready(&admin).await?;
        admin
    };
    ensure_spam_rules(&admin).await?;
    // The certificate: the one given, or a self-signed one when there is none at all.
    let have = admin.ids("Certificate", json!({})).await?;
    let wanted = match certificate {
        Some((cert, key)) => Some((cert.to_owned(), key.to_owned())),
        None if have.is_empty() => Some(self_signed(&hostname)?),
        None => None,
    };
    if let Some((cert, key)) = wanted {
        install_certificate(&admin, &cert, &key).await?;
        restart(context).await?;
        wait_ready(&admin).await?;
    }
    Ok(created)
}

/// The login of an administrator whose server is set up.
fn login(context: &TenantContext) -> Result<Jmap, AgentError> {
    admin_login(context)?
        .ok_or_else(|| failed("the mail server is not set up on this node yet").retryable())
}

/// `ensure_mail_server`.
///
/// # Errors
/// As [`ensure_server`].
pub async fn ensure_mail_server(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::EnsureMailServer {
        hostname,
        certificate_pem,
        private_key_pem,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let pair = match (&certificate_pem, &private_key_pem) {
        (Some(c), Some(k)) => Some((c.as_str(), k.0.as_str())),
        (None, None) => None,
        _ => {
            return Err(invalid(
                "a certificate needs its key and the other way round",
            ));
        }
    };
    let created = ensure_server(&context, &hostname, pair).await?;
    Ok(Output::MailServerReady { created })
}

// ------------------------------------------------------------------ domains

/// The records of a domain's zone file that mail needs: MX, SPF, DKIM, DMARC.
fn dns_records(zone: &str, domain: &str) -> Vec<MailDnsRecord> {
    // Joins what a zone file continues in parentheses into one line.
    let mut lines: Vec<String> = Vec::new();
    let mut open: Option<String> = None;
    for raw in zone.lines() {
        match open.take() {
            Some(mut line) => {
                line.push(' ');
                line.push_str(raw.trim());
                if raw.contains(')') {
                    lines.push(line);
                } else {
                    open = Some(line);
                }
            }
            None if raw.contains('(') && !raw.contains(')') => open = Some(raw.to_owned()),
            None => lines.push(raw.to_owned()),
        }
    }
    let suffix = format!(".{domain}.");
    let apex = format!("{domain}.");
    let mut out = Vec::new();
    for line in lines {
        let line = line.replace(['(', ')'], " ");
        let mut parts = line.split_whitespace();
        let (Some(owner), Some("IN"), Some(rtype)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let rest: Vec<&str> = parts.collect();
        let relative = if owner == apex {
            "@".to_owned()
        } else if let Some(r) = owner.strip_suffix(&suffix) {
            r.to_owned()
        } else {
            continue;
        };
        match rtype {
            "MX" => {
                let (Some(priority), Some(target)) = (rest.first(), rest.get(1)) else {
                    continue;
                };
                out.push(MailDnsRecord {
                    name: relative,
                    rtype: "MX".into(),
                    content: target.trim_end_matches('.').to_owned(),
                    priority: priority.parse().ok(),
                });
            }
            "TXT" => {
                // Quoted pieces, joined (a long key is split in several).
                let text = rest.join(" ");
                let content: String = text
                    .split('"')
                    .enumerate()
                    .filter(|(i, _)| i % 2 == 1)
                    .map(|(_, piece)| piece)
                    .collect();
                let wanted = content.starts_with("v=spf1")
                    || content.starts_with("v=DKIM1")
                    || content.starts_with("v=DMARC1");
                if wanted {
                    out.push(MailDnsRecord {
                        name: relative,
                        rtype: "TXT".into(),
                        content,
                        priority: None,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

fn throttle_description(domain: &str) -> String {
    format!("builtpanel:{domain}")
}

/// Sets, changes or removes the domain's outgoing limit.
async fn set_throttle(
    jmap: &Jmap,
    domain: &str,
    per_hour: Option<u32>,
    held: bool,
) -> Result<(), AgentError> {
    let description = throttle_description(domain);
    let ids = jmap.ids("MtaOutboundThrottle", json!({})).await?;
    let mine: Vec<String> = jmap
        .get("MtaOutboundThrottle", &ids)
        .await?
        .into_iter()
        .filter(|t| t["description"] == description)
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .collect();
    // A held domain may send one message a day; otherwise the limit, if there is one.
    let rate = match (held, per_hour) {
        (true, _) => Some((1_u32, 24 * 3_600_000_u64)),
        (false, Some(n)) => Some((n.clamp(1, 1_000_000), 3_600_000_u64)),
        (false, None) => None,
    };
    match (rate, mine.first()) {
        (None, _) => {
            if !mine.is_empty() {
                jmap.set("x:MtaOutboundThrottle/set", json!({ "destroy": mine }))
                    .await?;
            }
        }
        (Some((count, period)), existing) => {
            let object = json!({
                "description": description,
                "enable": true,
                "key": { "senderDomain": true },
                // The domain was checked as a plain domain name, so nothing in it can end the string.
                "match": { "else": format!("sender_domain == '{domain}'") },
                "rate": { "count": count, "period": period },
            });
            match existing {
                Some(id) => {
                    jmap.set(
                        "x:MtaOutboundThrottle/set",
                        json!({ "update": { id: object } }),
                    )
                    .await?;
                }
                None => {
                    jmap.set(
                        "x:MtaOutboundThrottle/set",
                        json!({ "create": { "t": object } }),
                    )
                    .await?;
                }
            }
        }
    }
    Ok(())
}

/// `set_mail_domain`.
///
/// # Errors
/// [`AgentError`]: bad names (not retryable); the server failing.
pub async fn set_mail_domain(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SetMailDomain {
        domain,
        catch_all,
        per_hour,
        held,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let domain = domain_of(&domain)?;
    let catch_all = catch_all.as_deref().map(address_of).transpose()?;
    let jmap = login(&context)?;
    let id = match jmap.domain_id(&domain).await? {
        Some(id) => {
            jmap.set(
                "x:Domain/set",
                json!({ "update": { id.clone(): { "catchAllAddress": catch_all } } }),
            )
            .await?;
            id
        }
        None => {
            let made = jmap
                .set(
                    "x:Domain/set",
                    json!({ "create": { "d": {
                        "name": domain,
                        "catchAllAddress": catch_all,
                    } } }),
                )
                .await?;
            made["created"]["d"]["id"]
                .as_str()
                .ok_or_else(|| failed("the mail server did not say what it made"))?
                .to_owned()
        }
    };
    set_throttle(&jmap, &domain, per_hour, held).await?;
    // The server makes the domain's two DKIM keys a moment after the domain: wait for both.
    let mut records = Vec::new();
    for attempt in 0..30 {
        let zone = jmap
            .get("Domain", std::slice::from_ref(&id))
            .await?
            .into_iter()
            .next()
            .and_then(|d| d["dnsZoneFile"].as_str().map(str::to_owned))
            .unwrap_or_default();
        records = dns_records(&zone, &domain);
        let keys = records
            .iter()
            .filter(|r| r.content.starts_with("v=DKIM1"))
            .count();
        if keys >= 2 || attempt == 29 {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Ok(Output::MailDomainSet { records })
}

/// `remove_mail_domain`.
///
/// # Errors
/// As [`set_mail_domain`].
pub async fn remove_mail_domain(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::RemoveMailDomain { domain } = op else {
        return Err(super::wrong_operation());
    };
    let domain = domain_of(&domain)?;
    let jmap = login(&context)?;
    let Some(id) = jmap.domain_id(&domain).await? else {
        set_throttle(&jmap, &domain, None, false).await?;
        return Ok(Output::MailDomainRemoved { existed: false });
    };
    // Its mailboxes and lists first: the server will not drop a domain that has them.
    let accounts = jmap.ids("Account", json!({ "domainId": id })).await?;
    if !accounts.is_empty() {
        jmap.set("x:Account/set", json!({ "destroy": accounts }))
            .await?;
    }
    let lists: Vec<String> = lists_of(&jmap, &id)
        .await?
        .iter()
        .filter_map(|l| l["id"].as_str().map(str::to_owned))
        .collect();
    if !lists.is_empty() {
        jmap.set("x:MailingList/set", json!({ "destroy": lists }))
            .await?;
    }
    set_throttle(&jmap, &domain, None, false).await?;
    // The domain's DKIM keys (made by the server) are linked to it.
    let key_ids = jmap.ids("DkimSignature", json!({})).await?;
    let keys: Vec<String> = jmap
        .get("DkimSignature", &key_ids)
        .await?
        .into_iter()
        .filter(|k| k["domainId"] == id.as_str())
        .filter_map(|k| k["id"].as_str().map(str::to_owned))
        .collect();
    if !keys.is_empty() {
        jmap.set("x:DkimSignature/set", json!({ "destroy": keys }))
            .await?;
    }
    jmap.set("x:Domain/set", json!({ "destroy": [id] })).await?;
    Ok(Output::MailDomainRemoved { existed: true })
}

// ------------------------------------------------------------------ mailboxes

/// `upsert_mailbox`.
///
/// # Errors
/// [`AgentError`]: bad names, a new mailbox without a password, a domain the server does not
/// know (not retryable); the server failing.
pub async fn upsert_mailbox(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::UpsertMailbox {
        domain,
        local,
        password,
        quota_bytes,
        aliases,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let domain = domain_of(&domain)?;
    let local = local_of(&local)?;
    let mut alias_map = serde_json::Map::new();
    for (i, alias) in aliases.iter().enumerate().take(50) {
        alias_map.insert(i.to_string(), json!({ "name": local_of(alias)? }));
    }
    if let Some(p) = &password {
        builtpanel_domain::databases::check_password(&p.0).map_err(|e| invalid(&e.to_string()))?;
    }
    let jmap = login(&context)?;
    let domain_id = jmap
        .domain_id(&domain)
        .await?
        .ok_or_else(|| invalid("the mail server does not know this domain"))?;
    // Every alias needs the domain's id in its object.
    for v in alias_map.values_mut() {
        v["domainId"] = json!(domain_id);
    }
    let quotas = json!({ "maxDiskQuota": quota_bytes });
    match jmap.account_id(&local, &domain_id).await? {
        Some(id) => {
            let mut change = json!({ "quotas": quotas, "aliases": alias_map });
            if let Some(p) = &password {
                change["credentials"] = json!({ "0": { "@type": "Password", "secret": p.0 } });
            }
            jmap.set("x:Account/set", json!({ "update": { id: change } }))
                .await?;
            Ok(Output::MailboxSet { created: false })
        }
        None => {
            let p = password
                .ok_or_else(|| invalid("a password is needed for a mailbox that is new here"))?;
            jmap.set(
                "x:Account/set",
                json!({ "create": { "m": {
                    "@type": "User",
                    "name": local,
                    "domainId": domain_id,
                    "credentials": { "0": { "@type": "Password", "secret": p.0 } },
                    "quotas": quotas,
                    "aliases": alias_map,
                } } }),
            )
            .await?;
            Ok(Output::MailboxSet { created: true })
        }
    }
}

/// `drop_mailbox`.
///
/// # Errors
/// As [`upsert_mailbox`].
pub async fn drop_mailbox(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::DropMailbox { domain, local } = op else {
        return Err(super::wrong_operation());
    };
    let (domain, local) = (domain_of(&domain)?, local_of(&local)?);
    let jmap = login(&context)?;
    let Some(domain_id) = jmap.domain_id(&domain).await? else {
        return Ok(Output::MailboxDropped { existed: false });
    };
    match jmap.account_id(&local, &domain_id).await? {
        Some(id) => {
            jmap.set("x:Account/set", json!({ "destroy": [id] }))
                .await?;
            Ok(Output::MailboxDropped { existed: true })
        }
        None => Ok(Output::MailboxDropped { existed: false }),
    }
}

// ------------------------------------------------------------------ forwarders, replies

/// The lists (forwarders) of a domain: the server cannot filter lists by domain.
async fn lists_of(jmap: &Jmap, domain_id: &str) -> Result<Vec<Value>, AgentError> {
    let ids = jmap.ids("MailingList", json!({})).await?;
    Ok(jmap
        .get("MailingList", &ids)
        .await?
        .into_iter()
        .filter(|l| l["domainId"] == domain_id)
        .collect())
}

/// `set_mail_forwarders`.
///
/// # Errors
/// As [`upsert_mailbox`].
pub async fn set_mail_forwarders(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SetMailForwarders { domain, forwarders } = op else {
        return Err(super::wrong_operation());
    };
    let domain = domain_of(&domain)?;
    if forwarders.len() > 500 {
        return Err(invalid("too many forwarders"));
    }
    let mut wanted: Vec<(String, Vec<String>)> = Vec::new();
    for MailForwarder { local, to } in &forwarders {
        let local = local_of(local)?;
        let to: Result<Vec<String>, AgentError> = to.iter().map(|a| address_of(a)).collect();
        let to = to?;
        if to.is_empty() || to.len() > 20 {
            return Err(invalid("a forwarder sends to 1 to 20 addresses"));
        }
        wanted.push((local, to));
    }
    let jmap = login(&context)?;
    let domain_id = jmap
        .domain_id(&domain)
        .await?
        .ok_or_else(|| invalid("the mail server does not know this domain"))?;
    let have = lists_of(&jmap, &domain_id).await?;
    let mut destroy = Vec::new();
    for list in &have {
        let n = list["name"].as_str().unwrap_or_default();
        if !wanted.iter().any(|(local, _)| local == n)
            && let Some(id) = list["id"].as_str()
        {
            destroy.push(id.to_owned());
        }
    }
    if !destroy.is_empty() {
        jmap.set("x:MailingList/set", json!({ "destroy": destroy }))
            .await?;
    }
    for (local, to) in &wanted {
        let recipients: serde_json::Map<String, Value> =
            to.iter().map(|a| (a.clone(), json!(true))).collect();
        match have.iter().find(|l| l["name"] == local.as_str()) {
            Some(list) => {
                let id = list["id"].as_str().unwrap_or_default();
                jmap.set(
                    "x:MailingList/set",
                    json!({ "update": { id: { "recipients": recipients } } }),
                )
                .await?;
            }
            None => {
                jmap.set(
                    "x:MailingList/set",
                    json!({ "create": { "l": {
                        "name": local,
                        "domainId": domain_id,
                        "recipients": recipients,
                    } } }),
                )
                .await?;
            }
        }
    }
    Ok(Output::MailForwardersSet {
        count: wanted.len(),
    })
}

fn rfc3339(seconds: i64) -> Result<String, AgentError> {
    time::OffsetDateTime::from_unix_timestamp(seconds)
        .map_err(|_| invalid("a time is out of range"))?
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| invalid("a time is out of range"))
}

/// `set_mail_autoresponder`.
///
/// # Errors
/// As [`upsert_mailbox`].
pub async fn set_mail_autoresponder(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SetMailAutoresponder {
        domain,
        local,
        enabled,
        subject,
        body,
        from,
        until,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let (domain, local) = (domain_of(&domain)?, local_of(&local)?);
    if subject.len() > 200 || body.len() > 10_000 {
        return Err(invalid("the reply is too long"));
    }
    let jmap = login(&context)?;
    let domain_id = jmap
        .domain_id(&domain)
        .await?
        .ok_or_else(|| invalid("the mail server does not know this domain"))?;
    let account = jmap
        .account_id(&local, &domain_id)
        .await?
        .ok_or_else(|| invalid("the mailbox does not exist here"))?;
    let from = from.map(rfc3339).transpose()?;
    let until = until.map(rfc3339).transpose()?;
    let answers = jmap
        .call(vec![json!([
            "VacationResponse/set",
            {
                "accountId": account,
                "update": { "singleton": {
                    "isEnabled": enabled,
                    "subject": subject,
                    "textBody": body,
                    "fromDate": from,
                    "toDate": until,
                } },
            },
            "c1"
        ])])
        .await?;
    let first = answers
        .into_iter()
        .next()
        .ok_or_else(|| failed("the mail server gave no answer"))?;
    if first[0] == "error"
        || first[1]["notUpdated"]
            .as_object()
            .is_some_and(|o| !o.is_empty())
    {
        return Err(failed("the mail server refused the automatic reply"));
    }
    Ok(Output::MailAutoresponderSet)
}

// ------------------------------------------------------------------ report and queue

fn seconds_of(text: &str) -> i64 {
    time::OffsetDateTime::parse(text, &time::format_description::well_known::Rfc3339)
        .map_or(0, time::OffsetDateTime::unix_timestamp)
}

/// `report_mail`.
///
/// # Errors
/// The server failing.
pub async fn report_mail(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::ReportMail = op else {
        return Err(super::wrong_operation());
    };
    let jmap = login(&context)?;
    let ids = jmap.ids("Account", json!({})).await?;
    let usage = jmap
        .get("Account", &ids)
        .await?
        .into_iter()
        .filter(|a| a["@type"] == "User")
        .filter_map(|a| {
            Some(MailUsage {
                address: a["emailAddress"].as_str()?.to_owned(),
                used_bytes: a["usedDiskQuota"].as_u64().unwrap_or(0),
            })
        })
        .collect();
    let queued = jmap.ids("QueuedMessage", json!({})).await?;
    let total = queued.len() as u64;
    let shown: Vec<String> = queued.into_iter().take(MAX_MAIL_QUEUE_ITEMS).collect();
    let queue = jmap
        .get("QueuedMessage", &shown)
        .await?
        .into_iter()
        .map(|m| {
            let recipients: Vec<String> = m["recipients"]
                .as_object()
                .map(|r| r.keys().cloned().collect())
                .unwrap_or_default();
            let tried = m["recipients"]
                .as_object()
                .into_iter()
                .flat_map(|r| r.values())
                .filter_map(|r| r["retryCount"].as_u64().or_else(|| r["retries"].as_u64()))
                .max()
                .unwrap_or(0);
            let error = m["recipients"]
                .as_object()
                .into_iter()
                .flat_map(|r| r.values())
                .find_map(|r| {
                    r["status"]["response"]
                        .as_str()
                        .or_else(|| r["status"]["reason"].as_str())
                        .or_else(|| r["message"].as_str())
                })
                .map(|e| e.chars().take(200).collect::<String>());
            MailQueueItem {
                id: m["id"].as_str().unwrap_or_default().to_owned(),
                sender: m["returnPath"].as_str().unwrap_or_default().to_owned(),
                recipients,
                status: if tried > 0 || error.is_some() {
                    "deferred".into()
                } else {
                    "queued".into()
                },
                error,
                queued_at: m["createdAt"].as_str().map_or(0, seconds_of),
                retries: u32::try_from(tried).unwrap_or(u32::MAX),
                size_bytes: m["size"].as_u64().unwrap_or(0),
            }
        })
        .collect();
    Ok(Output::MailReport {
        usage,
        queue,
        queue_total: total,
    })
}

/// `mail_queue_action`.
///
/// # Errors
/// A bad action or id (not retryable); the server failing.
pub async fn mail_queue_action(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::MailQueueAction { action, ids } = op else {
        return Err(super::wrong_operation());
    };
    if ids.len() > MAX_MAIL_QUEUE_ITEMS
        || ids
            .iter()
            .any(|i| i.is_empty() || i.len() > 40 || !i.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return Err(invalid("a message id is letters and digits"));
    }
    let jmap = login(&context)?;
    let ids = if ids.is_empty() && action == "flush" {
        jmap.ids("QueuedMessage", json!({})).await?
    } else {
        ids
    };
    match action.as_str() {
        "delete" => {
            // A message that is being delivered this very moment cannot be removed; it is
            // asked again a few times, and the ones that are still busy are left.
            let mut left = ids.clone();
            for attempt in 0..6 {
                if left.is_empty() {
                    break;
                }
                let answer = jmap
                    .one("x:QueuedMessage/set", json!({ "destroy": left }))
                    .await?;
                let busy: Vec<String> = answer["notDestroyed"]
                    .as_object()
                    .map(|o| o.keys().cloned().collect())
                    .unwrap_or_default();
                left = busy;
                if !left.is_empty() && attempt < 5 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
            Ok(Output::MailQueueChanged {
                count: ids.len() - left.len(),
            })
        }
        "flush" => {
            let now = rfc3339(
                i64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs()),
                )
                .unwrap_or(0),
            )?;
            let update: serde_json::Map<String, Value> = ids
                .iter()
                .map(|id| (id.clone(), json!({ "nextRetry": now })))
                .collect();
            // A message that is being delivered right now cannot be touched: it is skipped.
            let mut done = 0;
            if !update.is_empty() {
                let answer = jmap
                    .one("x:QueuedMessage/set", json!({ "update": update }))
                    .await?;
                done = answer["updated"]
                    .as_object()
                    .map_or(0, serde_json::Map::len);
            }
            Ok(Output::MailQueueChanged { count: done })
        }
        _ => Err(invalid("the action is flush or delete")),
    }
}

/// Where the mail server's secrets are (for tests).
#[must_use]
pub fn secrets(context: &TenantContext) -> PathBuf {
    super::db::secrets_dir(context)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(
            base64(b"admin@example.com:KMTUaTmmO4plmMQo"),
            "YWRtaW5AZXhhbXBsZS5jb206S01UVWFUbW1PNHBsbU1Rbw=="
        );
    }

    #[test]
    fn names_that_reach_the_server_are_plain() {
        assert_eq!(local_of("info.sales-1").unwrap(), "info.sales-1");
        for bad in [
            "",
            "Info",
            "a b",
            "a'b",
            "a@b",
            "-a",
            "a\"",
            &"x".repeat(65),
        ] {
            assert!(local_of(bad).is_err(), "{bad}");
        }
        assert_eq!(address_of("info@example.com").unwrap(), "info@example.com");
        for bad in [
            "info",
            "info@",
            "@example.com",
            "in'fo@example.com",
            "info@exa mple.com",
        ] {
            assert!(address_of(bad).is_err(), "{bad}");
        }
        assert!(domain_of("clientmail.org").is_ok());
        assert!(domain_of("x' || true || '").is_err());
    }

    #[test]
    fn the_records_mail_needs_are_taken_out_of_a_zone_file() {
        let zone = "v1-ed25519-20261117._domainkey.clientmail.org. IN TXT \"v=DKIM1; k=ed25519; h=sha256; p=AAAA\"\n\
v1-rsa-20261117._domainkey.clientmail.org. IN TXT (\n    \"v=DKIM1; k=rsa; h=sha256; p=MIIB\"\n    \"CDEF\"\n)\n\
clientmail.org. IN TXT \"v=spf1 mx -all\"\n\
clientmail.org. IN MX 10 mail.example.com.\n\
_dmarc.clientmail.org. IN TXT \"v=DMARC1; p=reject; rua=mailto:postmaster@clientmail.org\"\n\
_imaps._tcp.clientmail.org. IN SRV 0 1 993 mail.example.com.\n\
mta-sts.clientmail.org. IN CNAME mail.example.com.\n\
_mta-sts.clientmail.org. IN TXT \"v=STSv1; id=1\"\n\
other.example. IN TXT \"v=spf1 -all\"\n";
        let records = dns_records(zone, "clientmail.org");
        let find = |name: &str| {
            records
                .iter()
                .filter(|r| r.name == name)
                .collect::<Vec<_>>()
        };
        assert_eq!(records.len(), 5, "{records:?}");
        assert_eq!(find("@").len(), 2);
        let mx = records.iter().find(|r| r.rtype == "MX").unwrap();
        assert_eq!(
            (mx.content.as_str(), mx.priority),
            ("mail.example.com", Some(10))
        );
        let spf = records
            .iter()
            .find(|r| r.content.starts_with("v=spf1"))
            .unwrap();
        assert_eq!(spf.content, "v=spf1 mx -all");
        assert_eq!(
            find("v1-rsa-20261117._domainkey")[0].content,
            "v=DKIM1; k=rsa; h=sha256; p=MIIBCDEF"
        );
        assert!(find("_dmarc")[0].content.starts_with("v=DMARC1; p=reject"));
    }

    #[test]
    fn a_time_becomes_the_text_the_server_reads() {
        assert_eq!(rfc3339(1_700_000_000).unwrap(), "2023-11-14T22:13:20Z");
        assert_eq!(seconds_of("2023-11-14T22:13:20Z"), 1_700_000_000);
    }
}
