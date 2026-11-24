//! The dry run (T15.04): what an import would do, and what stands in its way.
//!
//! [`plan`] is a pure function of the model, the names that already exist on the panel and
//! the operator's choices. Nothing is written. The answer lists the steps in the order they
//! would run, every conflict with the way it was settled, what was left out, and the model
//! with the new names applied: the executor runs exactly that model, so the dry run and the
//! real import cannot disagree.

use std::collections::BTreeSet;

use builtpanel_domain::{Fqdn, TenantUsername, databases::full_name};
use serde::{Deserialize, Serialize};

use crate::model::{DomainKind, Engine, ImportModel, Warning};

/// What to do when a name is already taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnConflict {
    /// Stop: nothing is imported until the operator decides.
    #[default]
    Fail,
    /// Take a free name (`shop` becomes `shop2`) where a name may change; a domain cannot
    /// change, so a domain in the way still stops the import.
    Rename,
    /// Leave the conflicting item out and import the rest.
    Skip,
}

/// What to bring over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Include {
    /// The account's files.
    pub files: bool,
    /// Databases and their users.
    pub databases: bool,
    /// DNS zones.
    pub dns: bool,
    /// Scheduled jobs.
    pub cron: bool,
    /// Mailboxes and forwarders.
    pub mail: bool,
}

impl Default for Include {
    fn default() -> Self {
        Self {
            files: true,
            databases: true,
            dns: true,
            cron: true,
            mail: true,
        }
    }
}

/// The operator's choices.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    /// The account's name here; the old name, made valid, when not given.
    pub username: Option<String>,
    /// The main domain here; the old one when not given.
    pub main_domain: Option<String>,
    /// How to settle conflicts.
    pub on_conflict: OnConflict,
    /// What to bring over.
    pub include: Include,
    /// Whether this panel can host mail (the mail module is on).
    pub mail_supported: bool,
}

/// What the panel already has.
#[derive(Debug, Clone, Default)]
pub struct Existing {
    /// Login names and hosting-account names in use.
    pub usernames: BTreeSet<String>,
    /// Domains in use, anywhere on the panel.
    pub domains: BTreeSet<String>,
    /// Database and database-user names in use.
    pub db_names: BTreeSet<String>,
}

/// One thing the import does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Make the account and its hosting account.
    CreateAccount,
    /// Put the files in place.
    RestoreFiles,
    /// Add a domain.
    AddDomain,
    /// Make a database.
    CreateDatabase,
    /// Load a dump into it.
    ImportDump,
    /// Make a database user and its grants.
    CreateDbUser,
    /// Make a DNS zone with its records.
    CreateDnsZone,
    /// Add a scheduled job.
    AddCronJob,
    /// Make a mailbox.
    CreateMailbox,
    /// Make a forwarder.
    CreateForwarder,
}

/// A step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// What it does.
    pub action: Action,
    /// To what, by its new name.
    pub subject: String,
    /// Its name before, when it changed.
    pub renamed_from: Option<String>,
    /// Bytes to move, when that is known.
    pub bytes: Option<u64>,
}

/// How a conflict was settled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// The item took another name.
    Renamed {
        /// The name it took.
        to: String,
    },
    /// The item is left out.
    Skipped,
    /// Nothing can be imported until the operator decides.
    Blocks,
}

/// A name that was already taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    /// `username`, `domain`, `database` or `db_user`.
    pub kind: String,
    /// The name as the backup has it (after the prefix rule for databases).
    pub name: String,
    /// What happens.
    pub resolution: Resolution,
}

/// Something left out and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skipped {
    /// What kind of thing.
    pub kind: String,
    /// Its name.
    pub name: String,
    /// Why.
    pub reason: String,
}

/// The dry run's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Whether the import can run: no blockers.
    pub ok: bool,
    /// Why it cannot.
    pub blockers: Vec<String>,
    /// The steps, in order.
    pub steps: Vec<Step>,
    /// Names that were taken.
    pub conflicts: Vec<Conflict>,
    /// Left out.
    pub skipped: Vec<Skipped>,
    /// What the reader could not bring over.
    pub warnings: Vec<Warning>,
    /// Bytes of files to move.
    pub file_bytes: u64,
    /// Bytes of database dumps to load.
    pub dump_bytes: u64,
    /// The model with the new names; the executor runs this one.
    pub model: ImportModel,
}

/// A valid account name from an old one: lower case letters and digits, starting with a
/// letter, at least 3 and at most 32 characters.
fn account_name(old: &str) -> String {
    let mut s: String = old
        .to_ascii_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    if !s.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        s.insert(0, 'u');
    }
    while s.len() < 3 {
        s.push('x');
    }
    s.truncate(32);
    s
}

fn free_account(base: &str, taken: &BTreeSet<String>) -> String {
    (2u32..)
        .map(|n| {
            let suffix = n.to_string();
            let keep = 32 - suffix.len();
            format!("{}{suffix}", &base[..base.len().min(keep)])
        })
        .find(|c| !taken.contains(c) && TenantUsername::parse(c).is_ok())
        .unwrap_or_else(|| base.to_owned())
}

/// A database or user name with the new account's prefix: an old prefix is replaced.
fn db_name(old_account: &str, new_account: &str, old: &str) -> Option<String> {
    let lower: String = old
        .to_ascii_lowercase()
        .chars()
        .filter_map(|c| match c {
            'a'..='z' | '0'..='9' | '_' => Some(c),
            '-' => Some('_'),
            _ => None,
        })
        .collect();
    let old_prefix = format!("{}_", account_name_lower(old_account));
    let own = lower.strip_prefix(&old_prefix).unwrap_or(&lower);
    let tenant = TenantUsername::parse(new_account).ok()?;
    full_name(&tenant, own, true).ok()
}

fn account_name_lower(s: &str) -> String {
    s.to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

fn free_db(name: &str, taken: &BTreeSet<String>) -> String {
    (2u32..)
        .map(|n| {
            let suffix = n.to_string();
            let keep = 64 - suffix.len();
            format!("{}{suffix}", &name[..name.len().min(keep)])
        })
        .find(|c| !taken.contains(c))
        .unwrap_or_else(|| name.to_owned())
}

/// Works out what importing `model` would do.
#[must_use]
pub fn plan(model: &ImportModel, existing: &Existing, options: &Options) -> Plan {
    let mut out = Plan {
        ok: true,
        blockers: Vec::new(),
        steps: Vec::new(),
        conflicts: Vec::new(),
        skipped: Vec::new(),
        warnings: model.warnings.clone(),
        file_bytes: 0,
        dump_bytes: 0,
        model: model.clone(),
    };
    let mut m = model.clone();

    // The account.
    let wanted = options
        .username
        .clone()
        .unwrap_or_else(|| account_name(&model.account.username));
    let mut username = wanted.to_ascii_lowercase();
    if let Err(e) = TenantUsername::parse(&username) {
        out.blockers
            .push(format!("`{username}` cannot be an account name here ({e})"));
    }
    if existing.usernames.contains(&username) {
        let resolution = if options.on_conflict == OnConflict::Rename {
            let to = free_account(&username, &existing.usernames);
            let r = Resolution::Renamed { to: to.clone() };
            username = to;
            r
        } else {
            out.blockers
                .push(format!("the account name `{username}` is already in use"));
            Resolution::Blocks
        };
        out.conflicts.push(Conflict {
            kind: "username".into(),
            name: wanted,
            resolution,
        });
    }
    let old_user = model.account.username.clone();
    m.account.username.clone_from(&username);
    out.steps.push(Step {
        action: Action::CreateAccount,
        subject: username.clone(),
        renamed_from: (username != old_user).then(|| old_user.clone()),
        bytes: None,
    });

    // Domains.
    let main_override = options.main_domain.as_deref().map(str::to_ascii_lowercase);
    let mut kept_domains = BTreeSet::new();
    m.domains.clear();
    let mut moved_domains = BTreeSet::new();
    let old_main = model
        .domains
        .iter()
        .find(|d| d.kind == DomainKind::Main)
        .map(|d| d.name.clone());
    for mut d in model.domains.iter().cloned() {
        let original = d.name.clone();
        if d.kind == DomainKind::Main
            && let Some(over) = &main_override
        {
            d.name.clone_from(over);
        }
        // A name below the old main domain moves with it: it has to lie below one of the
        // account's domains here.
        if matches!(d.kind, DomainKind::Sub | DomainKind::Alias)
            && let (Some(old), Some(new)) = (&old_main, &main_override)
            && old != new
            && let Some(prefix) = d.name.strip_suffix(&format!(".{old}"))
        {
            d.name = format!("{prefix}.{new}");
        }
        if Fqdn::parse(&d.name).is_err() {
            out.skipped.push(Skipped {
                kind: "domain".into(),
                name: d.name,
                reason: "not a valid domain name".into(),
            });
            continue;
        }
        if existing.domains.contains(&d.name) {
            let skip = options.on_conflict == OnConflict::Skip && d.kind != DomainKind::Main;
            out.conflicts.push(Conflict {
                kind: "domain".into(),
                name: d.name.clone(),
                resolution: if skip {
                    Resolution::Skipped
                } else {
                    Resolution::Blocks
                },
            });
            if skip {
                out.skipped.push(Skipped {
                    kind: "domain".into(),
                    name: d.name,
                    reason: "already on this panel".into(),
                });
            } else {
                out.blockers
                    .push(format!("the domain `{}` is already on this panel", d.name));
            }
            continue;
        }
        if d.name != original {
            moved_domains.insert(original.clone());
        }
        kept_domains.insert(d.name.clone());
        out.steps.push(Step {
            action: Action::AddDomain,
            subject: d.name.clone(),
            renamed_from: (d.name != original).then_some(original),
            bytes: None,
        });
        m.domains.push(d);
    }
    if let Some(over) = main_override {
        m.account.main_domain = Some(over);
    }

    // Files.
    if options.include.files {
        if let Some(f) = &model.files {
            out.file_bytes = f.bytes;
            out.steps.push(Step {
                action: Action::RestoreFiles,
                subject: f.root.as_str().to_owned(),
                renamed_from: None,
                bytes: Some(f.bytes),
            });
        }
    } else {
        m.files = None;
    }

    // Databases and their users.
    let mut taken = existing.db_names.clone();
    m.databases.clear();
    m.db_users.clear();
    let mut renamed: Vec<(Engine, String, String)> = Vec::new();
    if options.include.databases {
        for mut d in model.databases.iter().cloned() {
            if d.engine != Engine::Mysql {
                out.skipped.push(Skipped {
                    kind: "database".into(),
                    name: d.name,
                    reason: "PostgreSQL databases cannot be imported yet".into(),
                });
                continue;
            }
            let Some(base) = db_name(&old_user, &username, &d.name) else {
                out.skipped.push(Skipped {
                    kind: "database".into(),
                    name: d.name,
                    reason: "the name cannot be used here".into(),
                });
                continue;
            };
            let mut name = base.clone();
            if taken.contains(&name) {
                match options.on_conflict {
                    OnConflict::Rename => {
                        name = free_db(&base, &taken);
                        out.conflicts.push(Conflict {
                            kind: "database".into(),
                            name: base.clone(),
                            resolution: Resolution::Renamed { to: name.clone() },
                        });
                    }
                    OnConflict::Skip => {
                        out.conflicts.push(Conflict {
                            kind: "database".into(),
                            name: base.clone(),
                            resolution: Resolution::Skipped,
                        });
                        out.skipped.push(Skipped {
                            kind: "database".into(),
                            name: base,
                            reason: "a database with this name exists".into(),
                        });
                        continue;
                    }
                    OnConflict::Fail => {
                        out.conflicts.push(Conflict {
                            kind: "database".into(),
                            name: base.clone(),
                            resolution: Resolution::Blocks,
                        });
                        out.blockers
                            .push(format!("the database `{base}` already exists"));
                        continue;
                    }
                }
            }
            taken.insert(name.clone());
            renamed.push((d.engine, d.name.clone(), name.clone()));
            let from = (name != d.name).then(|| d.name.clone());
            out.steps.push(Step {
                action: Action::CreateDatabase,
                subject: name.clone(),
                renamed_from: from,
                bytes: None,
            });
            if d.dump.is_some() {
                let bytes = d.dump_bytes.unwrap_or(0);
                out.dump_bytes += bytes;
                out.steps.push(Step {
                    action: Action::ImportDump,
                    subject: name.clone(),
                    renamed_from: None,
                    bytes: Some(bytes),
                });
            }
            d.name = name;
            m.databases.push(d);
        }
        for mut u in model.db_users.iter().cloned() {
            if u.engine != Engine::Mysql {
                out.skipped.push(Skipped {
                    kind: "db_user".into(),
                    name: u.name,
                    reason: "PostgreSQL users cannot be imported yet".into(),
                });
                continue;
            }
            let Some(base) = db_name(&old_user, &username, &u.name) else {
                out.skipped.push(Skipped {
                    kind: "db_user".into(),
                    name: u.name,
                    reason: "the name cannot be used here".into(),
                });
                continue;
            };
            let mut name = base.clone();
            if taken.contains(&name) {
                match options.on_conflict {
                    OnConflict::Rename => {
                        name = free_db(&base, &taken);
                        out.conflicts.push(Conflict {
                            kind: "db_user".into(),
                            name: base.clone(),
                            resolution: Resolution::Renamed { to: name.clone() },
                        });
                    }
                    OnConflict::Skip => {
                        out.conflicts.push(Conflict {
                            kind: "db_user".into(),
                            name: base.clone(),
                            resolution: Resolution::Skipped,
                        });
                        out.skipped.push(Skipped {
                            kind: "db_user".into(),
                            name: base,
                            reason: "a user with this name exists".into(),
                        });
                        continue;
                    }
                    OnConflict::Fail => {
                        out.conflicts.push(Conflict {
                            kind: "db_user".into(),
                            name: base.clone(),
                            resolution: Resolution::Blocks,
                        });
                        out.blockers
                            .push(format!("the database user `{base}` already exists"));
                        continue;
                    }
                }
            }
            taken.insert(name.clone());
            // Grants follow the databases' new names; a grant on a database that was not
            // brought over goes.
            u.grants.retain_mut(|g| {
                match renamed
                    .iter()
                    .find(|(e, old, _)| *e == u.engine && *old == g.database)
                {
                    Some((_, _, new)) => {
                        g.database.clone_from(new);
                        true
                    }
                    None => false,
                }
            });
            out.steps.push(Step {
                action: Action::CreateDbUser,
                subject: name.clone(),
                renamed_from: (name != u.name).then(|| u.name.clone()),
                bytes: None,
            });
            u.name = name;
            m.db_users.push(u);
        }
    }

    // DNS: only zones of domains that came over.
    m.dns_zones.clear();
    if options.include.dns {
        for z in model.dns_zones.iter().cloned() {
            if moved_domains.contains(&z.name) {
                out.skipped.push(Skipped {
                    kind: "dns_zone".into(),
                    name: z.name,
                    reason:
                        "its domain has another name here, and the records belong to the old name"
                            .into(),
                });
            } else if kept_domains.contains(&z.name) {
                out.steps.push(Step {
                    action: Action::CreateDnsZone,
                    subject: z.name.clone(),
                    renamed_from: None,
                    bytes: None,
                });
                m.dns_zones.push(z);
            } else {
                out.skipped.push(Skipped {
                    kind: "dns_zone".into(),
                    name: z.name,
                    reason: "its domain is not brought over".into(),
                });
            }
        }
    }

    // Cron.
    m.cron_jobs.clear();
    if options.include.cron {
        for c in model.cron_jobs.iter().cloned() {
            out.steps.push(Step {
                action: Action::AddCronJob,
                subject: c.command.chars().take(60).collect(),
                renamed_from: None,
                bytes: None,
            });
            m.cron_jobs.push(c);
        }
    }

    // Mail.
    m.mailboxes.clear();
    m.forwarders.clear();
    if options.include.mail {
        let mail_items = model.mailboxes.len() + model.forwarders.len();
        if !options.mail_supported {
            if mail_items > 0 {
                out.skipped.push(Skipped {
                    kind: "mail".into(),
                    name: format!("{mail_items} mailbox(es) and forwarder(s)"),
                    reason: "this panel has no mail service switched on".into(),
                });
            }
        } else {
            for b in model.mailboxes.iter().cloned() {
                if !kept_domains.contains(&b.domain) {
                    out.skipped.push(Skipped {
                        kind: "mailbox".into(),
                        name: format!("{}@{}", b.local, b.domain),
                        reason: "its domain is not brought over".into(),
                    });
                    continue;
                }
                out.steps.push(Step {
                    action: Action::CreateMailbox,
                    subject: format!("{}@{}", b.local, b.domain),
                    renamed_from: None,
                    bytes: None,
                });
                m.mailboxes.push(b);
            }
            for f in model.forwarders.iter().cloned() {
                if !kept_domains.contains(&f.domain) {
                    continue;
                }
                out.steps.push(Step {
                    action: Action::CreateForwarder,
                    subject: format!("{}@{}", f.local, f.domain),
                    renamed_from: None,
                    bytes: None,
                });
                m.forwarders.push(f);
            }
        }
    }

    out.ok = out.blockers.is_empty();
    out.model = m;
    out
}
