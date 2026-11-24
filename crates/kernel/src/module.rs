//! What a module is to the rest of the system (T03.02).
//!
//! Modules are compiled in (workspace crates), not loaded at run time: there
//! is no plugin ABI to defend. A module is an object that hands out its
//! manifest and, optionally, routes, migrations, background jobs, reactions to
//! events and a health check. Everything except the manifest has a default, so
//! a small module is a few lines.

use std::{future::Future, pin::Pin, time::Duration};

use axum::Router;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::manifest::Manifest;

/// A boxed future, so the trait can be used as `dyn Module`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The HTTP routes of a module, already carrying their state. Paths are full
/// paths (`/api/v1/admin/...`), as everywhere in this code base.
#[derive(Default)]
pub struct ModuleRoutes {
    /// For administrators (`/api/v1/admin/...`).
    pub admin: Option<Router>,
    /// For customers (`/api/v1/client/...`).
    pub client: Option<Router>,
    /// Without a session (`/api/v1/public/...`): sign-in, enrollment, webhooks.
    pub public: Option<Router>,
}

impl ModuleRoutes {
    /// No routes.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Whether the module has no routes at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.admin.is_none() && self.client.is_none() && self.public.is_none()
    }
}

/// A database migration owned by a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    /// Sorting key, unique across the whole system (`YYYYMMDDNNNNNN`).
    pub version: i64,
    /// Short description.
    pub description: &'static str,
    /// The SQL to apply.
    pub up: &'static str,
}

/// A background job a module wants the worker to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobSpec {
    /// Unique name: `<module>.<job>`.
    pub name: &'static str,
    /// How often it runs.
    pub every: Duration,
}

/// Something that happened, announced to every enabled module (T03.05).
///
/// Events are facts in the past tense with the ids needed to act on them, not
/// copies of the data: a handler reads what it needs. They are written to the
/// outbox in the transaction that made them true and delivered afterwards, at
/// least once, so handlers must be safe to repeat. New kinds are added here as
/// modules need them; a build that does not know an event's type cannot
/// deliver it and reports that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Event {
    /// A module was switched on.
    ModuleEnabled {
        /// Its name.
        name: String,
    },
    /// A module was switched off.
    ModuleDisabled {
        /// Its name.
        name: String,
    },
    /// The settings of a module were changed. Modules that cache a setting
    /// re-read it on this event.
    SettingsChanged {
        /// The module whose settings changed.
        module: String,
    },
    /// A tenant (hosting account) was created.
    TenantCreated {
        /// The tenant.
        tenant_id: Uuid,
    },
    /// A tenant was deleted; everything that belongs to it has to go.
    TenantDeleted {
        /// The tenant.
        tenant_id: Uuid,
    },
    /// A domain was added to a tenant.
    DomainAdded {
        /// The tenant.
        tenant_id: Uuid,
        /// The domain's id.
        domain_id: Uuid,
        /// The domain name.
        name: String,
    },
    /// A domain was removed from a tenant.
    DomainRemoved {
        /// The tenant.
        tenant_id: Uuid,
        /// The domain's id.
        domain_id: Uuid,
        /// The domain name.
        name: String,
    },
    /// A domain's certificate ends soon (or has ended) and its owner has not been
    /// told yet. Sent once per certificate; a renewed one starts afresh.
    CertificateExpiring {
        /// The tenant.
        tenant_id: Uuid,
        /// The domain's id.
        domain_id: Uuid,
        /// The domain name.
        name: String,
        /// When the certificate ends (Unix seconds).
        not_after: i64,
    },
    /// A core service of a node (its database server, phpMyAdmin, its file server, its web
    /// server) is not running. Sent once when it stops, not every time it is seen stopped.
    NodeServiceDown {
        /// The node.
        node_id: Uuid,
        /// The service (`database`, `phpmyadmin`, `sftp`, `caddy`).
        service: String,
        /// How it is: `stopped`, `unreachable`.
        state: String,
    },
    /// A core service that was down is running again.
    NodeServiceRecovered {
        /// The node.
        node_id: Uuid,
        /// The service.
        service: String,
    },
    /// A node that was online is not (its session ended, or it fell silent). Written by a
    /// database trigger on the change itself.
    NodeOffline {
        /// The node.
        node_id: Uuid,
        /// Its name.
        name: String,
    },
    /// The panel's own update: a newer release is on offer (`available`), the automatic update has
    /// begun (`started`), or an automatic update did not work (`failed`). For the administrators.
    PanelUpdate {
        /// The release's version.
        version: String,
        /// `available`, `started` or `failed`.
        stage: String,
        /// With `available`: in how many days the panel will install it by itself (none when
        /// automatic updates are off).
        auto_in_days: Option<i64>,
    },
    /// A backup failed. Written by a database trigger on the change itself.
    BackupFailed {
        /// The backup.
        backup_id: Uuid,
        /// The hosting account it belongs to (none for the panel's own backup).
        tenant_id: Option<Uuid>,
        /// What went wrong, shortened.
        error: String,
    },
}

impl Event {
    /// The event's wire name (the `type` in its JSON form).
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::ModuleEnabled { .. } => "module_enabled",
            Self::ModuleDisabled { .. } => "module_disabled",
            Self::SettingsChanged { .. } => "settings_changed",
            Self::TenantCreated { .. } => "tenant_created",
            Self::TenantDeleted { .. } => "tenant_deleted",
            Self::DomainAdded { .. } => "domain_added",
            Self::DomainRemoved { .. } => "domain_removed",
            Self::CertificateExpiring { .. } => "certificate_expiring",
            Self::NodeServiceDown { .. } => "node_service_down",
            Self::NodeServiceRecovered { .. } => "node_service_recovered",
            Self::NodeOffline { .. } => "node_offline",
            Self::PanelUpdate { .. } => "panel_update",
            Self::BackupFailed { .. } => "backup_failed",
        }
    }
}

/// A module could not handle an event. The message is for the log.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct EventError(pub String);

/// How a module is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    /// Working.
    Ok,
    /// Working with a problem worth showing to an administrator.
    Degraded(String),
    /// Not working.
    Down(String),
}

/// The contract every module implements.
pub trait Module: Send + Sync + 'static {
    /// The module's manifest (name, dependencies, permissions, ...).
    fn manifest(&self) -> &Manifest;

    /// The module's HTTP routes. Only called for enabled modules.
    fn routes(&self) -> ModuleRoutes {
        ModuleRoutes::none()
    }

    /// The module's settings, as a JSON Schema (T03.08): an object whose
    /// properties are the settings, each with a `type`, a `default` and
    /// optionally `x-scope` (`global`, `node` or `tenant`; `global` if absent),
    /// `title`, `description`. The settings service checks it at startup and
    /// validates every value against it before storing.
    fn settings_schema(&self) -> Option<serde_json::Value> {
        None
    }

    /// The module's database migrations, oldest first.
    fn migrations(&self) -> &'static [Migration] {
        &[]
    }

    /// Background jobs for the worker.
    fn jobs(&self) -> Vec<JobSpec> {
        Vec::new()
    }

    /// Reacts to an event from another module (for example: a tenant was
    /// deleted, remove its domains). Must be safe to run more than once.
    fn on_event<'a>(&'a self, _event: &'a Event) -> BoxFuture<'a, Result<(), EventError>> {
        Box::pin(async { Ok(()) })
    }

    /// The module's own health.
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ok })
    }
}

/// A module that is only its manifest: for modules whose code is not yet
/// behind the trait, and for tests.
#[derive(Debug, Clone)]
pub struct ManifestOnly(pub Manifest);

impl Module for ManifestOnly {
    fn manifest(&self) -> &Manifest {
        &self.0
    }
}
