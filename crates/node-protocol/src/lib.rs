//! Wire types shared by the control plane and the node agent.
//!
//! See `docs/adr/0004-node-agent-protocol.md`. In short: one WebSocket per
//! node over mutual TLS, one JSON object per text frame, at most
//! [`MAX_FRAME_BYTES`]. Both ends are Rust and use these very types, so a
//! mismatch is a compile error, not a runtime surprise.
//!
//! ## Compatibility rules
//! * Changes within one [`PROTOCOL_VERSION`] are **additive**: new operations,
//!   new outputs, new optional fields (give them `#[serde(default)]`).
//!   Unknown JSON fields are ignored, never rejected.
//! * An operation an older agent does not know does not break the connection:
//!   [`parse_operation_frame`] returns [`Parsed::Unsupported`] and the agent
//!   answers that `op_id` with an `unsupported_operation` error.
//! * A breaking change bumps [`PROTOCOL_VERSION`]; the control plane keeps
//!   accepting the previous one ([`MIN_SUPPORTED_VERSION`]).
//!
//! No operation here carries a secret; if one ever must, its type gets a
//! redacting `Debug` and is left out of logs.

pub mod files;
pub mod studio;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Version spoken by this build.
pub const PROTOCOL_VERSION: u16 = 1;
/// Oldest version this build still understands.
pub const MIN_SUPPORTED_VERSION: u16 = 1;
/// Largest frame accepted or produced, in bytes. Bulk data uses other channels.
pub const MAX_FRAME_BYTES: usize = 1 << 20;
/// Longest time an envelope may stay valid, in seconds. Bounds the replay window.
pub const MAX_ENVELOPE_LIFETIME_SECS: i64 = 3600;
/// How far in the future `issued_at` may be before the agent distrusts it.
pub const MAX_CLOCK_SKEW_SECS: i64 = 300;

/// How often the control plane pings an agent, in seconds. The agent answers
/// with a pong; a ping also tells the agent the control plane is alive.
pub const HEARTBEAT_INTERVAL_SECS: u64 = 15;
/// A session that has sent nothing (no pong, no message) for this long (three
/// missed pings) is dead: the control plane closes it and the node shows as
/// offline. The check runs on each ping tick, so a node that vanishes without a
/// goodbye is offline within this plus one interval: at most 60 seconds.
pub const LIVENESS_TIMEOUT_SECS: u64 = 45;
/// An agent that has heard nothing from the control plane for this long
/// (three missed pings) drops the connection and reconnects.
pub const AGENT_SILENCE_TIMEOUT_SECS: u64 = 45;
/// Path of the agent port's WebSocket endpoint (the only one it serves).
pub const STREAM_PATH: &str = "/agent/v1/stream";

/// The protocol version to speak with a peer announcing `peer`, or `None`
/// when the peer is too old.
#[must_use]
pub fn negotiate_version(peer: u16) -> Option<u16> {
    (peer >= MIN_SUPPORTED_VERSION).then_some(peer.min(PROTOCOL_VERSION))
}

// ------------------------------------------------------------------ errors

/// Machine-readable failure reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The agent does not know this operation (older agent, newer control plane).
    UnsupportedOperation,
    /// The operation is known but its arguments are invalid.
    InvalidArguments,
    /// The envelope's validity window has passed (or is not plausible).
    Expired,
    /// The envelope's protocol version is not supported.
    UnsupportedVersion,
    /// The agent is running as many operations as it allows; try again shortly.
    Busy,
    /// The operation did not finish in the time it had.
    Timeout,
    /// Something went wrong inside the agent; see the message.
    Internal,
}

/// A failure reported by the agent. The message is safe to show to administrators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct AgentError {
    /// What kind of failure.
    pub code: ErrorCode,
    /// Human-readable detail (English; never contains secrets).
    pub message: String,
    /// Whether trying the same operation again may succeed.
    #[serde(default)]
    pub retryable: bool,
}

impl AgentError {
    /// A non-retryable error.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: false,
        }
    }

    /// Marks the error as worth retrying.
    #[must_use]
    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }
}

// ------------------------------------------------------------------- routes

/// One redirect of a site: the exact path it applies to and where it sends the
/// visitor. Both were checked by the control plane and are checked again by the
/// agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteRedirect {
    /// The exact path, starting with `/`.
    pub path: String,
    /// A path of the same site or an http(s) address.
    pub target: String,
    /// 301, 302, 307 or 308.
    pub code: u16,
}

/// How the web server answers for a group of names: the same site under every
/// name in `hosts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteRoute {
    /// The names (lower-case ASCII), at least one.
    pub hosts: Vec<String>,
    /// Where the site's web container listens, `host:port` (for example
    /// `bp-alice-web:8080`), or `None` for a suspended site: visitors then get
    /// the "suspended" page and nothing is served from the tenant's files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    /// Redirects, answered before the site is asked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redirects: Vec<RouteRedirect>,
    /// Visitors who come over plain HTTP are sent to the same address over HTTPS.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub force_https: bool,
    /// Answers over HTTPS carry `Strict-Transport-Security` (one year). Only with
    /// `force_https`; the agent refuses it without.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hsts: bool,
    /// For a suspended site (no `upstream`): who suspended it, `domain` or
    /// `account`, which picks the page visitors see. Empty means `domain`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspended_by: Option<String>,
    /// The web application firewall for this site: `None` is off. Only a node whose web
    /// server has the firewall (`BUILTPANEL_AGENT_WAF_DIR`) applies it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waf: Option<WafSetting>,
    /// Addresses and networks (canonical, see [`normalize_ip_block`]) the web server answers
    /// `403` before anything else looks at the request; at most [`MAX_BLOCKED_IPS`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_ips: Vec<String>,
}

/// How the firewall acts for one site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WafSetting {
    /// `detect` (log what it would block) or `block`.
    pub mode: String,
    /// Rule ids that do not apply (1 to 999999, at most [`MAX_WAF_RULES`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_rules: Vec<u32>,
}

/// The values an application's steps are given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSite {
    /// Its address, `https://example.com/blog`.
    pub url: String,
    /// Its title.
    pub title: String,
    /// The first administrator's login (empty when the definition has none).
    #[serde(default)]
    pub admin_user: String,
    /// Their email address (empty when the definition has none).
    #[serde(default)]
    pub admin_email: String,
}

/// Where an application's files come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct AppSource {
    /// An `https://` address.
    pub url: String,
    /// The SHA-256 of the file, 64 hex digits: a file that does not match is not used.
    pub sha256: String,
    /// `zip` or `tar.gz`.
    pub format: String,
    /// Leading folders of the archive to drop (0 or 1).
    #[serde(default)]
    pub strip: u8,
}

/// One step of an installation: a fixed vocabulary, never shell text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppStep {
    /// Write a file (relative to the application's folder) from a text with placeholders.
    WriteFile {
        /// Where, `config/app.php`.
        path: String,
        /// The text; `{{title}}`, `{{site_url}}`, `{{admin_user}}`, `{{admin_email}}`,
        /// `{{admin_password}}`, `{{db_name}}`, `{{db_user}}`, `{{db_password}}`, `{{db_host}}`,
        /// `{{random}}` (32 hex digits, new each time) and `{{app_key}}` (`base64:` and 32
        /// random bytes) are replaced.
        content: String,
    },
    /// Run `php` in the application's folder with these arguments (placeholders as above;
    /// one argument may not hold a line break).
    Php {
        /// The arguments.
        args: Vec<String>,
    },
    /// Run Composer (a pinned version, checked by its checksum) in the application's folder.
    Composer {
        /// The arguments.
        args: Vec<String>,
    },
    /// Remove a file or folder inside the application's folder.
    Remove {
        /// What.
        path: String,
    },
}

/// An application of the catalog: data, not code. Checked by [`AppDefinition::checked`] on
/// the control plane when it is loaded and again on the node before anything runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct AppDefinition {
    /// The catalog name, `joomla`.
    pub slug: String,
    /// What people see, `Joomla`.
    pub name: String,
    /// The version it installs.
    pub version: String,
    /// A line about it.
    #[serde(default)]
    pub description: String,
    /// A group, `cms`.
    #[serde(default)]
    pub category: String,
    /// It needs a database of its own.
    #[serde(default)]
    pub needs_database: bool,
    /// `wordpress` for the built-in WordPress installer (which has no steps of its own);
    /// empty for everything made of source and steps.
    #[serde(default)]
    pub engine: String,
    /// The folder of the application the domain should be served from (`public`, `web`),
    /// when it is not the application's own folder.
    #[serde(default)]
    pub document_root: String,
    /// Where the files come from; none for an application made of its steps alone.
    #[serde(default)]
    pub source: Option<AppSource>,
    /// What to do after the files are in place.
    #[serde(default)]
    pub steps: Vec<AppStep>,
}

/// The placeholders a definition's texts may hold.
pub const APP_PLACEHOLDERS: [&str; 10] = [
    "title",
    "site_url",
    "admin_user",
    "admin_email",
    "admin_password",
    "db_name",
    "db_user",
    "db_password",
    "db_host",
    "random",
];

impl AppDefinition {
    /// The placeholders the definition uses (`app_key` counts as `random`).
    #[must_use]
    pub fn placeholders(&self) -> Vec<String> {
        let mut found = Vec::new();
        let mut scan = |text: &str| {
            let mut rest = text;
            while let Some(start) = rest.find("{{") {
                let after = &rest[start + 2..];
                let Some(end) = after.find("}}") else { break };
                let name = after[..end].trim().to_owned();
                if !found.contains(&name) {
                    found.push(name);
                }
                rest = &after[end + 2..];
            }
        };
        for step in &self.steps {
            match step {
                AppStep::WriteFile { content, .. } => scan(content),
                AppStep::Php { args } | AppStep::Composer { args } => {
                    args.iter().for_each(|a| scan(a));
                }
                AppStep::Remove { .. } => {}
            }
        }
        found
    }

    /// Whether the steps need an administrator's login, address and password.
    #[must_use]
    pub fn needs_admin(&self) -> bool {
        self.engine == "wordpress"
            || self
                .placeholders()
                .iter()
                .any(|p| matches!(p.as_str(), "admin_user" | "admin_email" | "admin_password"))
    }

    /// Checks everything that is checkable without running anything.
    ///
    /// # Errors
    /// The first thing that is not valid, in words.
    pub fn checked(&self) -> Result<(), String> {
        fn plain(text: &str, min: usize, max: usize, extra: &str) -> bool {
            (min..=max).contains(&text.len())
                && text
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
        }
        fn rel_path(path: &str) -> bool {
            !path.is_empty()
                && path.len() <= 200
                && path.split('/').all(|part| {
                    !part.is_empty()
                        && part != "."
                        && part != ".."
                        && part
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
                })
        }
        if !plain(&self.slug, 2, 32, "-_")
            || !self.slug.starts_with(|c: char| c.is_ascii_lowercase())
        {
            return Err("slug: 2 to 32 lower-case letters, digits, - and _".into());
        }
        if self.name.is_empty()
            || self.name.chars().count() > 60
            || self.name.chars().any(char::is_control)
        {
            return Err("name: 1 to 60 characters".into());
        }
        if !plain(&self.version, 1, 40, ".+_-") {
            return Err("version: not valid".into());
        }
        if self.description.chars().count() > 300 || self.description.chars().any(char::is_control)
        {
            return Err("description: at most 300 characters".into());
        }
        if !self.category.is_empty() && !plain(&self.category, 1, 32, "-_") {
            return Err("category: not valid".into());
        }
        if !["", "wordpress"].contains(&self.engine.as_str()) {
            return Err("engine: empty or wordpress".into());
        }
        if self.engine == "wordpress" && (self.source.is_some() || !self.steps.is_empty()) {
            return Err("a wordpress definition has no source or steps".into());
        }
        if !self.document_root.is_empty() && !rel_path(&self.document_root) {
            return Err("document_root: a plain relative folder".into());
        }
        if let Some(source) = &self.source {
            let url_ok = source.url.starts_with("https://")
                && source.url.len() <= 400
                && source
                    .url
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-._~:/?#[]@!&()*+,;=%".contains(c));
            if !url_ok {
                return Err("source.url: an https address of plain characters".into());
            }
            if source.sha256.len() != 64
                || !source
                    .sha256
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
            {
                return Err("source.sha256: 64 lower-case hex digits".into());
            }
            if !["zip", "tar.gz"].contains(&source.format.as_str()) || source.strip > 1 {
                return Err("source: format is zip or tar.gz, strip is 0 or 1".into());
            }
        }
        if self.steps.len() > 40 {
            return Err("steps: at most 40".into());
        }
        for (i, step) in self.steps.iter().enumerate() {
            let bad = |what: &str| Err(format!("step {}: {what}", i + 1));
            match step {
                AppStep::WriteFile { path, content } => {
                    if !rel_path(path) {
                        return bad("path must be a plain relative path");
                    }
                    if content.len() > 64 * 1024 || content.contains('\0') {
                        return bad("content: at most 64 KiB, no NUL");
                    }
                    if content.replace(' ', "").contains("{{admin_password}}") {
                        return bad(
                            "the administrator's password may only be an argument, not file text",
                        );
                    }
                }
                AppStep::Php { args } | AppStep::Composer { args } => {
                    if args.is_empty()
                        || args.len() > 40
                        || args
                            .iter()
                            .any(|a| a.len() > 1000 || a.contains(['\n', '\r', '\0']))
                    {
                        return bad("1 to 40 arguments of at most 1000 characters, no line breaks");
                    }
                }
                AppStep::Remove { path } => {
                    if !rel_path(path) {
                        return bad("path must be a plain relative path");
                    }
                }
            }
        }
        for name in self.placeholders() {
            let known = APP_PLACEHOLDERS.contains(&name.as_str()) || name == "app_key";
            if !known {
                return Err(format!("unknown placeholder {{{{{name}}}}}"));
            }
            if name.starts_with("db_") && !self.needs_database {
                return Err(format!("{{{{{name}}}}} needs needs_database = true"));
            }
        }
        Ok(())
    }
}

/// What can be done to a WordPress site (each is a fixed WP-CLI command line).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WpCommand {
    /// Versions of the core, plugins and themes, which have updates, the maintenance state.
    Overview,
    /// Update one plugin, or all of them.
    UpdatePlugins {
        /// A plugin's folder name; all plugins when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slug: Option<String>,
    },
    /// Update one theme, or all of them.
    UpdateThemes {
        /// A theme's folder name; all themes when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slug: Option<String>,
    },
    /// Update the core (to a version, or the latest) and its database.
    UpdateCore {
        /// A version such as `6.8.2`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },
    /// Switch maintenance mode on or off.
    Maintenance {
        /// On or off.
        on: bool,
    },
    /// Check the core's files against WordPress's published checksums and look at the
    /// settings that matter for safety (debug mode, the file editor, XML-RPC, permissions).
    Scan,
    /// Apply the hardening the flags ask for.
    Harden {
        /// Turn `WP_DEBUG` (and its display) off.
        debug_off: bool,
        /// Forbid editing plugin and theme files from the admin area.
        file_edit_off: bool,
        /// Switch XML-RPC off (a must-use plugin of ours).
        xmlrpc_off: bool,
        /// Take write permission from everybody but the owner and group, and make
        /// `wp-config.php` private to its owner and group.
        fix_permissions: bool,
    },
    /// Set a user's password (given with the operation, not in it).
    ResetAdminPassword {
        /// The user's login.
        user: String,
    },
}

/// What a scan of a WordPress site found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct WpScan {
    /// Every core file is as WordPress published it (and none is missing or extra).
    pub checksums_ok: bool,
    /// Core files whose content is not the published one (at most 100).
    pub modified: Vec<String>,
    /// Files in the core's folders that WordPress does not ship (at most 100).
    pub extra: Vec<String>,
    /// Core files that are gone (at most 100).
    pub missing: Vec<String>,
    /// `WP_DEBUG` is on.
    pub debug: bool,
    /// Plugin and theme files can be edited from the admin area.
    pub file_edit_allowed: bool,
    /// XML-RPC was switched off by the panel.
    pub xmlrpc_disabled: bool,
    /// The permission bits of `wp-config.php` (`640`).
    pub config_mode: String,
    /// Files and folders anybody may write to.
    pub world_writable: u32,
}

/// A plugin or theme of a site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct WpItem {
    /// Its folder name.
    pub name: String,
    /// `active`, `inactive`, `must-use`, ...
    pub status: String,
    /// The version installed.
    pub version: String,
    /// A newer version exists.
    pub update_available: bool,
    /// That version.
    pub update_version: String,
}

/// What a WordPress site is made of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct WpOverview {
    /// The core's version.
    pub core: String,
    /// The version the core can be updated to, when there is one.
    pub core_update: Option<String>,
    /// Maintenance mode is on.
    pub maintenance: bool,
    /// The plugins.
    pub plugins: Vec<WpItem>,
    /// The themes.
    pub themes: Vec<WpItem>,
}

/// The database a WordPress site uses (on the node's database server, reached as `db`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WpDatabase {
    /// The whole name, `alice_blog`.
    pub name: String,
    /// The user's whole name, `alice_web`.
    pub user: String,
}

/// A new WordPress site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WpSite {
    /// Its address, `https://example.com/blog`.
    pub url: String,
    /// Its title.
    pub title: String,
    /// The first administrator's login.
    pub admin_user: String,
    /// Their email address.
    pub admin_email: String,
}

/// One vulnerability Trivy found in an image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct ImageVulnerability {
    /// `CVE-2025-1234`.
    pub id: String,
    /// The package.
    pub package: String,
    /// The version in the image.
    pub installed: String,
    /// The first version that fixes it (empty when none does yet).
    pub fixed: String,
    /// `CRITICAL`, `HIGH`, `MEDIUM` or `LOW`.
    pub severity: String,
    /// A short title (clipped).
    pub title: String,
}

/// What a registry says a tag is now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageDigest {
    /// The `name:tag` asked about.
    pub image: String,
    /// Its digest now (`sha256:…`).
    pub digest: String,
}

/// A WordPress plugin or theme and the version it says it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct WpComponent {
    /// Its folder name (`woocommerce`).
    pub slug: String,
    /// Its version (empty when it does not say).
    pub version: String,
}

/// One WordPress installation in a tenant's files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct WpInstall {
    /// Its folder, relative to the tenant's files (`www`, `www/blog`).
    pub path: String,
    /// The core's version (empty when unreadable).
    pub core: String,
    /// The plugins.
    pub plugins: Vec<WpComponent>,
    /// The themes.
    pub themes: Vec<WpComponent>,
}

/// One infected file a scan found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct Infection {
    /// Where it is, relative to the tenant's files (`www/uploads/x.php`).
    pub path: String,
    /// The signature that matched (`Php.Trojan.Agent`).
    pub signature: String,
    /// It was moved to the trash.
    pub quarantined: bool,
}

/// The node's firewall (T11.06): who may reach which port. Applied by the privileged helper as
/// one nftables table of its own; IPv4 only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct FirewallPolicy {
    /// Drop whatever is not allowed below (and not SSH, loopback, answers to what the node
    /// asked for, ICMP and the `always_allow` addresses). Without it only the `blocked`
    /// addresses and the rate limits act.
    #[serde(default)]
    pub default_deny: bool,
    /// Addresses that always get in, whatever else is said: the panel's own, an administrator's.
    /// With `default_deny` at least one is needed (it is what keeps the panel reachable).
    #[serde(default)]
    pub always_allow: Vec<String>,
    /// Addresses allowed to everything (before `blocked` is looked at).
    #[serde(default)]
    pub allowed: Vec<String>,
    /// Addresses refused everything.
    #[serde(default)]
    pub blocked: Vec<String>,
    /// Ports open to everybody (or to the listed addresses only).
    #[serde(default)]
    pub open_ports: Vec<PortRule>,
    /// New connections per minute one address may make to a port.
    #[serde(default)]
    pub rate_limits: Vec<RateLimit>,
}

/// One open port (or range).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct PortRule {
    /// The port (the first of a range).
    pub port: u16,
    /// The last port of a range (`port` to `to`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<u16>,
    /// `tcp` or `udp`.
    pub proto: String,
    /// Only these addresses (empty: everybody).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub from: Vec<String>,
}

/// A limit on new connections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct RateLimit {
    /// The port.
    pub port: u16,
    /// `tcp` or `udp`.
    pub proto: String,
    /// New connections a minute per source address (1 to 100000).
    pub per_minute: u32,
}

/// Most entries in each list of a firewall policy.
pub const MAX_FIREWALL_ENTRIES: usize = 2000;

/// An IPv4 address or range as `a.b.c.d` or `a.b.c.d/n` (n from 8 to 32, at the start of its
/// range), made `a.b.c.d/n`; `None` when it is anything else.
#[must_use]
pub fn normalize_cidr(text: &str) -> Option<String> {
    let (ip, bits) = match text.split_once('/') {
        Some((ip, bits)) => (
            ip,
            bits.parse::<u32>().ok().filter(|b| bits == b.to_string())?,
        ),
        None => (text, 32),
    };
    if !(8..=32).contains(&bits) {
        return None;
    }
    let parts: Vec<&str> = ip.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut octets = [0u8; 4];
    for (slot, part) in octets.iter_mut().zip(&parts) {
        let plain = !part.is_empty()
            && part.len() <= 3
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part.len() == 1 || !part.starts_with('0'));
        match part.parse::<u8>() {
            Ok(n) if plain => *slot = n,
            _ => return None,
        }
    }
    let mask = u32::MAX << (32 - bits);
    (u32::from_be_bytes(octets) & !mask == 0).then(|| format!("{ip}/{bits}"))
}

impl FirewallPolicy {
    /// Checks the policy and returns it in its plain form (addresses as `a.b.c.d/n`, no
    /// duplicates), or says what is wrong.
    ///
    /// # Errors
    /// A message naming the field.
    pub fn checked(&self) -> Result<Self, String> {
        fn cidrs(name: &str, list: &[String]) -> Result<Vec<String>, String> {
            if list.len() > MAX_FIREWALL_ENTRIES {
                return Err(format!("{name}: too many entries"));
            }
            let mut out: Vec<String> = Vec::new();
            for item in list {
                let c = normalize_cidr(item).ok_or_else(|| {
                    format!(
                        "{name}: `{}` is not an IPv4 address or range (a.b.c.d/n, n from 8 to 32)",
                        item.escape_default()
                    )
                })?;
                if !out.contains(&c) {
                    out.push(c);
                }
            }
            Ok(out)
        }
        let proto = |p: &str| matches!(p, "tcp" | "udp");
        let always_allow = cidrs("always_allow", &self.always_allow)?;
        let allowed = cidrs("allowed", &self.allowed)?;
        let blocked = cidrs("blocked", &self.blocked)?;
        if self.default_deny && always_allow.is_empty() {
            return Err("always_allow: at least one address is needed with default_deny".into());
        }
        if self.open_ports.len() > MAX_FIREWALL_ENTRIES
            || self.rate_limits.len() > MAX_FIREWALL_ENTRIES
        {
            return Err("too many ports".into());
        }
        let mut open_ports = Vec::new();
        for rule in &self.open_ports {
            if rule.port == 0
                || rule.to.is_some_and(|t| t < rule.port || t == 0)
                || !proto(&rule.proto)
            {
                return Err("open_ports: a port, a range or a protocol is not valid".into());
            }
            open_ports.push(PortRule {
                from: cidrs("open_ports.from", &rule.from)?,
                ..rule.clone()
            });
        }
        for rule in &self.rate_limits {
            if rule.port == 0 || !proto(&rule.proto) || !(1..=100_000).contains(&rule.per_minute) {
                return Err("rate_limits: a port, a protocol or a rate is not valid".into());
            }
        }
        Ok(Self {
            default_deny: self.default_deny,
            always_allow,
            allowed,
            blocked,
            open_ports,
            rate_limits: self.rate_limits.clone(),
        })
    }
}

/// Most User-Agent texts one node's list may hold.
pub const MAX_USER_AGENT_PATTERNS: usize = 500;

/// Most rules one site may switch off.
pub const MAX_WAF_RULES: usize = 200;

/// Most refused requests one read of the firewall's log names.
pub const MAX_WAF_EVENTS: u32 = 200;

/// Most names one read of the firewall's log may be limited to.
pub const MAX_WAF_HOSTS: usize = 500;

/// One request the web application firewall stopped (or, for a site that only watches, saw).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WafEvent {
    /// When (Unix milliseconds, UTC).
    pub at_ms: i64,
    /// The request method.
    pub method: String,
    /// The name asked for.
    pub host: String,
    /// The path and query asked for (cut at 200 characters).
    pub uri: String,
    /// The rule that named the attack (0 when the log does not say).
    pub rule_id: u32,
    /// What the rule says (cut at 200 characters).
    pub message: String,
    /// The request was refused.
    pub blocked: bool,
}

/// Most addresses or networks one site may turn away.
pub const MAX_BLOCKED_IPS: usize = 100;

/// An address or a network the web server may be told to turn away, in its canonical text:
/// an IPv4 or IPv6 address (`203.0.113.9`, `2001:db8::1`), or a network (`203.0.113.0/24`) whose
/// host bits are cleared. Networks larger than a /8 (IPv4) or a /16 (IPv6) are refused: such a
/// list is a mistake far more often than a wish, and the loopback, unspecified and multicast
/// addresses cannot be visitors.
///
/// # Errors
/// A short reason, fit to show to a person.
pub fn normalize_ip_block(text: &str) -> Result<String, &'static str> {
    use std::net::IpAddr;
    let text = text.trim();
    let (addr, prefix) = match text.split_once('/') {
        Some((a, p)) => (a, Some(p)),
        None => (text, None),
    };
    if addr.is_empty() || addr.len() > 39 {
        return Err("Not an IP address or network.");
    }
    let ip: IpAddr = addr.parse().map_err(|_| "Not an IP address or network.")?;
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return Err("That address cannot be a visitor.");
    }
    let (bits, shortest) = match ip {
        IpAddr::V4(_) => (32u8, 8u8),
        IpAddr::V6(_) => (128u8, 16u8),
    };
    let prefix = match prefix {
        None => bits,
        Some(p) => {
            if p.is_empty() || p.len() > 3 || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err("The network size after the / is not a number.");
            }
            p.parse::<u8>()
                .map_err(|_| "The network size is too large.")?
        }
    };
    if prefix > bits {
        return Err("The network size is too large.");
    }
    if prefix < shortest {
        return Err("That network is too large to block.");
    }
    Ok(match ip {
        IpAddr::V4(v4) => {
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - u32::from(prefix))
            };
            let net = std::net::Ipv4Addr::from(u32::from(v4) & mask);
            if prefix == 32 {
                net.to_string()
            } else {
                format!("{net}/{prefix}")
            }
        }
        IpAddr::V6(v6) => {
            let mask = u128::MAX << (128 - u32::from(prefix));
            let net = std::net::Ipv6Addr::from(u128::from(v6) & mask);
            if prefix == 128 {
                net.to_string()
            } else {
                format!("{net}/{prefix}")
            }
        }
    })
}

/// One page for the web server to show when a site cannot answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorPage {
    /// `suspended_domain`, `suspended_account`, `not_found` or `bad_gateway`.
    pub kind: String,
    /// The page (HTML, at most [`MAX_ERROR_PAGE_BYTES`]).
    pub html: String,
}

/// Longest error page.
pub const MAX_ERROR_PAGE_BYTES: usize = 16 * 1024;

/// A certificate a person uploaded for one of their names, for the web server to use.
///
/// The private key is not part of what the control plane records in its ledger:
/// there it is `None`, and the dispatcher fills it in at the moment the message is
/// sent (see `builtpanel_dispatcher::SecretFiller`). `Debug` never shows it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomCertificate {
    /// The name it is for (lower-case ASCII).
    pub host: String,
    /// The certificates, PEM: the site's first, then the ones that vouch for it.
    pub certificate: String,
    /// The private key, PKCS#8 PEM. Present only on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
}

impl std::fmt::Debug for CustomCertificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomCertificate")
            .field("host", &self.host)
            .field("has_key", &self.private_key.is_some())
            .finish_non_exhaustive()
    }
}

/// A database user's password. It is not part of what the control plane records
/// in its ledger (there it is absent): the dispatcher puts it in at the moment the
/// message is sent. `Debug` never shows it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DbPassword(pub String);

impl std::fmt::Debug for DbPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DbPassword(..)")
    }
}

/// The environment of a tenant's own application: names and values. Like [`DbPassword`] it is
/// absent in the ledger and put in when the message is sent; `Debug` shows the names only.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeEnv(pub Vec<(String, String)>);

impl std::fmt::Debug for RuntimeEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|(k, _)| k.as_str()))
            .finish()
    }
}

/// One DNS record a mail domain needs, as the mail server gives it (names are relative to the
/// domain: `@` is the domain itself).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailDnsRecord {
    /// `@`, `selector._domainkey`, `_dmarc`, ...
    pub name: String,
    /// `MX` or `TXT`.
    pub rtype: String,
    /// The value (an MX's target host, a TXT's text).
    pub content: String,
    /// An MX's preference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u16>,
}

/// An address that sends what it gets on to others (the address is not a mailbox).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailForwarder {
    /// The local part of the address.
    pub local: String,
    /// Where it goes (full addresses).
    pub to: Vec<String>,
}

/// A message waiting in the mail server's outgoing queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailQueueItem {
    /// The queue's own id.
    pub id: String,
    /// The sender.
    pub sender: String,
    /// The recipients.
    pub recipients: Vec<String>,
    /// `queued` (not tried yet) or `deferred` (tried, will be again).
    pub status: String,
    /// Why the last try failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// When it was queued (Unix seconds).
    pub queued_at: i64,
    /// How many times it was tried.
    pub retries: u32,
    /// Its size in bytes.
    pub size_bytes: u64,
}

/// How much one mailbox holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailUsage {
    /// The whole address.
    pub address: String,
    /// Bytes stored.
    pub used_bytes: u64,
}

/// Which kind of database server an operation is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbEngine {
    /// The node's MariaDB server (what every operation meant before PostgreSQL).
    #[default]
    Mysql,
    /// The node's PostgreSQL server.
    Postgres,
}

impl DbEngine {
    /// Whether it is the default (so that it is left out of what is written).
    #[must_use]
    pub fn is_mysql(&self) -> bool {
        matches!(self, Self::Mysql)
    }

    /// The name kept in the database and shown.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mysql => "mysql",
            Self::Postgres => "postgres",
        }
    }

    /// Reads the name kept in the database.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "mysql" => Some(Self::Mysql),
            "postgres" => Some(Self::Postgres),
            _ => None,
        }
    }
}

/// A database user whose password the node keeps in a backup (inside the encrypted setup
/// data), so that a transfer to another node makes the user again with the same password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupDbUser {
    /// The user's whole name.
    pub name: String,
    /// Which kind of server it is on.
    #[serde(default, skip_serializing_if = "DbEngine::is_mysql")]
    pub engine: DbEngine,
}

/// What one user may do in one database, as [`Operation::MigrateTenantDatabases`] carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrateGrant {
    /// The database's whole name.
    pub database: String,
    /// The user's whole name.
    pub user: String,
    /// `read_only`, `read_write` or `all`.
    pub access: String,
}

/// The remote addresses of one user, as [`Operation::MigrateTenantDatabases`] carries them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrateRemote {
    /// The user's whole name.
    pub user: String,
    /// IPv4 `a.b.c.d/n`.
    pub cidrs: Vec<String>,
}

/// The size of one database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DbSize {
    /// The whole name.
    pub name: String,
    /// Data and indexes, in bytes.
    pub bytes: u64,
}

/// One cron job, as the node's scheduler needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CronJobSpec {
    /// The job's id.
    pub id: Uuid,
    /// Five fields: minute hour day-of-month month day-of-week (UTC).
    pub schedule: String,
    /// The command line, run with `sh -c` inside the account's PHP container.
    pub command: String,
    /// Whether it runs.
    pub enabled: bool,
    /// Stopped after this many seconds.
    pub timeout_seconds: u32,
}

/// Most cron jobs one account may have on a node.
pub const MAX_CRON_JOBS: usize = 200;

/// How a cron run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CronStatus {
    /// Exit code 0.
    Ok,
    /// Another exit code, or the container could not run it.
    Failed,
    /// Stopped for taking longer than its timeout.
    Timeout,
    /// Not started: the last run was still going.
    Skipped,
}

/// One run of a cron job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CronRun {
    /// The job.
    pub job_id: Uuid,
    /// Its account.
    pub tenant_id: Uuid,
    /// When it started (Unix seconds).
    pub started_at: i64,
    /// How it ended.
    pub status: CronStatus,
    /// The exit code, when there was one.
    pub exit_code: Option<i32>,
    /// The end of what it printed (clipped).
    pub output: String,
}

/// Which log of an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogSource {
    /// What the account's web server (nginx) wrote to standard output: requests.
    WebAccess,
    /// What the account's web server wrote to standard error: its errors.
    WebError,
    /// What the account's PHP container wrote to standard error: PHP and PHP-FPM errors.
    PhpError,
}

/// One line of a log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogLine {
    /// When it was written (Unix milliseconds).
    pub at_ms: i64,
    /// The line.
    pub text: String,
}

/// Most lines one log read returns.
pub const MAX_LOG_LINES: u32 = 2000;

/// Most pages one traffic count names.
pub const MAX_TRAFFIC_PATHS: u32 = 50;

/// One hour of an account's web traffic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficHour {
    /// The start of the hour (Unix milliseconds, UTC).
    pub at_ms: i64,
    /// Requests in it.
    pub requests: u64,
    /// Bytes sent in it.
    pub bytes: u64,
}

/// A page and how often it was asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrafficPath {
    /// The path without the query string.
    pub path: String,
    /// Requests for it.
    pub requests: u64,
}

/// The node as a whole, as counters and levels (rates are made from two readings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeMetrics {
    /// Logical CPUs.
    pub cpus: u32,
    /// Jiffies (1/100 s) all CPUs spent working, since boot.
    pub cpu_busy_jiffies: u64,
    /// Jiffies all CPUs existed, since boot (busy and idle).
    pub cpu_total_jiffies: u64,
    /// Memory of the node, in bytes.
    pub memory_total_bytes: u64,
    /// Memory that programs could still use, in bytes.
    pub memory_available_bytes: u64,
    /// Swap of the node, in bytes.
    pub swap_total_bytes: u64,
    /// Swap in use, in bytes.
    pub swap_used_bytes: u64,
    /// Load averages over 1, 5 and 15 minutes, times 100.
    pub load_x100: [u32; 3],
    /// Seconds since boot.
    pub uptime_seconds: u64,
    /// The disk of the tenants' files: size and bytes in use.
    pub disk_total_bytes: u64,
    /// Bytes in use on that disk.
    pub disk_used_bytes: u64,
}

/// One container's usage, as counters since it started and levels now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerMetrics {
    /// The container's name (`bp-<account>-<role>`).
    pub name: String,
    /// Memory in use, in bytes.
    pub memory_bytes: u64,
    /// The memory limit, in bytes.
    pub memory_limit_bytes: u64,
    /// CPU time used since it started, in nanoseconds.
    pub cpu_nanos: u64,
    /// Processes right now.
    pub pids: u64,
    /// Bytes received since it started.
    pub net_rx_bytes: u64,
    /// Bytes sent since it started.
    pub net_tx_bytes: u64,
    /// Bytes read from disk since it started.
    pub io_read_bytes: u64,
    /// Bytes written to disk since it started.
    pub io_write_bytes: u64,
}

/// Most containers in one metrics report.
pub const MAX_METRIC_CONTAINERS: usize = 5000;

/// One of an account's containers, as listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerItem {
    /// Its name (`bp-<account>-<role>`).
    pub name: String,
    /// What it is for (`web`, `php`).
    pub role: String,
    /// The image.
    pub image: String,
    /// `running`, `exited`, `restarting`, `created`, ...
    pub state: String,
    /// Docker's words ("Up 3 hours").
    pub status: String,
    /// When it was made (Unix seconds).
    pub created: i64,
}

/// A core service of the node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceStatus {
    /// `database`, `phpmyadmin`, `sftp`, `caddy` or `agent`.
    pub name: String,
    /// `running`, `stopped`, `unreachable` or `not_installed`.
    pub state: String,
    /// Docker's words, or what went wrong.
    pub detail: String,
}

/// The core services a node can name (and only these can be restarted or have their log read).
pub const NODE_SERVICES: [&str; 3] = ["database", "phpmyadmin", "sftp"];

/// A process inside one of an account's containers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessItem {
    /// The container it runs in (`bp-<account>-<role>`).
    pub container: String,
    /// Its id inside the container.
    pub pid: u32,
    /// The user id it runs as.
    pub uid: u32,
    /// `R` running, `S` sleeping, `D` waiting for the disk, `Z` zombie, ...
    pub state: String,
    /// The command line, clipped.
    pub command: String,
}

/// How a signalled process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalOutcome {
    /// It ended after SIGTERM.
    Exited,
    /// It did not end and was killed (SIGKILL).
    Killed,
    /// There was no such process (any more).
    NotFound,
    /// It is still there (SIGKILL was not asked for, or did not work).
    StillRunning,
}

/// What a node says about itself (T10.15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerInfo {
    /// The machine's name.
    pub hostname: String,
    /// The distribution (`PRETTY_NAME` of os-release).
    pub os: String,
    /// The kernel release.
    pub kernel: String,
    /// The time zone (`Area/City`, or empty when it is not known).
    pub timezone: String,
    /// Seconds since boot.
    pub uptime_seconds: u64,
    /// Memory of the machine, in bytes.
    pub memory_total_bytes: u64,
    /// Memory programs could still use, in bytes.
    pub memory_available_bytes: u64,
    /// Swap in all, in bytes.
    pub swap_total_bytes: u64,
    /// Swap in use, in bytes.
    pub swap_used_bytes: u64,
    /// The settings of the SSH server that matter for safety, as it will use them
    /// (`name value`), read-only.
    pub ssh: Vec<(String, String)>,
}

/// The SSH access of a node (System menu).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SshAccess {
    /// Every port the SSH server listens on.
    pub ports: Vec<u16>,
    /// `PermitRootLogin`: `yes`, `no`, `prohibit-password` or `forced-commands-only`.
    pub root_login: String,
    /// Whether passwords may be used to log in.
    pub password_authentication: bool,
    /// Whether keys may be used to log in.
    pub pubkey_authentication: bool,
    /// The keys that may log in as root.
    pub keys: Vec<SshAuthorizedKey>,
    /// The text of `sshd_config` (at most 64 KiB).
    pub config: String,
}

/// One key that may log in as root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SshAuthorizedKey {
    /// `ssh-ed25519`, `ssh-rsa`, ...
    pub kind: String,
    /// `SHA256:` and the key's SHA-256 in base64, as `ssh-keygen -l` prints it.
    pub fingerprint: String,
    /// The comment at the end of the line (often `user@machine`).
    pub comment: String,
}

/// Most runs in one report.
pub const MAX_CRON_RUNS_PER_REPORT: usize = 500;

/// Most databases in one size report.
pub const MAX_DB_SIZES_PER_REPORT: usize = 5000;
/// Most databases (and users) one [`Operation::MigrateTenantDatabases`] carries.
pub const MAX_MIGRATED_DATABASES: usize = 500;
/// Most queued messages one mail report carries.
pub const MAX_MAIL_QUEUE_ITEMS: usize = 500;

/// Most address ranges allowed to reach one node's database server in all.
pub const MAX_DB_REMOTE_ACCESS: usize = 2000;

/// Most remote addresses one database user may have.
pub const MAX_DB_REMOTE_HOSTS: usize = 20;

/// Longest php.ini file the control plane sends.
pub const MAX_PHP_INI_BYTES: usize = 4096;

/// Most PHP versions besides the account's own that one tenant may run.
pub const MAX_EXTRA_PHP_VERSIONS: usize = 3;

/// Longest web server configuration of one domain.
pub const MAX_VHOST_CONFIG_BYTES: usize = 32 * 1024;

/// Most uploaded certificates one tenant may have on a node.
pub const MAX_CUSTOM_CERTIFICATES_PER_TENANT: usize = 100;

/// A certificate the web server holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateInfo {
    /// The name it is for (lower-case ASCII).
    pub host: String,
    /// Who issued it (the issuer's common name).
    pub issuer: String,
    /// Valid from (Unix seconds).
    pub not_before: i64,
    /// Valid until (Unix seconds).
    pub not_after: i64,
}

/// Most certificates one report holds.
pub const MAX_CERTIFICATES_PER_REPORT: usize = 4000;

/// Most sites one tenant may have on a node.
pub const MAX_SITES_PER_TENANT: usize = 200;
/// Most names in all of a tenant's sites.
pub const MAX_HOSTS_PER_TENANT: usize = 500;
/// Most redirects in all of a tenant's sites.
pub const MAX_REDIRECTS_PER_TENANT: usize = 500;

// ------------------------------------------------------------ skeleton files

/// One default file of a new tenant: text, at a plain relative path below its
/// docroot. At most [`SKELETON_MAX_FILES`] files, [`SKELETON_MAX_FILE_BYTES`]
/// each and [`SKELETON_MAX_TOTAL_BYTES`] together: they travel inside one
/// operation, which is a single bounded message end to end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkeletonFile {
    /// Path with `/` between names, for example `css/site.css`.
    pub path: String,
    /// The file's text.
    pub content: String,
}

/// Most skeleton files.
pub const SKELETON_MAX_FILES: usize = 20;
/// Largest skeleton file, in bytes.
pub const SKELETON_MAX_FILE_BYTES: usize = 16 * 1024;
/// Most bytes in all skeleton files together.
pub const SKELETON_MAX_TOTAL_BYTES: usize = 24 * 1024;
/// Deepest nesting of a skeleton file.
pub const SKELETON_MAX_DEPTH: usize = 6;
/// Longest single name.
pub const SKELETON_MAX_NAME: usize = 64;

/// Checks one skeleton path: relative, plain names (letters, digits, `.`, `_`,
/// `-`), no `.` or `..`, limited depth.
///
/// # Errors
/// What is wrong, for the person who wrote it.
pub fn check_skeleton_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() || path.len() > SKELETON_MAX_DEPTH * (SKELETON_MAX_NAME + 1) {
        return Err("the path must be 1 to 450 characters");
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > SKELETON_MAX_DEPTH {
        return Err("the path is nested too deeply (at most 6 levels)");
    }
    for part in parts {
        let plain = !part.is_empty()
            && part.len() <= SKELETON_MAX_NAME
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if !plain {
            return Err(
                "names may hold letters, digits, dot, dash and underscore only (no empty names, `.` or `..`)",
            );
        }
    }
    Ok(())
}

/// Checks a whole list the way the node will. Within these limits the JSON of a
/// set-up operation stays under 56 KiB, inside the 64 KiB of the operations
/// ledger and of the privileged helper's request line.
///
/// # Errors
/// What is wrong, for the person who wrote it.
pub fn check_skeleton(files: &[SkeletonFile]) -> Result<(), &'static str> {
    if files.len() > SKELETON_MAX_FILES {
        return Err("too many files (at most 20)");
    }
    let mut total = 0usize;
    for (i, file) in files.iter().enumerate() {
        check_skeleton_path(&file.path)?;
        if file.content.len() > SKELETON_MAX_FILE_BYTES {
            return Err("a file is larger than 16 KiB");
        }
        // Control characters other than line breaks and tabs can grow six-fold
        // when written as JSON, which would break the size promise below.
        if file
            .content
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err("a file holds control characters (only line breaks and tabs are allowed)");
        }
        total += file.content.len();
        if total > SKELETON_MAX_TOTAL_BYTES {
            return Err("the files are larger than 24 KiB together");
        }
        for other in &files[..i] {
            if other.path == file.path {
                return Err("a path is listed twice");
            }
            let (a, b) = (other.path.as_str(), file.path.as_str());
            if (b.len() > a.len() && b.starts_with(a) && b.as_bytes()[a.len()] == b'/')
                || (a.len() > b.len() && a.starts_with(b) && a.as_bytes()[b.len()] == b'/')
            {
                return Err("a path is both a file and a directory of another file");
            }
        }
    }
    Ok(())
}

// -------------------------------------------------------------- operations

/// What the control plane asks a node to do. Every variant is a narrow,
/// typed action; there is no variant that runs a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Operation {
    /// Liveness check; the agent answers [`Output::Pong`] with the same nonce.
    Ping {
        /// Echoed back.
        nonce: u64,
    },
    /// Ask the node to describe itself; answered with [`Output::Capabilities`].
    ReportCapabilities,
    /// Prepare a node for a new tenant: its directories (owned by its id), its
    /// private network and its files volume. Safe to repeat. Answered with
    /// [`Output::TenantCreated`].
    CreateTenant {
        /// The tenant's id (also its directory name).
        tenant_id: Uuid,
        /// The tenant's system name (`a-z`, `0-9`, 3-32 characters).
        username: String,
        /// The tenant's numeric user id (10000..59999) on this node.
        uid: u32,
    },
    /// Limit what a tenant may store: bytes and number of files. `None` is
    /// unlimited. The node must support project quotas
    /// ([`Capabilities::storage_quota`]), else the operation fails as unsupported.
    SetStorageQuota {
        /// The tenant's id.
        tenant_id: Uuid,
        /// The tenant's numeric user id.
        uid: u32,
        /// Most bytes.
        disk_bytes: Option<u64>,
        /// Most files and directories.
        inodes: Option<u64>,
    },
    /// Create and start a tenant's containers (web server + PHP) from the
    /// stack template, and wait until they run. Needs [`Operation::CreateTenant`]
    /// first. Safe to repeat: containers that exist are only started. Answered
    /// with [`Output::StackProvisioned`].
    ProvisionTenantStack {
        /// The tenant's id.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// The tenant's numeric user id on this node.
        uid: u32,
        /// CPU the plan gives the stack, in thousandths of a core (`None` = unlimited).
        cpu_millis: Option<u32>,
        /// Memory the plan gives the stack, in bytes (`None` = unlimited).
        memory_bytes: Option<u64>,
        /// Processes the plan gives the stack (`None` = unlimited).
        pids: Option<u32>,
    },
    /// Make the tenant's PHP containers match: the account's version in its main
    /// PHP container, one more container for each extra version, none for any
    /// other. A container that runs another version than asked is replaced (the
    /// files stay), new ones are started and waited for, and the plan's PHP share
    /// is divided between them. The web server is not touched (its configuration
    /// says which container each site uses). Safe to repeat. Answered with
    /// [`Output::PhpVersionsSet`].
    SetPhpVersions {
        /// The tenant's id.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// The tenant's numeric user id on this node.
        uid: u32,
        /// CPU the plan gives the stack, in thousandths of a core (`None` = unlimited).
        cpu_millis: Option<u32>,
        /// Memory the plan gives the stack, in bytes (`None` = unlimited).
        memory_bytes: Option<u64>,
        /// Processes the plan gives the stack (`None` = unlimited).
        pids: Option<u32>,
        /// The version of the main PHP container, `8.4`.
        php_version: String,
        /// More versions to run next to it (at most [`MAX_EXTRA_PHP_VERSIONS`]).
        extra_php: Vec<String>,
    },
    /// Make sure the node's shared database server runs: its volume, its root
    /// password (made on first use and kept in the agent's secret store, never
    /// sent anywhere), and the container, published on the node's loopback address
    /// only. Safe to repeat. Answered with [`Output::DbServerReady`].
    EnsureDbServer,
    /// Make a database in the node's shared database server (utf8mb4). The name
    /// is checked as a plain identifier and put in the statement quoted; nothing
    /// is concatenated. Safe to repeat. Answered with [`Output::DatabaseCreated`].
    CreateDatabase {
        /// The whole name, `alice_shop`.
        name: String,
        /// Which server (the node's shared MariaDB when left out).
        #[serde(default, skip_serializing_if = "DbEngine::is_mysql")]
        engine: DbEngine,
        /// The tenant whose own server it is (the node's shared one when left out).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dedicated: Option<String>,
    },
    /// Remove a database and everything in it. Safe to repeat. Answered with
    /// [`Output::DatabaseDropped`].
    DropDatabase {
        /// The whole name.
        name: String,
        /// Which server (the node's shared MariaDB when left out).
        #[serde(default, skip_serializing_if = "DbEngine::is_mysql")]
        engine: DbEngine,
        /// The tenant whose own server it is (the node's shared one when left out).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dedicated: Option<String>,
    },
    /// Make a database user, or change its password, for connections from the
    /// tenant's own network only (never from `%`). The user is created with the
    /// password; for a user that exists the password is changed when one is given.
    /// Safe to repeat. Answered with [`Output::DbUserSet`].
    UpsertDbUser {
        /// The tenant's system name (its network says where the user may connect from).
        username: String,
        /// The user's whole name, `alice_web`.
        name: String,
        /// The password; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<DbPassword>,
        /// Which server (the node's shared MariaDB when left out).
        #[serde(default, skip_serializing_if = "DbEngine::is_mysql")]
        engine: DbEngine,
        /// The tenant whose own server it is (the node's shared one when left out).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dedicated: Option<String>,
    },
    /// Give a user an access level in one database (replacing what it had there):
    /// `read_only`, `read_write` or `all`, each a fixed list of privileges, never
    /// `GRANT OPTION` or anything outside that one database. Safe to repeat.
    /// Answered with [`Output::DbGranted`].
    GrantDb {
        /// The database's whole name.
        database: String,
        /// The user's whole name.
        user: String,
        /// The access level.
        access: String,
        /// Which server (the node's shared MariaDB when left out).
        #[serde(default, skip_serializing_if = "DbEngine::is_mysql")]
        engine: DbEngine,
        /// The tenant whose own server it is (the node's shared one when left out).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dedicated: Option<String>,
    },
    /// Take a user's access to one database away. Safe to repeat. Answered with
    /// [`Output::DbRevoked`].
    RevokeDb {
        /// The database's whole name.
        database: String,
        /// The user's whole name.
        user: String,
        /// Which server (the node's shared MariaDB when left out).
        #[serde(default, skip_serializing_if = "DbEngine::is_mysql")]
        engine: DbEngine,
        /// The tenant whose own server it is (the node's shared one when left out).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dedicated: Option<String>,
    },
    /// Set the addresses a database user may connect from besides its tenant's own
    /// network (IPv4 `a.b.c.d/n`, at most [`MAX_DB_REMOTE_HOSTS`], none wider than
    /// a /8): the user gets the same password and the same grants for each, and
    /// loses the ones not listed. An empty list ends remote access. Safe to
    /// repeat. Answered with [`Output::DbRemoteHostsSet`].
    SetDbRemoteHosts {
        /// The tenant's system name.
        username: String,
        /// The user's whole name.
        name: String,
        /// The addresses or ranges.
        cidrs: Vec<String>,
        /// Which server (the node's shared MariaDB when left out).
        #[serde(default, skip_serializing_if = "DbEngine::is_mysql")]
        engine: DbEngine,
        /// The tenant whose own server it is (the node's shared one when left out).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dedicated: Option<String>,
    },
    /// Set which addresses may reach the node's database server from outside the
    /// node: the union of every remote address of every user on the node (IPv4
    /// `a.b.c.d/n`, none wider than a /8, at most [`MAX_DB_REMOTE_ACCESS`]). The
    /// node's packet filter is set first (through the privileged helper), and only
    /// then, when there is anything to allow, is the server's public port opened.
    /// An empty list closes the port for everybody. Safe to repeat. Answered with
    /// [`Output::DbRemoteAccessSet`].
    SetDbRemoteAccess {
        /// The allowed addresses and ranges.
        cidrs: Vec<String>,
        /// Which server (the node's shared MariaDB when left out).
        #[serde(default, skip_serializing_if = "DbEngine::is_mysql")]
        engine: DbEngine,
    },
    /// Remove a database user (from every address it could connect from). Safe to
    /// repeat. Answered with [`Output::DbUserDropped`].
    DropDbUser {
        /// The user's whole name.
        name: String,
        /// Which server (the node's shared MariaDB when left out).
        #[serde(default, skip_serializing_if = "DbEngine::is_mysql")]
        engine: DbEngine,
        /// The tenant whose own server it is (the node's shared one when left out).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dedicated: Option<String>,
    },
    /// Measure the size of every database on the node's shared server (data and
    /// indexes). Answered with [`Output::DbSizes`].
    ReportDbSizes,
    /// Make sure the node's mail server (Stalwart) runs and is set up: its volumes, its
    /// administrator (made on first use, kept in the agent's secret store, never sent), the
    /// certificate for `hostname` (the one given, or a self-signed one until there is a real
    /// one) and the container, with the mail ports published. Safe to repeat. Answered with
    /// [`Output::MailServerReady`].
    EnsureMailServer {
        /// The name the server says it is, `mail.example.com`.
        hostname: String,
        /// The certificate chain in PEM, with its key; the key is absent in the ledger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        certificate_pem: Option<String>,
        /// The certificate's private key in PEM.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        private_key_pem: Option<DbPassword>,
    },
    /// Make a domain known to the mail server (its DKIM keys are made by the server), set its
    /// catch-all address and its outgoing limit, and answer with the DNS records it needs.
    /// Safe to repeat. Answered with [`Output::MailDomainSet`].
    SetMailDomain {
        /// The domain.
        domain: String,
        /// Where mail to an address the domain does not have goes (a full address), if anywhere.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        catch_all: Option<String>,
        /// Most messages the domain may send out in an hour (none: no limit).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        per_hour: Option<u32>,
        /// Hold the domain's outgoing mail (an abuse stop): a limit of one a day.
        #[serde(default)]
        held: bool,
    },
    /// Remove a domain from the mail server, with its mailboxes and lists. Safe to repeat.
    /// Answered with [`Output::MailDomainRemoved`].
    RemoveMailDomain {
        /// The domain.
        domain: String,
    },
    /// Make a mailbox, or change its password, quota or other addresses. Safe to repeat.
    /// Answered with [`Output::MailboxSet`].
    UpsertMailbox {
        /// The domain.
        domain: String,
        /// The local part.
        local: String,
        /// The password; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<DbPassword>,
        /// How much it may hold, in bytes (past it mail is refused).
        quota_bytes: u64,
        /// Other local parts that reach the same mailbox.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        aliases: Vec<String>,
    },
    /// Remove a mailbox with its mail. Safe to repeat. Answered with [`Output::MailboxDropped`].
    DropMailbox {
        /// The domain.
        domain: String,
        /// The local part.
        local: String,
    },
    /// Set the domain's forwarders: the whole list (what is not in it is removed). Safe to
    /// repeat. Answered with [`Output::MailForwardersSet`].
    SetMailForwarders {
        /// The domain.
        domain: String,
        /// The forwarders.
        forwarders: Vec<MailForwarder>,
    },
    /// Turn a mailbox's automatic reply on or off. Safe to repeat. Answered with
    /// [`Output::MailAutoresponderSet`].
    SetMailAutoresponder {
        /// The domain.
        domain: String,
        /// The local part.
        local: String,
        /// Whether it replies.
        enabled: bool,
        /// The subject of the reply.
        subject: String,
        /// The text of the reply.
        body: String,
        /// Only from this time (Unix seconds).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<i64>,
        /// Only until this time (Unix seconds).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until: Option<i64>,
    },
    /// What the mail server holds: every mailbox's size and the outgoing queue (at most
    /// [`MAX_MAIL_QUEUE_ITEMS`] messages). Answered with [`Output::MailReport`].
    ReportMail,
    /// Retry the queued messages now (`flush`), or remove them (`delete`); `ids` limits it to
    /// those messages (all of the queue when empty, for `flush`). Answered with
    /// [`Output::MailQueueChanged`].
    MailQueueAction {
        /// `flush` or `delete`.
        action: String,
        /// The messages' ids.
        ids: Vec<String>,
    },
    /// Make sure the node's PostgreSQL server runs: its volume, its superuser password
    /// (made on first use, kept in the agent's secret store, never sent anywhere), and
    /// the container, published on the node's loopback address only. Safe to repeat.
    /// Answered with [`Output::DbServerReady`].
    EnsurePgServer,
    /// [`Operation::ReportDbSizes`] for the node's PostgreSQL server.
    ReportPgSizes,
    /// Move a tenant's databases of one kind between the node's shared server and the tenant's
    /// own server (T07.13): the target server is made if it is not there, the users are made
    /// there with the same passwords (copied on the node; no password is sent), each database
    /// is dumped from the source, loaded into the target (a copy of an earlier try is replaced),
    /// and the grants and remote addresses are set again. The source is left as it was: the
    /// control plane removes it when it has switched. Safe to repeat. Answered with
    /// [`Output::DbMigrated`].
    MigrateTenantDatabases {
        /// The tenant's system name.
        username: String,
        /// Which kind of server.
        engine: DbEngine,
        /// Whether the target is the tenant's own server (otherwise the shared one).
        to_dedicated: bool,
        /// The databases' whole names (at most [`MAX_MIGRATED_DATABASES`]).
        databases: Vec<String>,
        /// The users' whole names (at most [`MAX_MIGRATED_DATABASES`]).
        users: Vec<String>,
        /// Who may do what where.
        grants: Vec<MigrateGrant>,
        /// The addresses users may connect from besides their own network.
        remote_hosts: Vec<MigrateRemote>,
    },
    /// Remove a tenant's own database server of one kind, with its data. Safe to repeat.
    /// Answered with [`Output::DedicatedDbRemoved`].
    RemoveDedicatedDb {
        /// The tenant's system name.
        username: String,
        /// Which kind of server.
        engine: DbEngine,
    },
    /// Fetch an SQL dump from the control plane and run it in one of the tenant's
    /// databases, as a temporary account that has rights in that database only. The
    /// file is checked (size and SHA-256) before anything runs, may be gzip
    /// compressed, and is read as a stream; the run stops at the first statement the
    /// server refuses. Answered with [`Output::SqlImported`]; how far it has come can
    /// be asked with [`Operation::ReportImportProgress`] meanwhile.
    ImportSql {
        /// The tenant's system name.
        username: String,
        /// The database's whole name.
        database: String,
        /// This import's id.
        import_id: Uuid,
        /// Where the dump is fetched from (`https`, or `http` for loopback).
        url: String,
        /// The dump's size in bytes, as uploaded (compressed, if it is).
        size_bytes: u64,
        /// Its SHA-256, 64 hex digits.
        sha256: String,
        /// Whether it is gzip compressed.
        gzip: bool,
        /// The credential for the download; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<DbPassword>,
        /// Which server (the node's shared MariaDB when left out).
        #[serde(default, skip_serializing_if = "DbEngine::is_mysql")]
        engine: DbEngine,
        /// The tenant whose own server it is (the node's shared one when left out).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dedicated: Option<String>,
    },
    /// Dumps one database (structure, data, views, triggers, routines, events),
    /// gzip-compresses it and sends it to the control plane in chunks of at most
    /// 8 MiB (`PUT {url}/chunks/{n}`, then `POST {url}/complete`), all with the
    /// token as a Bearer credential. Answered with [`Output::SqlExported`].
    ExportSql {
        /// The tenant's system name.
        username: String,
        /// The database's whole name.
        database: String,
        /// This export's id.
        export_id: Uuid,
        /// Where the dump goes (`https`, or `http` for loopback).
        url: String,
        /// The credential for the upload; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<DbPassword>,
        /// Which server (the node's shared MariaDB when left out).
        #[serde(default, skip_serializing_if = "DbEngine::is_mysql")]
        engine: DbEngine,
        /// The tenant whose own server it is (the node's shared one when left out).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dedicated: Option<String>,
    },
    /// Sets (or, with no host, removes) the name phpMyAdmin is served under: the
    /// web server routes that name to the node's phpMyAdmin container. Answered with
    /// [`Output::PhpMyAdminHostSet`].
    #[serde(rename = "set_phpmyadmin_host")]
    SetPhpMyAdminHost {
        /// The host name, or none to switch it off.
        host: Option<String>,
    },
    /// Lets one sign-in to phpMyAdmin happen: makes sure phpMyAdmin runs, makes a
    /// temporary account with all rights on this one database and keeps its
    /// credentials where only the one-time sign-in page reads them (under the
    /// SHA-256 of the token the client holds; the token itself never reaches the
    /// node). The account is removed when the session's time is up. Answered with
    /// [`Output::PhpMyAdminOpened`].
    #[serde(rename = "open_phpmyadmin")]
    OpenPhpMyAdmin {
        /// The tenant's system name.
        username: String,
        /// The database's whole name.
        database: String,
        /// This sign-in's id.
        session_id: Uuid,
        /// SHA-256 of the sign-in token, 64 lower-case hex digits.
        token_hash: String,
        /// How long the session lasts, in minutes (5..=720).
        session_minutes: u32,
    },
    /// Sets (or, with no host, removes) the name webmail is served under: the web server routes
    /// that name to the node's webmail container. Answered with [`Output::WebmailHostSet`].
    #[serde(rename = "set_webmail_host")]
    SetWebmailHost {
        /// The host name, or none to switch it off.
        host: Option<String>,
    },
    /// Lets one sign-in to webmail happen: makes sure webmail runs and keeps the credentials
    /// of a sign-in as `address` (the mail server's administrator signing in as that mailbox)
    /// where only the one-time sign-in page reads them, under the SHA-256 of the token the
    /// client holds (the token itself never reaches the node). Answered with
    /// [`Output::WebmailOpened`].
    #[serde(rename = "open_webmail")]
    OpenWebmail {
        /// The mailbox's whole address.
        address: String,
        /// SHA-256 of the sign-in token, 64 lower-case hex digits.
        token_hash: String,
    },
    /// Fetches an uploaded file from the control plane (resuming a cut download, size
    /// checked) and puts it in the tenant's files as the tenant's own user, atomically
    /// and, unless `overwrite`, never over something. Answered with [`Output::FileFetched`].
    FetchFile {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's numeric user id.
        uid: u32,
        /// This transfer's id.
        transfer_id: Uuid,
        /// Where it goes, relative to the tenant's files.
        dest: String,
        /// Where the file is fetched from (`https`, or `http` for loopback).
        url: String,
        /// Its size in bytes.
        size_bytes: u64,
        /// Replace a file that is already there.
        #[serde(default)]
        overwrite: bool,
        /// The credential for the download; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<DbPassword>,
    },
    /// Reads a file of the tenant's (as the tenant's own user) and sends it to the
    /// control plane in chunks of at most 8 MiB (`PUT {url}/chunks/{n}`, then
    /// `POST {url}/complete`) with the token as a Bearer credential. Answered with
    /// [`Output::FileSent`].
    SendFile {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's numeric user id.
        uid: u32,
        /// This transfer's id.
        transfer_id: Uuid,
        /// The file, relative to the tenant's files.
        path: String,
        /// Where it goes (`https`, or `http` for loopback).
        url: String,
        /// Refuse a file bigger than this.
        max_bytes: u64,
        /// The credential for the upload; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<DbPassword>,
    },
    /// Makes a backup of a hosting account: its files (a tar, compressed, encrypted, read
    /// as the tenant's own user), one consistent dump per database, the DNS records and
    /// the setup data, then the manifest; every part goes to the destination as it is
    /// made. The key and the destination's credentials are not in the ledger. Answered
    /// with [`Output::BackupCreated`].
    CreateBackup {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's numeric user id.
        uid: u32,
        /// This backup's id (the folder it is stored in at the destination).
        backup_id: Uuid,
        /// The hosting account's system name.
        account: String,
        /// Where it goes.
        target: BackupTarget,
        /// The destination's secret part, as JSON text; absent in the ledger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credentials: Option<DbPassword>,
        /// The key the parts are encrypted with (64 hex digits); absent in the ledger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<DbPassword>,
        /// The whole names of the databases to dump.
        databases: Vec<String>,
        /// Database users whose password hashes go into the setup data (for a transfer).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        db_users: Vec<BackupDbUser>,
        /// Pack only the files made or changed after this time (Unix seconds): the last pass of
        /// a transfer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        files_since: Option<i64>,
        /// The DNS records of the account's domains (stored as `dns.json`).
        dns: serde_json::Value,
        /// What is needed to set the account up again (stored as `meta.json`).
        meta: serde_json::Value,
        /// When it was asked for (Unix seconds).
        created_at: i64,
    },
    /// Checks a backup and, unless `verify_only`, puts all of it or a part of it back.
    ///
    /// First the manifest is opened and every part that is needed is read, hashed and
    /// compared with the manifest: one that is missing, changed or cut stops everything
    /// before anything is touched. Then, when `snapshot` is set, the present state of what
    /// will be replaced (the files, and each database to restore) is stored at the
    /// destination under `pre-<restore_id>/`, so a restore can be taken back. Then the files
    /// (all, or the `folder`) are written over the present ones (files that are not in the
    /// backup stay) and each database is replaced by its dump. Answered with
    /// [`Output::BackupRestored`].
    RestoreBackup {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's numeric user id.
        uid: u32,
        /// The backup to restore from.
        backup_id: Uuid,
        /// This restore's id (names the snapshot).
        restore_id: Uuid,
        /// The hosting account's system name (must be the one in the manifest).
        account: String,
        /// Where the backup is.
        target: BackupTarget,
        /// The destination's secret part, as JSON text; absent in the ledger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credentials: Option<DbPassword>,
        /// The key the parts were encrypted with (64 hex digits); absent in the ledger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<DbPassword>,
        /// Put the files back.
        files: bool,
        /// Only this folder of the files (relative); all of them when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        folder: Option<String>,
        /// The whole names of the databases to put back.
        databases: Vec<String>,
        /// Read the DNS records out of the backup and answer with them (the panel puts
        /// them back); nothing is changed on the node for this.
        #[serde(default)]
        dns: bool,
        /// Store the present state before replacing it.
        snapshot: bool,
        /// Only check the backup; change nothing.
        verify_only: bool,
        /// Make a database that is not there before it is filled (a transfer to a new node).
        #[serde(default)]
        create_databases: bool,
        /// Make the database users the backup carries, with their passwords (a transfer).
        #[serde(default)]
        restore_db_users: bool,
    },
    /// Removes everything a backup stored at its destination (`<backup id>/`; with
    /// `snapshots`, also the `pre-*` folders of its restores are not touched). Safe to
    /// repeat: what is gone is not an error. Answered with [`Output::BackupDeleted`].
    DeleteBackup {
        /// The backup.
        backup_id: Uuid,
        /// Where it is.
        target: BackupTarget,
        /// The destination's secret part, as JSON text; absent in the ledger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credentials: Option<DbPassword>,
    },
    /// Packs a backup into **one file** at the destination (`exports/<export_id>.tar`): the
    /// `header` (made by the control plane: who, what, and the key wrapped with a passphrase),
    /// the manifest and every part, as stored. The node never sees the key. Answered with
    /// [`Output::BackupExported`].
    ExportBackup {
        /// The backup.
        backup_id: Uuid,
        /// This export's id (names the file).
        export_id: Uuid,
        /// Where the backup is, and where the file goes.
        target: BackupTarget,
        /// The destination's secret part, as JSON text; absent in the ledger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credentials: Option<DbPassword>,
        /// The header text (`export.json`).
        header: String,
    },
    /// Unpacks an export file into a backup folder (`<backup_id>/`) at a destination, so that
    /// the backup can be restored as any other. Refuses to replace a backup that is there.
    /// Answered with [`Output::ExportImported`].
    ImportExport {
        /// The export file's name at the destination (`exports/<id>.tar`).
        object: String,
        /// The backup id the export carries (the folder it is unpacked into).
        backup_id: Uuid,
        /// Where the file is and where the backup goes.
        target: BackupTarget,
        /// The destination's secret part, as JSON text; absent in the ledger.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credentials: Option<DbPassword>,
    },
    /// Reads the end of one of an account's logs: the newest `tail` lines at or after
    /// `since_ms`, optionally only those that contain a text. Only the three logs of
    /// [`LogSource`], only the account's own containers. Answered with [`Output::LogLines`].
    ReadLog {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// Which log.
        source: LogSource,
        /// Lines before this time (Unix milliseconds) are left out; `0` for none.
        since_ms: i64,
        /// At most this many lines (1 to [`MAX_LOG_LINES`]), the newest.
        tail: u32,
        /// Only lines that contain this text (case-insensitive, at most 200 characters).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        contains: Option<String>,
    },
    /// Counts an account's web requests since a time, from the web container's access log:
    /// how many, how many bytes, by kind of answer, per hour, and the most asked pages.
    /// Answered with [`Output::Traffic`].
    ReadTraffic {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// Requests before this time (Unix milliseconds) are not counted.
        since_ms: i64,
        /// How many of the most asked pages to name (1 to [`MAX_TRAFFIC_PATHS`]).
        top: u32,
    },
    /// The requests the web application firewall stopped for these names since a time, the
    /// newest first, from the web server's firewall log. Answered with [`Output::WafEvents`].
    ReadWafEvents {
        /// The tenant.
        tenant_id: Uuid,
        /// The names (lower-case) of the tenant's sites; at most [`MAX_WAF_HOSTS`].
        hosts: Vec<String>,
        /// Events before this time (Unix milliseconds) are left out.
        since_ms: i64,
        /// At most this many (1 to [`MAX_WAF_EVENTS`]).
        limit: u32,
    },
    /// The state of the node's core services that run as containers (database server,
    /// phpMyAdmin, file server) and of the agent itself. Answered with [`Output::Services`].
    ReportServices,
    /// The state of the node's web server (Caddy), asked through its admin address; only a
    /// node that drives one has this operation. Answered with [`Output::Services`].
    ReportWebService,
    /// Restarts one core service (one of [`NODE_SERVICES`]: nothing else can be named).
    /// Answered with [`Output::ContainerRestarted`].
    RestartNodeService {
        /// The service.
        name: String,
    },
    /// The end of a core service's log (one of [`NODE_SERVICES`]): the newest `tail` lines.
    /// Answered with [`Output::LogLines`].
    ReadServiceLog {
        /// The service.
        name: String,
        /// At most this many lines (1 to [`MAX_LOG_LINES`]).
        tail: u32,
    },
    /// The processes inside the containers of one account (only those: the ones that carry its
    /// id). Answered with [`Output::Processes`].
    ListProcesses {
        /// The tenant.
        tenant_id: Uuid,
    },
    /// Sends SIGTERM to one process of an account (SIGKILL when `kill` is set; or, with
    /// `escalate`, SIGKILL after ten seconds if it is still there). Process 1 of a container
    /// cannot be signalled (restart the service instead). Answered with
    /// [`Output::ProcessSignalled`].
    SignalProcess {
        /// The tenant.
        tenant_id: Uuid,
        /// The container (one of the account's).
        container: String,
        /// The process, from 2 up.
        pid: u32,
        /// SIGKILL at once.
        kill: bool,
        /// After SIGTERM, wait up to ten seconds and then SIGKILL.
        escalate: bool,
    },
    /// The machine's name, system, time zone, memory, swap, uptime and the SSH server's
    /// safety settings. Answered with [`Output::ServerInfo`].
    ReportServerInfo,
    /// Sets the machine's time zone (a zone file of the machine). Answered with
    /// [`Output::ServerActionDone`].
    SetTimezone {
        /// `Area/City`.
        zone: String,
    },
    /// Makes the swap file this many MiB big (0 takes it away). Answered with
    /// [`Output::ServerActionDone`].
    SetSwap {
        /// MiB, 0 to 32768.
        megabytes: u32,
    },
    /// Drops the page cache. Answered with [`Output::ServerActionDone`].
    DropCaches,
    /// Reboots the machine: the answer comes just before it goes down. Answered with
    /// [`Output::ServerActionDone`].
    RebootNode {
        /// Restart at once, without waiting for the services to stop.
        #[serde(default)]
        hard: bool,
    },
    /// Moves what is swapped out back into memory and turns swap on again; refused when there is not
    /// enough free memory. Answered with [`Output::ServerActionDone`].
    DropSwap,
    /// The SSH server's settings as it will use them, the keys that may log in as root and the
    /// text of `sshd_config`. Answered with [`Output::SshAccess`].
    ReadSshAccess,
    /// Changes the SSH server's safety settings; what is absent stays as it is. The node checks the
    /// result, refuses a change that would lock everyone out, and puts the old settings back when
    /// SSH does not come up. Answered with [`Output::ServerActionDone`].
    SetSshSettings {
        /// The port: 22, or 1024 to 65535 (not one the panel uses).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        /// `yes`, `no`, `prohibit-password` or `forced-commands-only`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        root_login: Option<String>,
        /// Whether passwords may be used to log in.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password_authentication: Option<bool>,
        /// Whether keys may be used to log in.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pubkey_authentication: Option<bool>,
    },
    /// Authorises one public key (`type key [comment]`, no options) for root over SSH. Answered
    /// with [`Output::ServerActionDone`].
    AddSshKey {
        /// The key line.
        key: String,
    },
    /// Takes one authorised key away, named by its fingerprint (`SHA256:...`). Answered with
    /// [`Output::ServerActionDone`].
    RemoveSshKey {
        /// The fingerprint.
        fingerprint: String,
    },
    /// Replaces `sshd_config` (checked first, kept until SSH has come back with it). Answered with
    /// [`Output::ServerActionDone`].
    WriteSshdConfig {
        /// The whole text, at most 64 KiB.
        text: String,
    },
    /// Lists the containers of one account (and only those: the ones that carry its id).
    /// Answered with [`Output::Containers`].
    ListContainers {
        /// The tenant.
        tenant_id: Uuid,
    },
    /// Restarts one container of an account: it must carry the account's id, or nothing is
    /// done. Answered with [`Output::ContainerRestarted`].
    RestartContainer {
        /// The tenant.
        tenant_id: Uuid,
        /// The container's name.
        name: String,
    },
    /// Reads the node's and every running container's usage now (cheap: one statistics read
    /// per container, a few at a time). Answered with [`Output::Metrics`].
    CollectMetrics,
    /// Replaces the cron jobs of an account on this node; the node's scheduler runs them
    /// from now on, and keeps them across restarts. Safe to repeat. Answered with
    /// [`Output::CronSynced`].
    SyncCronJobs {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's system name (its PHP container is `bp-<name>-php`).
        username: String,
        /// All of the account's jobs (at most [`MAX_CRON_JOBS`]).
        jobs: Vec<CronJobSpec>,
    },
    /// Starts one job now, apart from its schedule (the result is reported like any run).
    /// Answered with [`Output::CronStarted`].
    RunCronJob {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// The job (as it is now: it need not be synced).
        job: CronJobSpec,
    },
    /// The runs that started at or after `since` (Unix seconds; the node keeps the last
    /// hour). Answered with [`Output::CronRuns`].
    ReportCronRuns {
        /// The oldest start wanted.
        since: i64,
    },
    /// One file manager request on a tenant's files, done as the tenant's own user
    /// id. Answered with [`Output::File`].
    FileOp {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's numeric user id.
        uid: u32,
        /// What to do.
        request: files::FileRequest,
    },
    /// One database studio request on a tenant's own database on the node's shared
    /// server, done with a temporary account that has rights on that one database.
    /// Answered with [`Output::Studio`].
    StudioOp {
        /// The database's whole name.
        database: String,
        /// Which server (the node's shared one).
        #[serde(default, skip_serializing_if = "DbEngine::is_mysql")]
        engine: DbEngine,
        /// What to do.
        request: studio::StudioRequest,
    },
    /// Makes (or changes) an SFTP/FTPS account of a tenant on the node's file transfer
    /// server: a virtual user whose home is a folder of the tenant's files, working as
    /// the tenant's own user id. The password is set when one is given (and is needed
    /// for a new account). Safe to repeat. Answered with [`Output::FtpAccountSet`].
    UpsertFtpAccount {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's numeric user id.
        uid: u32,
        /// The account's name (lower-case letters, digits, `_`, `-`, `.`; at most 64).
        name: String,
        /// Its home, relative to the tenant's files (`""` is all of them).
        home: String,
        /// The password; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<DbPassword>,
        /// May log in over SFTP.
        sftp: bool,
        /// May log in over FTP with TLS (FTPS).
        ftps: bool,
        /// May also log in over FTP without TLS.
        ftp_plain: bool,
        /// May read but not change anything.
        read_only: bool,
        /// Most disk the account may use, in MB.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        quota_mb: Option<u64>,
        /// Whether it may log in at all.
        enabled: bool,
    },
    /// Removes an SFTP/FTPS account. Answered with [`Output::FtpAccountDropped`].
    DropFtpAccount {
        /// The account's name.
        name: String,
    },
    /// How far an import has come. Answered with [`Output::ImportProgress`].
    ReportImportProgress {
        /// The import.
        import_id: Uuid,
    },
    /// Let a tenant's containers reach the shared database server as `db` on their
    /// own network (the server is started first when it does not run). Safe to
    /// repeat. Answered with [`Output::TenantDbAllowed`].
    AllowTenantDatabases {
        /// The tenant's system name.
        username: String,
        /// Which server (the node's shared MariaDB when left out).
        #[serde(default, skip_serializing_if = "DbEngine::is_mysql")]
        engine: DbEngine,
    },
    /// Restart one of the tenant's own services: its web server (`web`) or all its
    /// PHP containers (`php`): stopped with a short grace period, started, and
    /// waited for. The files stay. Answered with [`Output::ServiceRestarted`].
    RestartService {
        /// The tenant's system name.
        username: String,
        /// `web` or `php`.
        service: String,
    },
    /// Put the tenant's php.ini options (a file of `option = value` lines made by
    /// the control plane) where its PHP containers read them and have the running
    /// ones load it. The file is checked against the list of options that may be
    /// set. Safe to repeat. Answered with [`Output::PhpIniSet`].
    SetPhpIni {
        /// The tenant's system name.
        username: String,
        /// The file (at most [`MAX_PHP_INI_BYTES`]; empty removes the options).
        ini: String,
    },
    /// Everything a new tenant needs on its node, in one operation: directories,
    /// network and volume ([`Operation::CreateTenant`]), the storage limits
    /// ([`Operation::SetStorageQuota`], skipped when the plan has none or the node
    /// cannot enforce them) and the containers
    /// ([`Operation::ProvisionTenantStack`]). Safe to repeat. Answered with
    /// [`Output::TenantReady`].
    SetUpTenant {
        /// The tenant's id.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// The tenant's numeric user id on this node.
        uid: u32,
        /// Most bytes of storage (`None` = unlimited).
        disk_bytes: Option<u64>,
        /// Most files (`None` = unlimited).
        inodes: Option<u64>,
        /// CPU in thousandths of a core (`None` = unlimited).
        cpu_millis: Option<u32>,
        /// Memory in bytes (`None` = unlimited).
        memory_bytes: Option<u64>,
        /// Processes (`None` = unlimited).
        pids: Option<u32>,
        /// Default files for the tenant's docroot, written when its directories
        /// are first made (see [`SkeletonFile`] for the limits).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        skeleton: Vec<SkeletonFile>,
    },
    /// Set the certificates one tenant uploaded: the web server uses them for those
    /// names instead of getting its own. Replaces what the tenant had; an empty list
    /// removes them. Kept by the agent (the keys in a file only it can read) and put
    /// back whenever the web server loses them. Answered with
    /// [`Output::CustomCertificatesSet`].
    SetCustomCertificates {
        /// The tenant's system name.
        username: String,
        /// The certificates, each with its key.
        items: Vec<CustomCertificate>,
    },
    /// Set the pages the node's web server shows when a site cannot answer.
    /// Replaces what it had; a kind that is not listed uses the installation's
    /// own page. Kept by the agent. Answered with [`Output::ErrorPagesSet`].
    SetErrorPages {
        /// At most one page of each kind.
        pages: Vec<ErrorPage>,
    },
    /// Replaces the node's firewall (see [`FirewallPolicy`]): checked, then put in place by the
    /// privileged helper as one transaction, so that a ruleset the packet filter refuses leaves
    /// the old one in force. SSH, loopback, answers and the `always_allow` addresses are never
    /// cut off. Answered with [`Output::FirewallSet`].
    SetFirewallRules {
        /// The policy.
        policy: FirewallPolicy,
    },
    /// Looks for malware in a tenant's files with ClamAV, in a container of its own with no
    /// network and the files read-only. With `quarantine`, each infected file is moved to the
    /// tenant's trash (the file manager can put it back). Answered with [`Output::Scanned`].
    ScanTenant {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's numeric user id.
        uid: u32,
        /// Only this folder of the tenant's files (relative to its `www`); all when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        folder: Option<String>,
        /// Move what is found to the trash.
        #[serde(default)]
        quarantine: bool,
    },
    /// Lists the WordPress installations in a tenant's files with the version of the core, of
    /// every plugin and of every theme (read only; nothing is run). Answered with
    /// [`Output::WordPressInventory`].
    InventoryWordPress {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's numeric user id.
        uid: u32,
    },
    /// Installs WordPress in a folder of a tenant's files with WP-CLI, in a container of its own
    /// that runs as the tenant's user on the tenant's network (it reaches the database as the
    /// site will). The database and its user must exist. The core is downloaded with its
    /// checksum verified, the configuration gets fresh salts made on the node, and the first
    /// administrator is made. Answered with [`Output::WordPressInstalled`].
    InstallWordPress {
        /// The application's id (the control plane finds the passwords by it).
        app_id: Uuid,
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// The tenant's numeric user id.
        uid: u32,
        /// The folder under the tenant's `www` (empty: `www` itself).
        path: String,
        /// The database.
        database: WpDatabase,
        /// The database user's password; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database_password: Option<DbPassword>,
        /// The site.
        site: WpSite,
        /// The first administrator's password; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        admin_password: Option<DbPassword>,
        /// A WordPress version (`6.8.2`); the latest when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        /// A locale (`bn_BD`); `en_US` when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locale: Option<String>,
        /// Plugins of wordpress.org to install and switch on (at most 10).
        #[serde(default)]
        plugins: Vec<String>,
    },
    /// Makes (or changes) a tenant's own cache server: a Redis or Memcached container on the
    /// tenant's network only, reachable by its container name (`bp-<account>-redis`), with a memory limit; Redis
    /// also has a password. Safe to repeat. Answered with [`Output::CacheInstanceSet`].
    SetCacheInstance {
        /// The instance's id (the control plane finds the password by it).
        instance_id: Uuid,
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// The tenant's numeric user id.
        uid: u32,
        /// `redis` or `memcached`.
        kind: String,
        /// Memory it may use, in MiB (1 to 4096).
        memory_mb: u32,
        /// Redis's password; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<DbPassword>,
    },
    /// Removes a tenant's cache server (and what it kept). Safe to repeat. Answered with
    /// [`Output::CacheInstanceRemoved`].
    RemoveCacheInstance {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// `redis` or `memcached`.
        kind: String,
    },
    /// Makes (or changes) one of a tenant's own applications: a Node.js or Python container on
    /// the tenant's network only, reachable by its container name (`bp-<account>-app-<name>`),
    /// that runs the account's files from `dir` as the account's user. With `install` the
    /// dependencies are installed first, in a one-off container. The command is a list of
    /// words; nothing is run through a shell. Safe to repeat. Answered with
    /// [`Output::RuntimeAppSet`].
    SetRuntimeApp {
        /// The application's id.
        app_id: Uuid,
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// The tenant's numeric user id.
        uid: u32,
        /// `node`, `python` or `container`.
        kind: String,
        /// The runtime's version (`24`, `3.13`); `0` for a container.
        version: String,
        /// The application's short name.
        name: String,
        /// The folder below the web directory.
        dir: String,
        /// The start command, one word per element.
        command: Vec<String>,
        /// Whether to install the dependencies first.
        install: bool,
        /// The port it listens on (it is also given as `PORT`).
        port: u16,
        /// Memory it may use, in MiB (32 to 4096).
        memory_mb: u32,
        /// Its environment; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env: Option<RuntimeEnv>,
        /// For `kind` `container`: the approved image to run, `repo@sha256:<digest>`. The
        /// image's own start-up is used (no command), without the account's files.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
    },
    /// Deploys a site or an application from a git repository: fetches `branch` of the
    /// repository over http(s) into a new release folder (`.bp-releases/<name>/<time>` below the
    /// web directory), runs the build steps in a sandbox (each a list of words, no shell), then
    /// switches `dest` to the new release with one rename and keeps the last few releases. When
    /// `restart_app` names one of the tenant's applications it is restarted. Everything runs in
    /// one-off containers as the tenant's user. Answered with [`Output::GitDeployed`].
    RunGitDeploy {
        /// The deploy's id.
        deploy_id: Uuid,
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// The tenant's numeric user id.
        uid: u32,
        /// A short name (used for the release folder).
        name: String,
        /// The repository, `https://…` without a login.
        repo_url: String,
        /// The branch.
        branch: String,
        /// The folder below the web directory that becomes the link to the release.
        dest: String,
        /// `node` or `python` when there are build steps.
        build_kind: Option<String>,
        /// The runtime's version for the build.
        build_version: Option<String>,
        /// The build steps, each one a list of words.
        build_steps: Vec<Vec<String>>,
        /// The name of an application to restart afterwards.
        restart_app: Option<String>,
        /// A token for a private repository; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<DbPassword>,
    },
    /// Removes what a deploy kept on the node (its releases stay: they are the account's files
    /// only until the link is removed by the account). Safe to repeat. Answered with
    /// [`Output::GitDeployRemoved`].
    RemoveGitDeploy {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// The deploy's short name.
        name: String,
    },
    /// Removes one of a tenant's own applications. Safe to repeat. Answered with
    /// [`Output::RuntimeAppRemoved`].
    RemoveRuntimeApp {
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// The application's short name.
        name: String,
    },
    /// Installs an application of the catalog (see [`AppDefinition`]) in a folder of a tenant's
    /// files: the archive is downloaded and its checksum verified, then the definition's steps
    /// run, in a one-off container as the tenant's user on the tenant's network. The steps are
    /// from a fixed vocabulary; no shell text of the definition ever runs. Answered with
    /// [`Output::AppInstalled`].
    InstallApp {
        /// The application's id (the control plane finds the passwords by it).
        app_id: Uuid,
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// The tenant's numeric user id.
        uid: u32,
        /// The folder under the tenant's `www` (empty: `www` itself).
        path: String,
        /// What to install.
        definition: Box<AppDefinition>,
        /// The database, when the definition needs one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database: Option<WpDatabase>,
        /// The database user's password; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        database_password: Option<DbPassword>,
        /// The values the steps' placeholders take.
        site: AppSite,
        /// The first administrator's password; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        admin_password: Option<DbPassword>,
    },
    /// Looks at or changes a WordPress site with WP-CLI, in the same kind of one-off container
    /// as [`Operation::InstallWordPress`]. Only the commands of [`WpCommand`] exist. Answered
    /// with [`Output::WordPressOverview`] (for `overview`) or [`Output::WpCliDone`].
    RunWpCli {
        /// The application's id (the control plane finds a password by it).
        app_id: Uuid,
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// The tenant's numeric user id.
        uid: u32,
        /// The folder under the tenant's `www` (empty: `www` itself).
        path: String,
        /// What to do.
        command: WpCommand,
        /// The new password of `reset_admin_password`; absent in the ledger, present on the wire.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<DbPassword>,
    },
    /// Looks for known vulnerabilities in an image with Trivy (it reads the image from its
    /// registry; no Docker socket is involved). Answered with [`Output::ImageScanned`].
    ScanImage {
        /// The image, pinned (`name@sha256:…`).
        reference: String,
    },
    /// Asks the registries what digest each tag has now. Answered with
    /// [`Output::ImageDigests`].
    CheckImageUpdates {
        /// `name:tag` of each image to ask about (at most 50).
        images: Vec<String>,
    },
    /// Brings the node's malware signatures up to date (freshclam, in a container of its own).
    /// Answered with [`Output::SignaturesUpdated`].
    UpdateMalwareSignatures,
    /// Replaces the node's list of User-Agent texts the web server refuses: a request whose
    /// User-Agent contains one of them (any case) is answered `403` before any site sees it.
    /// The agent keeps the list and puts it back whenever the web server loses it. Answered
    /// with [`Output::UserAgentBlocklistSet`].
    SetUserAgentBlocklist {
        /// Plain texts (1 to 100 characters, no control characters), at most
        /// [`MAX_USER_AGENT_PATTERNS`].
        patterns: Vec<String>,
    },
    /// Put one domain's web server configuration (rendered by the control plane)
    /// into the tenant's web container and have it reloaded. The file is checked
    /// by the web server itself before it is used; when it is refused the
    /// previous file is put back and the operation fails. Safe to repeat.
    /// Answered with [`Output::DomainAttached`].
    AttachDomain {
        /// The tenant's system name.
        username: String,
        /// The domain, lower-case ASCII; the file is named for it.
        domain: String,
        /// The configuration (at most [`MAX_VHOST_CONFIG_BYTES`]).
        config: String,
    },
    /// Remove one domain's web server configuration and have it reloaded. Safe
    /// to repeat. Answered with [`Output::DomainDetached`].
    DetachDomain {
        /// The tenant's system name.
        username: String,
        /// The domain.
        domain: String,
    },
    /// Remove the web server configuration of every domain of the tenant that is
    /// not in `domains`, and reload. What the control plane sends after it has
    /// attached the tenant's current domains, so a domain that was deleted does not
    /// stay served. Safe to repeat. Answered with [`Output::DomainsKept`].
    KeepDomains {
        /// The tenant's system name.
        username: String,
        /// The domains whose configuration stays (at most [`MAX_HOSTS_PER_TENANT`]).
        domains: Vec<String>,
    },
    /// List the certificates the node's web server holds (name, issuer, validity).
    /// Answered with [`Output::Certificates`].
    ReportCertificates,
    /// Set the web server's routes for one tenant: which names it answers for and
    /// what it does for each (proxy to the tenant's web container, show the
    /// "suspended" page, redirect paths). Replaces what the tenant had; an empty
    /// list removes the tenant's routes. The agent keeps them and puts them back
    /// whenever the web server loses them. Answered with [`Output::SiteRoutesSet`].
    SetSiteRoutes {
        /// The tenant's system name.
        username: String,
        /// The tenant's sites.
        routes: Vec<SiteRoute>,
    },
    /// Stop a tenant's containers; its files stay. Safe to repeat. Answered with
    /// [`Output::TenantSuspended`].
    SuspendTenant {
        /// The tenant's system name.
        username: String,
    },
    /// Start a suspended tenant's containers again and wait until they run.
    /// Safe to repeat. Answered with [`Output::TenantResumed`].
    ResumeTenant {
        /// The tenant's system name.
        username: String,
    },
    /// Remove a tenant from the node: containers, network, volume and, when
    /// `purge_data` is set, its files and directories (without it the files stay
    /// for a later purge). Safe to repeat. Answered with [`Output::TenantDeleted`].
    DeleteTenant {
        /// The tenant's id.
        tenant_id: Uuid,
        /// The tenant's system name.
        username: String,
        /// The tenant's numeric user id.
        uid: u32,
        /// Delete the site files too. Irreversible.
        purge_data: bool,
    },
    /// Change the limits of a tenant's running containers to a plan's, without
    /// restarting them. Answered with [`Output::ResourceLimitsSet`].
    SetResourceLimits {
        /// The tenant's system name.
        username: String,
        /// CPU in thousandths of a core (`None` = unlimited).
        cpu_millis: Option<u32>,
        /// Memory in bytes (`None` = unlimited).
        memory_bytes: Option<u64>,
        /// Processes (`None` = unlimited).
        pids: Option<u32>,
    },
}

/// Where backups are stored: the kind and what is not secret about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupTarget {
    /// `local`, `s3` or `sftp`.
    pub kind: String,
    /// The settings as the panel keeps them (a folder; endpoint, bucket, region, prefix;
    /// host, port, user, path, host key).
    pub settings: serde_json::Value,
}

/// One stored part of a backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupPartInfo {
    /// Its name at the destination.
    pub name: String,
    /// `files`, `database`, `dns` or `meta`.
    pub kind: String,
    /// Size as stored (encrypted), in bytes.
    pub stored_bytes: u64,
    /// SHA-256 of the part as stored (hex).
    pub sha256: String,
    /// For a database: its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
}

impl Operation {
    /// Wire names of every operation this build knows, in declaration order.
    pub const TYPE_NAMES: &'static [&'static str] = &[
        "ping",
        "report_capabilities",
        "create_tenant",
        "set_storage_quota",
        "provision_tenant_stack",
        "set_resource_limits",
        "set_php_versions",
        "set_php_ini",
        "restart_service",
        "ensure_db_server",
        "allow_tenant_databases",
        "create_database",
        "drop_database",
        "import_sql",
        "export_sql",
        "file_op",
        "studio_op",
        "create_backup",
        "restore_backup",
        "delete_backup",
        "sync_cron_jobs",
        "set_firewall_rules",
        "scan_tenant",
        "install_wordpress",
        "run_wp_cli",
        "install_app",
        "set_cache_instance",
        "remove_cache_instance",
        "set_runtime_app",
        "remove_runtime_app",
        "run_git_deploy",
        "remove_git_deploy",
        "scan_image",
        "check_image_updates",
        "inventory_wordpress",
        "update_malware_signatures",
        "set_user_agent_blocklist",
        "report_server_info",
        "set_timezone",
        "set_swap",
        "drop_caches",
        "reboot_node",
        "drop_swap",
        "read_ssh_access",
        "set_ssh_settings",
        "add_ssh_key",
        "remove_ssh_key",
        "write_sshd_config",
        "list_processes",
        "signal_process",
        "report_services",
        "report_web_service",
        "restart_node_service",
        "read_service_log",
        "list_containers",
        "restart_container",
        "collect_metrics",
        "read_log",
        "read_traffic",
        "read_waf_events",
        "run_cron_job",
        "report_cron_runs",
        "export_backup",
        "import_export",
        "upsert_ftp_account",
        "drop_ftp_account",
        "fetch_file",
        "send_file",
        "set_phpmyadmin_host",
        "open_phpmyadmin",
        "set_webmail_host",
        "open_webmail",
        "report_import_progress",
        "report_db_sizes",
        "migrate_tenant_databases",
        "remove_dedicated_db",
        "ensure_mail_server",
        "set_mail_domain",
        "remove_mail_domain",
        "upsert_mailbox",
        "drop_mailbox",
        "set_mail_forwarders",
        "set_mail_autoresponder",
        "report_mail",
        "mail_queue_action",
        "ensure_pg_server",
        "report_pg_sizes",
        "upsert_db_user",
        "grant_db",
        "revoke_db",
        "set_db_remote_hosts",
        "set_db_remote_access",
        "drop_db_user",
        "set_up_tenant",
        "set_site_routes",
        "set_error_pages",
        "report_certificates",
        "set_custom_certificates",
        "attach_domain",
        "detach_domain",
        "keep_domains",
        "suspend_tenant",
        "resume_tenant",
        "delete_tenant",
    ];

    /// For an operation that acts on a database server: which kind, and whose own server it is
    /// (`None`: the node's shared one).
    #[must_use]
    pub fn db_target(&self) -> Option<(DbEngine, Option<&str>)> {
        match self {
            Self::CreateDatabase {
                engine, dedicated, ..
            }
            | Self::DropDatabase {
                engine, dedicated, ..
            }
            | Self::UpsertDbUser {
                engine, dedicated, ..
            }
            | Self::GrantDb {
                engine, dedicated, ..
            }
            | Self::RevokeDb {
                engine, dedicated, ..
            }
            | Self::SetDbRemoteHosts {
                engine, dedicated, ..
            }
            | Self::DropDbUser {
                engine, dedicated, ..
            }
            | Self::ImportSql {
                engine, dedicated, ..
            }
            | Self::ExportSql {
                engine, dedicated, ..
            } => Some((*engine, dedicated.as_deref())),
            Self::SetDbRemoteAccess { engine, .. }
            | Self::AllowTenantDatabases { engine, .. }
            | Self::StudioOp { engine, .. } => Some((*engine, None)),
            _ => None,
        }
    }

    /// The operation's wire name.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Ping { .. } => "ping",
            Self::ReportCapabilities => "report_capabilities",
            Self::CreateTenant { .. } => "create_tenant",
            Self::SetStorageQuota { .. } => "set_storage_quota",
            Self::ProvisionTenantStack { .. } => "provision_tenant_stack",
            Self::SetResourceLimits { .. } => "set_resource_limits",
            Self::SetPhpVersions { .. } => "set_php_versions",
            Self::SetPhpIni { .. } => "set_php_ini",
            Self::RestartService { .. } => "restart_service",
            Self::EnsureDbServer => "ensure_db_server",
            Self::AllowTenantDatabases { .. } => "allow_tenant_databases",
            Self::CreateDatabase { .. } => "create_database",
            Self::DropDatabase { .. } => "drop_database",
            Self::ImportSql { .. } => "import_sql",
            Self::ExportSql { .. } => "export_sql",
            Self::FileOp { .. } => "file_op",
            Self::StudioOp { .. } => "studio_op",
            Self::CreateBackup { .. } => "create_backup",
            Self::RestoreBackup { .. } => "restore_backup",
            Self::DeleteBackup { .. } => "delete_backup",
            Self::SyncCronJobs { .. } => "sync_cron_jobs",
            Self::SetFirewallRules { .. } => "set_firewall_rules",
            Self::ScanTenant { .. } => "scan_tenant",
            Self::InstallWordPress { .. } => "install_wordpress",
            Self::RunWpCli { .. } => "run_wp_cli",
            Self::InstallApp { .. } => "install_app",
            Self::SetCacheInstance { .. } => "set_cache_instance",
            Self::RemoveCacheInstance { .. } => "remove_cache_instance",
            Self::SetRuntimeApp { .. } => "set_runtime_app",
            Self::RemoveRuntimeApp { .. } => "remove_runtime_app",
            Self::RunGitDeploy { .. } => "run_git_deploy",
            Self::RemoveGitDeploy { .. } => "remove_git_deploy",
            Self::ScanImage { .. } => "scan_image",
            Self::CheckImageUpdates { .. } => "check_image_updates",
            Self::InventoryWordPress { .. } => "inventory_wordpress",
            Self::UpdateMalwareSignatures => "update_malware_signatures",
            Self::SetUserAgentBlocklist { .. } => "set_user_agent_blocklist",
            Self::ReportServerInfo => "report_server_info",
            Self::SetTimezone { .. } => "set_timezone",
            Self::SetSwap { .. } => "set_swap",
            Self::DropCaches => "drop_caches",
            Self::RebootNode { .. } => "reboot_node",
            Self::DropSwap => "drop_swap",
            Self::ReadSshAccess => "read_ssh_access",
            Self::SetSshSettings { .. } => "set_ssh_settings",
            Self::AddSshKey { .. } => "add_ssh_key",
            Self::RemoveSshKey { .. } => "remove_ssh_key",
            Self::WriteSshdConfig { .. } => "write_sshd_config",
            Self::ListProcesses { .. } => "list_processes",
            Self::SignalProcess { .. } => "signal_process",
            Self::ReportServices => "report_services",
            Self::ReportWebService => "report_web_service",
            Self::RestartNodeService { .. } => "restart_node_service",
            Self::ReadServiceLog { .. } => "read_service_log",
            Self::ListContainers { .. } => "list_containers",
            Self::RestartContainer { .. } => "restart_container",
            Self::CollectMetrics => "collect_metrics",
            Self::ReadLog { .. } => "read_log",
            Self::ReadTraffic { .. } => "read_traffic",
            Self::ReadWafEvents { .. } => "read_waf_events",
            Self::RunCronJob { .. } => "run_cron_job",
            Self::ReportCronRuns { .. } => "report_cron_runs",
            Self::ExportBackup { .. } => "export_backup",
            Self::ImportExport { .. } => "import_export",
            Self::UpsertFtpAccount { .. } => "upsert_ftp_account",
            Self::DropFtpAccount { .. } => "drop_ftp_account",
            Self::FetchFile { .. } => "fetch_file",
            Self::SendFile { .. } => "send_file",
            Self::SetPhpMyAdminHost { .. } => "set_phpmyadmin_host",
            Self::OpenPhpMyAdmin { .. } => "open_phpmyadmin",
            Self::SetWebmailHost { .. } => "set_webmail_host",
            Self::OpenWebmail { .. } => "open_webmail",
            Self::ReportImportProgress { .. } => "report_import_progress",
            Self::ReportDbSizes => "report_db_sizes",
            Self::MigrateTenantDatabases { .. } => "migrate_tenant_databases",
            Self::RemoveDedicatedDb { .. } => "remove_dedicated_db",
            Self::EnsureMailServer { .. } => "ensure_mail_server",
            Self::SetMailDomain { .. } => "set_mail_domain",
            Self::RemoveMailDomain { .. } => "remove_mail_domain",
            Self::UpsertMailbox { .. } => "upsert_mailbox",
            Self::DropMailbox { .. } => "drop_mailbox",
            Self::SetMailForwarders { .. } => "set_mail_forwarders",
            Self::SetMailAutoresponder { .. } => "set_mail_autoresponder",
            Self::ReportMail => "report_mail",
            Self::MailQueueAction { .. } => "mail_queue_action",
            Self::EnsurePgServer => "ensure_pg_server",
            Self::ReportPgSizes => "report_pg_sizes",
            Self::UpsertDbUser { .. } => "upsert_db_user",
            Self::GrantDb { .. } => "grant_db",
            Self::RevokeDb { .. } => "revoke_db",
            Self::SetDbRemoteHosts { .. } => "set_db_remote_hosts",
            Self::SetDbRemoteAccess { .. } => "set_db_remote_access",
            Self::DropDbUser { .. } => "drop_db_user",
            Self::SetUpTenant { .. } => "set_up_tenant",
            Self::SetSiteRoutes { .. } => "set_site_routes",
            Self::SetErrorPages { .. } => "set_error_pages",
            Self::ReportCertificates => "report_certificates",
            Self::SetCustomCertificates { .. } => "set_custom_certificates",
            Self::AttachDomain { .. } => "attach_domain",
            Self::DetachDomain { .. } => "detach_domain",
            Self::KeepDomains { .. } => "keep_domains",
            Self::SuspendTenant { .. } => "suspend_tenant",
            Self::ResumeTenant { .. } => "resume_tenant",
            Self::DeleteTenant { .. } => "delete_tenant",
        }
    }
}

/// One operation with the metadata needed to run it exactly once, in time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationEnvelope {
    /// The operations-ledger id; results refer to it.
    pub op_id: Uuid,
    /// Same key, same effect: lets the control plane retry safely.
    pub idempotency_key: String,
    /// The tenant the operation is about, when it has one.
    #[serde(default)]
    pub tenant_id: Option<Uuid>,
    /// When the control plane created it (Unix seconds).
    pub issued_at: i64,
    /// After this (Unix seconds) the agent must refuse to run it.
    pub expires_at: i64,
    /// Protocol version of this envelope.
    pub protocol_version: u16,
    /// What to do.
    pub op: Operation,
}

impl OperationEnvelope {
    /// Refuses envelopes that are expired, issued implausibly far in the
    /// future, valid for too long, or of an unsupported version. `now` is Unix
    /// seconds on the agent's clock.
    ///
    /// # Errors
    /// [`ErrorCode::Expired`] or [`ErrorCode::UnsupportedVersion`].
    pub fn check_fresh(&self, now: i64) -> Result<(), AgentError> {
        check_header(self.protocol_version, self.issued_at, self.expires_at, now)
    }
}

fn check_header(version: u16, issued_at: i64, expires_at: i64, now: i64) -> Result<(), AgentError> {
    if !(MIN_SUPPORTED_VERSION..=PROTOCOL_VERSION).contains(&version) {
        return Err(AgentError::new(
            ErrorCode::UnsupportedVersion,
            format!("protocol version {version} is not supported"),
        ));
    }
    if now >= expires_at {
        return Err(AgentError::new(
            ErrorCode::Expired,
            "the operation has expired",
        ));
    }
    if issued_at > now.saturating_add(MAX_CLOCK_SKEW_SECS) {
        return Err(AgentError::new(
            ErrorCode::Expired,
            "the operation was issued in the future; check the clocks",
        ));
    }
    if expires_at.saturating_sub(issued_at) > MAX_ENVELOPE_LIFETIME_SECS {
        return Err(AgentError::new(
            ErrorCode::Expired,
            "the operation's validity window is too long",
        ));
    }
    Ok(())
}

// ----------------------------------------------------------------- results

/// What the node reports about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Agent build version.
    pub agent_version: String,
    /// Logical CPU cores.
    pub cpu_cores: u32,
    /// Total memory in bytes.
    pub memory_bytes: u64,
    /// Size of the filesystem holding tenant data, in bytes.
    pub disk_total_bytes: u64,
    /// Free space there, in bytes.
    pub disk_free_bytes: u64,
    /// Kernel release, e.g. `6.8.0-31-generic`.
    pub kernel: String,
    /// Docker Engine version, if reachable.
    #[serde(default)]
    pub docker_version: Option<String>,
    /// How the agent reaches Docker.
    #[serde(default)]
    pub docker_mode: Option<DockerMode>,
    /// cgroup v2 is in use.
    #[serde(default)]
    pub cgroup_v2: bool,
    /// seccomp is available.
    #[serde(default)]
    pub seccomp: bool,
    /// AppArmor is enabled.
    #[serde(default)]
    pub apparmor: bool,
    /// Unprivileged user namespaces work.
    #[serde(default)]
    pub user_namespaces: bool,
    /// How tenant storage limits are enforced on this node (`xfs`), or absent
    /// when the filesystem under the tenants directory cannot enforce them.
    #[serde(default)]
    pub storage_quota: Option<String>,
}

/// How the agent reaches Docker (ADR 0004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerMode {
    /// A rootless daemon owned by the agent user. The production mode.
    Rootless,
    /// A root daemon reached through the `docker` group: root-equivalent;
    /// development only.
    Rootful,
}

/// The data an operation produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Output {
    /// Answer to [`Operation::Ping`].
    Pong {
        /// The nonce from the ping.
        nonce: u64,
    },
    /// Answer to [`Operation::ReportCapabilities`].
    Capabilities(Capabilities),
    /// Answer to [`Operation::SetStorageQuota`].
    StorageQuotaSet {
        /// How the node enforces it (`xfs`).
        backend: String,
    },
    /// Answer to [`Operation::SetUpTenant`].
    TenantReady {
        /// Names of the running containers, in start order.
        containers: Vec<String>,
        /// How storage limits are enforced (`xfs`), or `None` when they are not.
        storage_quota: Option<String>,
    },
    /// Answer to [`Operation::ScanTenant`].
    Scanned {
        /// How many files were looked at.
        scanned: u64,
        /// What was found (at most 500; `more` says when there was more).
        infected: Vec<Infection>,
        /// More was found than listed.
        more: bool,
    },
    /// Answer to [`Operation::InventoryWordPress`].
    WordPressInventory {
        /// The installations found (at most 20).
        installs: Vec<WpInstall>,
    },
    /// Answer to [`Operation::SetCacheInstance`].
    CacheInstanceSet {
        /// The name the tenant's sites reach it by.
        host: String,
        /// Its port.
        port: u16,
    },
    /// Answer to [`Operation::RemoveCacheInstance`].
    CacheInstanceRemoved {
        /// Whether there was one.
        existed: bool,
    },
    /// Answer to [`Operation::SetRuntimeApp`].
    RuntimeAppSet {
        /// `name:port` the web server reaches it by.
        host: String,
    },
    /// Answer to [`Operation::RunGitDeploy`].
    GitDeployed {
        /// The commit that is now live.
        commit: String,
        /// The end of what the fetch and the build printed.
        log: String,
    },
    /// Answer to [`Operation::RemoveGitDeploy`].
    GitDeployRemoved,
    /// Answer to [`Operation::RemoveRuntimeApp`].
    RuntimeAppRemoved {
        /// Whether there was one.
        existed: bool,
    },
    /// Answer to [`Operation::InstallApp`].
    AppInstalled {
        /// What the definition says it installed.
        version: String,
    },
    /// Answer to [`WpCommand::Overview`].
    WordPressOverview(WpOverview),
    /// Answer to [`WpCommand::Scan`].
    WordPressScan(WpScan),
    /// Answer to the other [`WpCommand`]s: what WP-CLI said (clipped, secrets scrubbed).
    WpCliDone {
        /// The last of what it printed.
        output: String,
    },
    /// Answer to [`Operation::InstallWordPress`].
    WordPressInstalled {
        /// The version now installed.
        version: String,
    },
    /// Answer to [`Operation::ScanImage`].
    ImageScanned {
        /// Findings by severity.
        critical: u32,
        /// Findings by severity.
        high: u32,
        /// Findings by severity.
        medium: u32,
        /// Findings by severity.
        low: u32,
        /// The worst findings (at most 50, worst first, fixable first).
        top: Vec<ImageVulnerability>,
    },
    /// Answer to [`Operation::CheckImageUpdates`].
    ImageDigests {
        /// What the registry said for each (an image it could not be asked about is left out).
        items: Vec<ImageDigest>,
    },
    /// Answer to [`Operation::UpdateMalwareSignatures`].
    SignaturesUpdated {
        /// What freshclam said last (clipped), such as the daily database's version.
        detail: String,
    },
    /// Answer to [`Operation::SetFirewallRules`].
    FirewallSet {
        /// How many rules the table has now.
        rules: usize,
    },
    /// Answer to [`Operation::SetUserAgentBlocklist`].
    UserAgentBlocklistSet {
        /// How many texts the node has now.
        count: usize,
        /// Whether the web server has them (they are kept either way).
        applied: bool,
    },
    /// Answer to [`Operation::SetErrorPages`].
    ErrorPagesSet {
        /// Pages that are not the installation's own.
        custom: usize,
        /// Whether the web server has them (they are kept either way).
        applied: bool,
    },
    /// Answer to [`Operation::EnsureDbServer`].
    DbServerReady {
        /// The loopback port the agent reaches it on.
        port: u16,
        /// Whether this call made the container.
        created: bool,
    },
    /// Answer to [`Operation::CreateDatabase`].
    DatabaseCreated {
        /// Whether this call made it (false: it was there).
        created: bool,
    },
    /// Answer to [`Operation::DropDatabase`].
    DatabaseDropped {
        /// Whether there was one to remove.
        existed: bool,
    },
    /// Answer to [`Operation::EnsureMailServer`].
    MailServerReady {
        /// Whether this call made the container.
        created: bool,
    },
    /// Answer to [`Operation::SetMailDomain`].
    MailDomainSet {
        /// The DNS records the domain needs for mail (MX, SPF, DKIM, DMARC).
        records: Vec<MailDnsRecord>,
    },
    /// Answer to [`Operation::RemoveMailDomain`].
    MailDomainRemoved {
        /// Whether the server knew the domain.
        existed: bool,
    },
    /// Answer to [`Operation::UpsertMailbox`].
    MailboxSet {
        /// Whether this call made the mailbox.
        created: bool,
    },
    /// Answer to [`Operation::DropMailbox`].
    MailboxDropped {
        /// Whether there was one.
        existed: bool,
    },
    /// Answer to [`Operation::SetMailForwarders`].
    MailForwardersSet {
        /// How many forwarders the domain has now.
        count: usize,
    },
    /// Answer to [`Operation::SetMailAutoresponder`].
    MailAutoresponderSet,
    /// Answer to [`Operation::ReportMail`].
    MailReport {
        /// Every mailbox's size.
        usage: Vec<MailUsage>,
        /// The outgoing queue.
        queue: Vec<MailQueueItem>,
        /// How many messages the queue holds (more than `queue` when it was cut).
        queue_total: u64,
    },
    /// Answer to [`Operation::MailQueueAction`].
    MailQueueChanged {
        /// How many messages it touched.
        count: usize,
    },
    /// Answer to [`Operation::MigrateTenantDatabases`].
    DbMigrated {
        /// How many databases were copied.
        databases: usize,
        /// How many users were made on the target.
        users: usize,
    },
    /// Answer to [`Operation::RemoveDedicatedDb`].
    DedicatedDbRemoved {
        /// Whether there was a server to remove.
        existed: bool,
    },
    /// Answer to [`Operation::ImportSql`].
    SqlImported {
        /// How many statements ran.
        statements: u64,
    },
    /// Answer to [`Operation::SetPhpMyAdminHost`].
    #[serde(rename = "phpmyadmin_host_set")]
    PhpMyAdminHostSet {
        /// Whether the web server has the route now (otherwise it is kept and put
        /// in place when the web server can be reached).
        applied: bool,
    },
    /// Answer to [`Operation::OpenPhpMyAdmin`].
    #[serde(rename = "phpmyadmin_opened")]
    PhpMyAdminOpened {
        /// Until when the sign-in link works (Unix seconds).
        link_expires_at: i64,
    },
    /// Answer to [`Operation::SetWebmailHost`].
    #[serde(rename = "webmail_host_set")]
    WebmailHostSet {
        /// Whether the web server has the route now.
        applied: bool,
    },
    /// Answer to [`Operation::OpenWebmail`].
    #[serde(rename = "webmail_opened")]
    WebmailOpened {
        /// Until when the sign-in link works (Unix seconds).
        link_expires_at: i64,
    },
    /// Answer to [`Operation::FetchFile`].
    FileFetched {
        /// Size of the file that was put in place.
        size_bytes: u64,
    },
    /// Answer to [`Operation::SendFile`].
    FileSent {
        /// Size of the file that was sent.
        size_bytes: u64,
    },
    /// Answer to [`Operation::UpsertFtpAccount`].
    FtpAccountSet {
        /// Whether the account was made now (otherwise it was changed).
        created: bool,
    },
    /// Answer to [`Operation::DropFtpAccount`].
    FtpAccountDropped {
        /// Whether there was one to remove.
        existed: bool,
    },
    /// Answer to [`Operation::CreateBackup`].
    BackupCreated {
        /// SHA-256 of the stored manifest (hex).
        manifest_sha256: String,
        /// Everything stored, in bytes.
        size_bytes: u64,
        /// The parts, as stored.
        parts: Vec<BackupPartInfo>,
    },
    /// Answer to [`Operation::RestoreBackup`].
    BackupRestored {
        /// How many parts were read and found whole.
        verified_parts: u32,
        /// Whether the files were put back.
        files_restored: bool,
        /// The databases that were put back.
        databases: Vec<String>,
        /// The parts of the snapshot taken before (its folder is `pre-<restore_id>`).
        snapshot: Vec<BackupPartInfo>,
        /// The DNS records of the backup (`dns.json`), when they were asked for.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dns: Option<serde_json::Value>,
    },
    /// Answer to [`Operation::ExportBackup`].
    BackupExported {
        /// The file's name at the destination.
        object: String,
        /// Its size in bytes.
        size_bytes: u64,
    },
    /// Answer to [`Operation::ImportExport`].
    ExportImported {
        /// The parts that were put in place.
        parts: Vec<BackupPartInfo>,
        /// Everything put in place, in bytes.
        size_bytes: u64,
    },
    /// Answer to [`Operation::ReadTraffic`].
    Traffic {
        /// Requests counted.
        requests: u64,
        /// Bytes sent (the body, as the web server logs it).
        bytes: u64,
        /// Answers 200-299.
        status_2xx: u64,
        /// Answers 300-399.
        status_3xx: u64,
        /// Answers 400-499.
        status_4xx: u64,
        /// Answers 500-599.
        status_5xx: u64,
        /// Requests whose User-Agent names a robot (bot, crawler, spider ...).
        bots: u64,
        /// Per hour, oldest first (hours without a request are left out).
        hours: Vec<TrafficHour>,
        /// The most asked pages, most first.
        top_paths: Vec<TrafficPath>,
        /// The log held less than the time asked for (it is a lower bound).
        partial: bool,
    },
    /// Answer to [`Operation::ReadWafEvents`].
    WafEvents {
        /// The web server of this node has the firewall and its log is there to read.
        available: bool,
        /// The newest first.
        events: Vec<WafEvent>,
        /// More was there than were asked for, or the log was cut at its start.
        partial: bool,
    },
    /// Answer to [`Operation::ReadLog`].
    LogLines {
        /// Oldest first.
        lines: Vec<LogLine>,
        /// More was there than one read returns.
        truncated: bool,
    },
    /// Answer to [`Operation::ReportServices`] and [`Operation::ReportWebService`].
    Services {
        /// One entry per service.
        items: Vec<ServiceStatus>,
    },
    /// Answer to [`Operation::ReportServerInfo`].
    ServerInfo {
        /// What the node says.
        info: ServerInfo,
    },
    /// Answer to the server tools ([`Operation::SetTimezone`], [`Operation::SetSwap`],
    /// [`Operation::DropCaches`], [`Operation::DropSwap`], [`Operation::RebootNode`]).
    ServerActionDone {
        /// Which one.
        action: String,
    },
    /// Answer to [`Operation::ReadSshAccess`].
    SshAccess {
        /// What the node says.
        access: SshAccess,
    },
    /// Answer to [`Operation::ListProcesses`].
    Processes {
        /// The processes (at most 2000).
        items: Vec<ProcessItem>,
    },
    /// Answer to [`Operation::SignalProcess`].
    ProcessSignalled {
        /// The process.
        pid: u32,
        /// How it ended.
        outcome: SignalOutcome,
    },
    /// Answer to [`Operation::ListContainers`].
    Containers {
        /// By name.
        items: Vec<ContainerItem>,
    },
    /// Answer to [`Operation::RestartContainer`].
    ContainerRestarted {
        /// The container.
        name: String,
        /// The state it is in now.
        state: String,
    },
    /// Answer to [`Operation::CollectMetrics`].
    Metrics {
        /// When it was read (Unix seconds).
        at: i64,
        /// The node.
        node: NodeMetrics,
        /// The running containers (at most [`MAX_METRIC_CONTAINERS`]).
        containers: Vec<ContainerMetrics>,
    },
    /// Answer to [`Operation::SyncCronJobs`].
    CronSynced {
        /// How many jobs the node has for the account now.
        count: u32,
    },
    /// Answer to [`Operation::RunCronJob`].
    CronStarted {
        /// Whether it was started (not when its last run is still going).
        started: bool,
    },
    /// Answer to [`Operation::ReportCronRuns`].
    CronRuns {
        /// The runs, oldest first (at most [`MAX_CRON_RUNS_PER_REPORT`]).
        runs: Vec<CronRun>,
    },
    /// Answer to [`Operation::DeleteBackup`].
    BackupDeleted {
        /// How many stored objects were removed.
        removed: u32,
    },
    /// Answer to [`Operation::FileOp`].
    File {
        /// What came of it.
        result: files::FileResult,
    },
    /// Answer to [`Operation::StudioOp`].
    Studio {
        /// What came of it.
        result: studio::StudioResult,
    },
    /// Answer to [`Operation::ExportSql`].
    SqlExported {
        /// Size of the compressed dump that was sent.
        size_bytes: u64,
    },
    /// Answer to [`Operation::ReportImportProgress`].
    ImportProgress {
        /// `unknown` (this node has no such import), `downloading`, `checking`,
        /// `importing`, `done` or `failed`.
        stage: String,
        /// Bytes of the uploaded file taken in so far (downloaded, then read).
        bytes_done: u64,
        /// The file's size.
        bytes_total: u64,
        /// Statements run so far.
        statements: u64,
        /// Why it failed (fixed words and the server's own message, clipped).
        error: Option<String>,
    },
    /// Answer to [`Operation::ReportDbSizes`].
    DbSizes {
        /// One entry per database (at most [`MAX_DB_SIZES_PER_REPORT`]).
        items: Vec<DbSize>,
    },
    /// Answer to [`Operation::UpsertDbUser`].
    DbUserSet {
        /// Whether this call made the user.
        created: bool,
        /// Where it may connect from (the tenant's network, as the server writes it).
        host: String,
    },
    /// Answer to [`Operation::GrantDb`].
    DbGranted,
    /// Answer to [`Operation::RevokeDb`].
    DbRevoked,
    /// Answer to [`Operation::SetDbRemoteHosts`].
    DbRemoteHostsSet {
        /// How many remote addresses the user has now.
        hosts: usize,
    },
    /// Answer to [`Operation::SetDbRemoteAccess`].
    DbRemoteAccessSet {
        /// How many address ranges may reach the server now.
        ranges: usize,
        /// Whether the server's public port is open (to those ranges only).
        published: bool,
    },
    /// Answer to [`Operation::DropDbUser`].
    DbUserDropped {
        /// Whether there was one to remove.
        existed: bool,
    },
    /// Answer to [`Operation::AllowTenantDatabases`].
    TenantDbAllowed {
        /// The tenant's network the server was put on.
        network: String,
    },
    /// Answer to [`Operation::RestartService`].
    ServiceRestarted {
        /// The containers that were restarted.
        containers: Vec<String>,
    },
    /// Answer to [`Operation::SetPhpIni`].
    PhpIniSet {
        /// How many PHP containers loaded it (the file is kept either way; a
        /// stopped container reads it when it starts).
        reloaded: usize,
    },
    /// Answer to [`Operation::SetPhpVersions`].
    PhpVersionsSet {
        /// The PHP containers that run now, the main one first.
        containers: Vec<String>,
        /// The containers that were removed because no domain uses their version.
        removed: Vec<String>,
    },
    /// Answer to [`Operation::AttachDomain`].
    DomainAttached {
        /// Whether the running web server has it (it is kept either way; a web
        /// container that is stopped loads it when it starts).
        applied: bool,
    },
    /// Answer to [`Operation::DetachDomain`].
    DomainDetached {
        /// Whether the running web server dropped it.
        applied: bool,
    },
    /// Answer to [`Operation::KeepDomains`].
    DomainsKept {
        /// How many configurations were removed.
        removed: usize,
        /// Whether the running web server dropped them.
        applied: bool,
    },
    /// Answer to [`Operation::SetCustomCertificates`].
    CustomCertificatesSet {
        /// How many certificates the tenant has now.
        count: usize,
        /// Whether the web server has them (they are kept either way).
        applied: bool,
    },
    /// Answer to [`Operation::ReportCertificates`].
    Certificates {
        /// The certificates, by name (at most [`MAX_CERTIFICATES_PER_REPORT`]).
        items: Vec<CertificateInfo>,
    },
    /// Answer to [`Operation::SetSiteRoutes`].
    SiteRoutesSet {
        /// Sites the tenant has now.
        sites: usize,
        /// Names they answer for.
        hosts: usize,
        /// Whether the web server has them (the routes are kept either way and
        /// applied as soon as it can take them).
        applied: bool,
    },
    /// Answer to [`Operation::SuspendTenant`].
    TenantSuspended {
        /// Names of the containers that are stopped, in the order they were stopped.
        containers: Vec<String>,
    },
    /// Answer to [`Operation::ResumeTenant`].
    TenantResumed {
        /// Names of the running containers, in start order.
        containers: Vec<String>,
    },
    /// Answer to [`Operation::DeleteTenant`].
    TenantDeleted {
        /// Whether the files were deleted too.
        purged: bool,
    },
    /// Answer to [`Operation::SetResourceLimits`].
    ResourceLimitsSet {
        /// Names of the containers that were updated.
        containers: Vec<String>,
    },
    /// Answer to [`Operation::ProvisionTenantStack`].
    StackProvisioned {
        /// Names of the running containers, in start order.
        containers: Vec<String>,
    },
    /// Answer to [`Operation::CreateTenant`].
    TenantCreated {
        /// The tenant's directory on the node.
        root: String,
        /// Name of its private network.
        network: String,
        /// Name of its files volume.
        volume: String,
    },
}

/// How an operation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    /// Done.
    Succeeded,
    /// Failed for good.
    Failed,
    /// Failed, but trying again may work.
    Retryable,
}

/// The agent's answer to one operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationResult {
    /// The operation this answers.
    pub op_id: Uuid,
    /// How it ended.
    pub status: ResultStatus,
    /// What it produced, on success.
    #[serde(default)]
    pub output: Option<Output>,
    /// Why it failed, otherwise.
    #[serde(default)]
    pub error: Option<AgentError>,
}

impl OperationResult {
    /// A successful result.
    #[must_use]
    pub fn success(op_id: Uuid, output: Output) -> Self {
        Self {
            op_id,
            status: ResultStatus::Succeeded,
            output: Some(output),
            error: None,
        }
    }

    /// A failed result; the status follows [`AgentError::retryable`].
    #[must_use]
    pub fn failure(op_id: Uuid, error: AgentError) -> Self {
        Self {
            op_id,
            status: if error.retryable {
                ResultStatus::Retryable
            } else {
                ResultStatus::Failed
            },
            output: None,
            error: Some(error),
        }
    }
}

// ---------------------------------------------------------------- messages

/// First message from the agent after the TLS handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Highest protocol version the agent speaks.
    pub protocol_version: u16,
    /// Agent build version.
    pub agent_version: String,
}

/// The control plane's answer to [`Hello`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    /// The version both sides will use (see [`negotiate_version`]).
    pub protocol_version: u16,
    /// The control plane's clock (Unix seconds), so the agent can warn about drift.
    pub server_time: i64,
}

/// Frames sent by the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    /// Opens the session.
    Hello(Hello),
    /// Answers an operation.
    Result(OperationResult),
}

/// Frames sent by the control plane. Operation frames are produced with
/// [`encode_operation`] and read with [`parse_operation_frame`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
// A frame is built, sent and dropped one at a time; boxing the operation would only
// add a pointer to follow.
#[allow(clippy::large_enum_variant)]
pub enum ControlMessage {
    /// Accepts the session.
    Welcome(Welcome),
    /// Asks the agent to run an operation.
    Operation(OperationEnvelope),
}

// -------------------------------------------------------------- enrollment

/// Path of the enrollment endpoint on the control plane.
pub const ENROLL_PATH: &str = "/api/v1/public/nodes/enroll";

/// What an agent sends to enrol (plain HTTPS, before it has a certificate).
#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnrollRequest {
    /// The one-time token from the "add node" screen.
    #[cfg_attr(feature = "openapi", schema(example = "dhn_…"))]
    pub token: String,
    /// PEM certificate signing request made with a fresh ECDSA P-256 key that
    /// never leaves the node.
    pub csr_pem: String,
    /// Host name of the server.
    #[serde(default)]
    pub hostname: Option<String>,
    /// Agent build version.
    #[serde(default)]
    pub agent_version: Option<String>,
}

impl std::fmt::Debug for EnrollRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the token or the request body.
        f.debug_struct("EnrollRequest")
            .field("hostname", &self.hostname)
            .field("agent_version", &self.agent_version)
            .finish_non_exhaustive()
    }
}

/// The node's new identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EnrollResponse {
    /// The node's id.
    pub node_id: Uuid,
    /// The node's name.
    pub name: String,
    /// The node certificate (PEM): client authentication for the mTLS connection.
    pub certificate_pem: String,
    /// The CA certificate (PEM) that signed it and that signs the control
    /// plane's server certificate.
    pub ca_certificate_pem: String,
    /// SHA-256 fingerprint of the CA certificate (lowercase hex), to compare
    /// with the one in the enrol command.
    pub ca_fingerprint_sha256: String,
    /// Certificate validity start (RFC 3339, UTC).
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub not_before: String,
    /// Certificate expiry (RFC 3339, UTC). The agent renews before this.
    #[cfg_attr(feature = "openapi", schema(format = DateTime))]
    pub not_after: String,
    /// Protocol version the control plane speaks.
    pub protocol_version: u16,
    /// Where the agent opens its long-lived mutual-TLS connection
    /// (`https://host:port`), when the control plane has the agent port switched on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_url: Option<String>,
}

// ----------------------------------------------------------------- framing

/// Why a frame could not be produced or read.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The frame is larger than [`MAX_FRAME_BYTES`].
    #[error("frame of {0} bytes exceeds the {MAX_FRAME_BYTES} byte limit")]
    TooLarge(usize),
    /// Not valid JSON, or not a message of the expected shape.
    #[error("malformed frame: {0}")]
    Malformed(String),
}

/// Serialises a message as one frame.
///
/// # Errors
/// [`FrameError::TooLarge`] or [`FrameError::Malformed`].
pub fn encode<T: Serialize>(message: &T) -> Result<String, FrameError> {
    let text = serde_json::to_string(message).map_err(|e| FrameError::Malformed(e.to_string()))?;
    if text.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(text.len()));
    }
    Ok(text)
}

/// Reads one frame. The size is checked before any parsing.
///
/// # Errors
/// [`FrameError::TooLarge`] or [`FrameError::Malformed`].
pub fn decode<T: for<'de> Deserialize<'de>>(frame: &str) -> Result<T, FrameError> {
    if frame.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(frame.len()));
    }
    serde_json::from_str(frame).map_err(|e| FrameError::Malformed(e.to_string()))
}

/// Frame for [`ControlMessage::Operation`].
///
/// # Errors
/// Same as [`encode`].
pub fn encode_operation(envelope: &OperationEnvelope) -> Result<String, FrameError> {
    #[derive(Serialize)]
    #[serde(tag = "type", rename = "operation")]
    struct Frame<'a> {
        #[serde(flatten)]
        envelope: &'a OperationEnvelope,
    }
    encode(&Frame { envelope })
}

/// The part of an operation frame every version understands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvelopeHeader {
    /// The operations-ledger id.
    pub op_id: Uuid,
    /// Idempotency key.
    pub idempotency_key: String,
    /// Tenant the operation is about.
    pub tenant_id: Option<Uuid>,
    /// Creation time (Unix seconds).
    pub issued_at: i64,
    /// Expiry (Unix seconds).
    pub expires_at: i64,
    /// Protocol version.
    pub protocol_version: u16,
}

impl EnvelopeHeader {
    /// Same checks as [`OperationEnvelope::check_fresh`].
    ///
    /// # Errors
    /// [`ErrorCode::Expired`] or [`ErrorCode::UnsupportedVersion`].
    pub fn check_fresh(&self, now: i64) -> Result<(), AgentError> {
        check_header(self.protocol_version, self.issued_at, self.expires_at, now)
    }
}

/// The result of reading an operation frame on the agent.
// A parsed frame is handled once and dropped; boxing the envelope would only add churn.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// A known, well-formed operation.
    Known(OperationEnvelope),
    /// A well-formed envelope carrying an operation this agent does not know.
    /// Answer it with [`ErrorCode::UnsupportedOperation`].
    Unsupported {
        /// The envelope header, to answer the right `op_id`.
        header: EnvelopeHeader,
        /// The unknown operation's wire name.
        op_type: String,
    },
    /// A known operation with invalid arguments. Answer it with
    /// [`ErrorCode::InvalidArguments`].
    InvalidArguments {
        /// The envelope header, to answer the right `op_id`.
        header: EnvelopeHeader,
        /// What is wrong.
        message: String,
    },
}

/// Reads an operation frame without letting an unknown operation kill the
/// session: the header is parsed first, the operation second.
///
/// # Errors
/// [`FrameError`] when the frame is not an operation envelope at all (no
/// usable header), in which case there is no `op_id` to answer.
pub fn parse_operation_frame(frame: &str) -> Result<Parsed, FrameError> {
    if frame.len() > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(frame.len()));
    }
    #[derive(Deserialize)]
    struct Raw {
        #[serde(rename = "type")]
        kind: String,
        op_id: Uuid,
        idempotency_key: String,
        #[serde(default)]
        tenant_id: Option<Uuid>,
        issued_at: i64,
        expires_at: i64,
        protocol_version: u16,
        op: serde_json::Value,
    }
    let raw: Raw = serde_json::from_str(frame).map_err(|e| FrameError::Malformed(e.to_string()))?;
    if raw.kind != "operation" {
        return Err(FrameError::Malformed(format!(
            "expected an operation frame, got `{}`",
            raw.kind
        )));
    }
    let header = EnvelopeHeader {
        op_id: raw.op_id,
        idempotency_key: raw.idempotency_key,
        tenant_id: raw.tenant_id,
        issued_at: raw.issued_at,
        expires_at: raw.expires_at,
        protocol_version: raw.protocol_version,
    };
    let op_type = raw
        .op
        .get("type")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| FrameError::Malformed("operation has no `type`".to_owned()))?;
    match serde_json::from_value::<Operation>(raw.op) {
        Ok(op) => Ok(Parsed::Known(OperationEnvelope {
            op_id: header.op_id,
            idempotency_key: header.idempotency_key,
            tenant_id: header.tenant_id,
            issued_at: header.issued_at,
            expires_at: header.expires_at,
            protocol_version: header.protocol_version,
            op,
        })),
        Err(_) if !Operation::TYPE_NAMES.contains(&op_type.as_str()) => {
            Ok(Parsed::Unsupported { header, op_type })
        }
        Err(e) => Ok(Parsed::InvalidArguments {
            header,
            message: e.to_string(),
        }),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    const ID: &str = "0190b6a2-0000-7000-8000-000000000001";

    fn id() -> Uuid {
        ID.parse().unwrap()
    }

    fn envelope(op: Operation) -> OperationEnvelope {
        OperationEnvelope {
            op_id: id(),
            idempotency_key: "tenant-1:create".into(),
            tenant_id: Some("0190b6a2-0000-7000-8000-0000000000aa".parse().unwrap()),
            issued_at: 1_000,
            expires_at: 1_300,
            protocol_version: PROTOCOL_VERSION,
            op,
        }
    }

    fn sample_capabilities() -> Capabilities {
        Capabilities {
            agent_version: "0.1.0".into(),
            cpu_cores: 4,
            memory_bytes: 8 << 30,
            disk_total_bytes: 80 << 30,
            disk_free_bytes: 40 << 30,
            kernel: "6.8.0-31-generic".into(),
            docker_version: Some("29.9.3".into()),
            docker_mode: Some(DockerMode::Rootless),
            cgroup_v2: true,
            seccomp: true,
            apparmor: true,
            user_namespaces: true,
            storage_quota: Some("xfs".into()),
        }
    }

    #[test]
    fn every_message_survives_a_round_trip() {
        for op in [Operation::Ping { nonce: 7 }, Operation::ReportCapabilities] {
            let e = envelope(op);
            let frame = encode_operation(&e).unwrap();
            assert_eq!(
                parse_operation_frame(&frame).unwrap(),
                Parsed::Known(e.clone())
            );
            // The generic path agrees with the dedicated one.
            let generic: ControlMessage = decode(&frame).unwrap();
            assert_eq!(generic, ControlMessage::Operation(e));
        }
        let results = [
            OperationResult::success(id(), Output::Pong { nonce: 7 }),
            OperationResult::success(id(), Output::Capabilities(sample_capabilities())),
            OperationResult::failure(id(), AgentError::new(ErrorCode::Internal, "boom")),
            OperationResult::failure(
                id(),
                AgentError::new(ErrorCode::Internal, "try later").retryable(),
            ),
        ];
        for r in results {
            let msg = AgentMessage::Result(r);
            assert_eq!(decode::<AgentMessage>(&encode(&msg).unwrap()).unwrap(), msg);
        }
        let hello = AgentMessage::Hello(Hello {
            protocol_version: 1,
            agent_version: "0.1.0".into(),
        });
        assert_eq!(
            decode::<AgentMessage>(&encode(&hello).unwrap()).unwrap(),
            hello
        );
        let welcome = ControlMessage::Welcome(Welcome {
            protocol_version: 1,
            server_time: 1_234,
        });
        assert_eq!(
            decode::<ControlMessage>(&encode(&welcome).unwrap()).unwrap(),
            welcome
        );
    }

    #[test]
    fn the_wire_format_is_pinned() {
        // These strings are the contract with deployed agents: change them only
        // together with PROTOCOL_VERSION or as a purely additive extension.
        let frame = encode_operation(&envelope(Operation::Ping { nonce: 7 })).unwrap();
        assert_eq!(
            frame,
            format!(
                r#"{{"type":"operation","op_id":"{ID}","idempotency_key":"tenant-1:create","tenant_id":"0190b6a2-0000-7000-8000-0000000000aa","issued_at":1000,"expires_at":1300,"protocol_version":1,"op":{{"type":"ping","nonce":7}}}}"#
            )
        );
        assert_eq!(
            encode(&AgentMessage::Hello(Hello {
                protocol_version: 1,
                agent_version: "0.1.0".into()
            }))
            .unwrap(),
            r#"{"type":"hello","protocol_version":1,"agent_version":"0.1.0"}"#
        );
        assert_eq!(
            encode(&AgentMessage::Result(OperationResult::success(
                id(),
                Output::Pong { nonce: 7 }
            )))
            .unwrap(),
            format!(
                r#"{{"type":"result","op_id":"{ID}","status":"succeeded","output":{{"type":"pong","nonce":7}},"error":null}}"#
            )
        );
        assert_eq!(
            encode(&AgentMessage::Result(OperationResult::failure(
                id(),
                AgentError::new(ErrorCode::UnsupportedOperation, "no such operation")
            )))
            .unwrap(),
            format!(
                r#"{{"type":"result","op_id":"{ID}","status":"failed","output":null,"error":{{"code":"unsupported_operation","message":"no such operation","retryable":false}}}}"#
            )
        );
    }

    #[test]
    fn the_enrollment_wire_format_is_pinned() {
        let request = EnrollRequest {
            token: "dhn_x".into(),
            csr_pem: "csr".into(),
            hostname: Some("h".into()),
            agent_version: None,
        };
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"token":"dhn_x","csr_pem":"csr","hostname":"h","agent_version":null}"#
        );
        // Older agents may omit the optional fields.
        let minimal: EnrollRequest =
            serde_json::from_str(r#"{"token":"t","csr_pem":"c"}"#).unwrap();
        assert_eq!((minimal.hostname, minimal.agent_version), (None, None));
        let dbg = format!("{request:?}");
        assert!(
            dbg.contains("hostname") && !dbg.contains("dhn_x") && !dbg.contains("csr"),
            "{dbg}"
        );

        let response = EnrollResponse {
            node_id: id(),
            name: "n1".into(),
            certificate_pem: "cert".into(),
            ca_certificate_pem: "ca".into(),
            ca_fingerprint_sha256: "ab".into(),
            not_before: "2026-01-01T00:00:00Z".into(),
            not_after: "2026-04-01T00:00:00Z".into(),
            protocol_version: 1,
            stream_url: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(
            json,
            format!(
                r#"{{"node_id":"{ID}","name":"n1","certificate_pem":"cert","ca_certificate_pem":"ca","ca_fingerprint_sha256":"ab","not_before":"2026-01-01T00:00:00Z","not_after":"2026-04-01T00:00:00Z","protocol_version":1}}"#
            )
        );
        // An older control plane omits the field; it reads back as absent.
        let with_url = EnrollResponse {
            stream_url: Some("https://panel.example.com:8444".into()),
            ..response.clone()
        };
        let json_with_url = serde_json::to_string(&with_url).unwrap();
        assert!(json_with_url.ends_with(r#""stream_url":"https://panel.example.com:8444"}"#));
        assert_eq!(
            serde_json::from_str::<EnrollResponse>(&json_with_url).unwrap(),
            with_url
        );
        assert_eq!(
            serde_json::from_str::<EnrollResponse>(&json).unwrap(),
            response
        );
        // A newer control plane may add fields.
        let newer = json.replacen('{', r#"{"future":true,"#, 1);
        assert_eq!(
            serde_json::from_str::<EnrollResponse>(&newer).unwrap(),
            response
        );
        assert_eq!(ENROLL_PATH, "/api/v1/public/nodes/enroll");
    }

    #[test]
    fn unknown_operations_do_not_break_the_session() {
        let frame = json!({
            "type": "operation", "op_id": ID, "idempotency_key": "k",
            "issued_at": 1000, "expires_at": 1300, "protocol_version": 1,
            "op": {"type": "teleport_tenant", "tenant": "x", "plan": {"cpus": 2}}
        })
        .to_string();
        match parse_operation_frame(&frame).unwrap() {
            Parsed::Unsupported { header, op_type } => {
                assert_eq!(op_type, "teleport_tenant");
                assert_eq!(header.op_id, id());
                assert_eq!(header.tenant_id, None);
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn known_operations_with_bad_arguments_are_reported_as_such() {
        let frame = json!({
            "type": "operation", "op_id": ID, "idempotency_key": "k",
            "issued_at": 1000, "expires_at": 1300, "protocol_version": 1,
            "op": {"type": "ping", "nonce": "not a number"}
        })
        .to_string();
        match parse_operation_frame(&frame).unwrap() {
            Parsed::InvalidArguments { header, message } => {
                assert_eq!(header.op_id, id());
                assert!(
                    message.contains("nonce") || message.contains("invalid type"),
                    "{message}"
                );
            }
            other => panic!("expected InvalidArguments, got {other:?}"),
        }
    }

    #[test]
    fn newer_peers_may_add_fields_everywhere() {
        // Extra fields on envelope, operation, hello and capabilities are ignored.
        let frame = json!({
            "type": "operation", "op_id": ID, "idempotency_key": "k", "future_field": [1, 2],
            "issued_at": 1000, "expires_at": 1300, "protocol_version": 1,
            "op": {"type": "ping", "nonce": 1, "trace": "abc"}
        })
        .to_string();
        assert!(matches!(
            parse_operation_frame(&frame).unwrap(),
            Parsed::Known(_)
        ));
        let hello =
            r#"{"type":"hello","protocol_version":1,"agent_version":"9.9","features":["x"]}"#;
        assert!(decode::<AgentMessage>(hello).is_ok());
        // Capabilities written by an older agent (without the optional fields) still parse.
        let old = r#"{"agent_version":"0.0.1","cpu_cores":2,"memory_bytes":1,"disk_total_bytes":2,"disk_free_bytes":1,"kernel":"6.8"}"#;
        let c: Capabilities = serde_json::from_str(old).unwrap();
        assert_eq!(
            (c.docker_version, c.docker_mode, c.cgroup_v2),
            (None, None, false)
        );
    }

    #[test]
    fn frames_without_a_usable_header_are_errors_not_unsupported() {
        for bad in [
            "not json",
            "{}",
            r#"{"type":"welcome","protocol_version":1,"server_time":1}"#,
            &json!({"type":"operation","op_id":"nope","idempotency_key":"k","issued_at":1,"expires_at":2,"protocol_version":1,"op":{"type":"ping","nonce":1}}).to_string(),
            &json!({"type":"operation","op_id":ID,"idempotency_key":"k","issued_at":1,"expires_at":2,"protocol_version":1,"op":{"nonce":1}}).to_string(),
        ] {
            assert!(matches!(parse_operation_frame(bad), Err(FrameError::Malformed(_))), "{bad}");
        }
    }

    #[test]
    fn oversized_frames_are_refused_before_parsing() {
        let big = "x".repeat(MAX_FRAME_BYTES + 1);
        assert!(matches!(
            decode::<AgentMessage>(&big),
            Err(FrameError::TooLarge(_))
        ));
        assert!(matches!(
            parse_operation_frame(&big),
            Err(FrameError::TooLarge(_))
        ));
        let hello = AgentMessage::Hello(Hello {
            protocol_version: 1,
            agent_version: "y".repeat(MAX_FRAME_BYTES),
        });
        assert!(matches!(encode(&hello), Err(FrameError::TooLarge(_))));
    }

    #[test]
    fn envelopes_are_accepted_only_inside_their_validity_window() {
        let e = envelope(Operation::ReportCapabilities); // valid 1000..1300
        assert!(e.check_fresh(1_000).is_ok());
        assert!(e.check_fresh(1_299).is_ok());
        assert_eq!(e.check_fresh(1_300).unwrap_err().code, ErrorCode::Expired);
        assert_eq!(e.check_fresh(9_999).unwrap_err().code, ErrorCode::Expired);
        // Issued far in the future (agent clock is behind, or the envelope is forged).
        assert_eq!(
            e.check_fresh(1_000 - MAX_CLOCK_SKEW_SECS - 1)
                .unwrap_err()
                .code,
            ErrorCode::Expired
        );
        assert!(e.check_fresh(1_000 - MAX_CLOCK_SKEW_SECS).is_ok());
        // A window longer than allowed is a replay risk.
        let mut long = e.clone();
        long.expires_at = long.issued_at + MAX_ENVELOPE_LIFETIME_SECS + 1;
        assert_eq!(
            long.check_fresh(1_000).unwrap_err().code,
            ErrorCode::Expired
        );
        // Unsupported versions.
        for v in [0, PROTOCOL_VERSION + 1, u16::MAX] {
            let mut bad = e.clone();
            bad.protocol_version = v;
            assert_eq!(
                bad.check_fresh(1_000).unwrap_err().code,
                ErrorCode::UnsupportedVersion,
                "{v}"
            );
        }
        // Extreme values do not overflow.
        let mut extreme = e;
        extreme.issued_at = i64::MIN;
        extreme.expires_at = i64::MAX;
        assert!(extreme.check_fresh(i64::MAX - 1).is_err());
    }

    #[test]
    fn versions_are_negotiated_down_never_up() {
        assert_eq!(negotiate_version(PROTOCOL_VERSION), Some(PROTOCOL_VERSION));
        assert_eq!(
            negotiate_version(PROTOCOL_VERSION + 5),
            Some(PROTOCOL_VERSION)
        );
        assert_eq!(
            negotiate_version(MIN_SUPPORTED_VERSION),
            Some(MIN_SUPPORTED_VERSION)
        );
        assert_eq!(
            negotiate_version(MIN_SUPPORTED_VERSION.wrapping_sub(1)),
            if MIN_SUPPORTED_VERSION == 0 {
                Some(PROTOCOL_VERSION)
            } else {
                None
            }
        );
    }

    #[test]
    fn operation_names_match_the_wire_and_the_list() {
        for op in [
            Operation::Ping { nonce: 1 },
            Operation::ReportCapabilities,
            Operation::CreateTenant {
                tenant_id: id(),
                username: "alice".into(),
                uid: 10_000,
            },
            Operation::SetStorageQuota {
                tenant_id: id(),
                uid: 10_000,
                disk_bytes: Some(1 << 30),
                inodes: None,
            },
            Operation::ProvisionTenantStack {
                tenant_id: id(),
                username: "alice".into(),
                uid: 10_000,
                cpu_millis: Some(1000),
                memory_bytes: Some(512 << 20),
                pids: None,
            },
            Operation::SetResourceLimits {
                username: "alice".into(),
                cpu_millis: None,
                memory_bytes: Some(1 << 30),
                pids: Some(300),
            },
            Operation::SetUpTenant {
                tenant_id: id(),
                username: "alice".into(),
                uid: 10_000,
                disk_bytes: Some(1 << 30),
                inodes: None,
                cpu_millis: Some(500),
                memory_bytes: Some(256 << 20),
                pids: Some(100),
                skeleton: Vec::new(),
            },
            Operation::ReportCertificates,
            Operation::SetCustomCertificates {
                username: "alice".into(),
                items: vec![],
            },
            Operation::SetSiteRoutes {
                username: "alice".into(),
                routes: vec![SiteRoute {
                    waf: None,
                    blocked_ips: Vec::new(),
                    hosts: vec!["example.com".into()],
                    upstream: Some("bp-alice-web:8080".into()),
                    redirects: vec![],
                    force_https: false,
                    hsts: false,
                    suspended_by: None,
                }],
            },
            Operation::SuspendTenant {
                username: "alice".into(),
            },
            Operation::ResumeTenant {
                username: "alice".into(),
            },
            Operation::SetPhpVersions {
                tenant_id: id(),
                username: "alice".into(),
                uid: 10_000,
                cpu_millis: None,
                memory_bytes: None,
                pids: None,
                php_version: "8.4".into(),
                extra_php: vec!["8.3".into()],
            },
            Operation::SetPhpIni {
                username: "alice".into(),
                ini: "memory_limit = 64M\n".into(),
            },
            Operation::RestartService {
                username: "alice".into(),
                service: "web".into(),
            },
            Operation::SetErrorPages { pages: vec![] },
            Operation::EnsureDbServer,
            Operation::ReportDbSizes,
            Operation::EnsurePgServer,
            Operation::ReportPgSizes,
            Operation::EnsureMailServer {
                hostname: "mail.example.com".into(),
                certificate_pem: None,
                private_key_pem: None,
            },
            Operation::SetMailDomain {
                domain: "example.com".into(),
                catch_all: None,
                per_hour: Some(100),
                held: false,
            },
            Operation::RemoveMailDomain {
                domain: "example.com".into(),
            },
            Operation::UpsertMailbox {
                domain: "example.com".into(),
                local: "info".into(),
                password: None,
                quota_bytes: 1 << 30,
                aliases: vec![],
            },
            Operation::DropMailbox {
                domain: "example.com".into(),
                local: "info".into(),
            },
            Operation::SetMailForwarders {
                domain: "example.com".into(),
                forwarders: vec![],
            },
            Operation::SetMailAutoresponder {
                domain: "example.com".into(),
                local: "info".into(),
                enabled: true,
                subject: "Away".into(),
                body: "Back soon".into(),
                from: None,
                until: None,
            },
            Operation::ReportMail,
            Operation::MailQueueAction {
                action: "flush".into(),
                ids: vec![],
            },
            Operation::MigrateTenantDatabases {
                username: "alice".into(),
                engine: DbEngine::Postgres,
                to_dedicated: true,
                databases: vec!["alice_shop".into()],
                users: vec!["alice_web".into()],
                grants: vec![MigrateGrant {
                    database: "alice_shop".into(),
                    user: "alice_web".into(),
                    access: "all".into(),
                }],
                remote_hosts: vec![],
            },
            Operation::RemoveDedicatedDb {
                username: "alice".into(),
                engine: DbEngine::Mysql,
            },
            Operation::ImportSql {
                username: "alice".into(),
                database: "alice_shop".into(),
                import_id: id(),
                url: "https://panel.example.com/api/v1/public/imports/x/data".into(),
                size_bytes: 10,
                sha256: "0".repeat(64),
                gzip: false,
                token: None,
                engine: DbEngine::Mysql,
                dedicated: None,
            },
            Operation::ExportSql {
                username: "alice".into(),
                database: "alice_shop".into(),
                export_id: id(),
                url: "https://panel.example.com/api/v1/public/exports/x".into(),
                token: None,
                engine: DbEngine::Mysql,
                dedicated: None,
            },
            Operation::SetPhpMyAdminHost {
                host: Some("pma.example.com".into()),
            },
            Operation::OpenPhpMyAdmin {
                username: "alice".into(),
                database: "alice_shop".into(),
                session_id: id(),
                token_hash: "0".repeat(64),
                session_minutes: 60,
            },
            Operation::SetWebmailHost {
                host: Some("webmail.example.com".into()),
            },
            Operation::OpenWebmail {
                address: "alice@example.com".into(),
                token_hash: "0".repeat(64),
            },
            Operation::FileOp {
                tenant_id: id(),
                uid: 10_000,
                request: files::FileRequest::TrashList,
            },
            Operation::StudioOp {
                database: "alice_shop".into(),
                engine: DbEngine::Mysql,
                request: studio::StudioRequest::Tables,
            },
            Operation::CreateBackup {
                tenant_id: id(),
                uid: 10_000,
                backup_id: id(),
                account: "alice".into(),
                target: BackupTarget {
                    kind: "local".into(),
                    settings: serde_json::json!({"path": "/var/backups/builtpanel"}),
                },
                credentials: None,
                key: None,
                databases: vec!["alice_shop".into()],
                dns: serde_json::json!([]),
                meta: serde_json::json!({}),
                created_at: 1,
                db_users: Vec::new(),
                files_since: None,
            },
            Operation::UpsertFtpAccount {
                tenant_id: id(),
                uid: 10_000,
                name: "alice_site".into(),
                home: "www".into(),
                password: None,
                sftp: true,
                ftps: true,
                ftp_plain: false,
                read_only: false,
                quota_mb: Some(100),
                enabled: true,
            },
            Operation::DropFtpAccount {
                name: "alice_site".into(),
            },
            Operation::FetchFile {
                tenant_id: id(),
                uid: 10_000,
                transfer_id: id(),
                dest: "www/a.zip".into(),
                url: "https://panel.example.com/api/v1/public/files/transfers/x".into(),
                size_bytes: 10,
                overwrite: false,
                token: None,
            },
            Operation::SendFile {
                tenant_id: id(),
                uid: 10_000,
                transfer_id: id(),
                path: "www/a.zip".into(),
                url: "https://panel.example.com/api/v1/public/files/transfers/x".into(),
                max_bytes: 10,
                token: None,
            },
            Operation::ReportImportProgress { import_id: id() },
            Operation::CreateDatabase {
                name: "alice_shop".into(),
                engine: DbEngine::Mysql,
                dedicated: None,
            },
            Operation::DropDatabase {
                name: "alice_shop".into(),
                engine: DbEngine::Mysql,
                dedicated: None,
            },
            Operation::UpsertDbUser {
                username: "alice".into(),
                name: "alice_web".into(),
                password: None,
                engine: DbEngine::Mysql,
                dedicated: None,
            },
            Operation::GrantDb {
                database: "alice_shop".into(),
                user: "alice_web".into(),
                access: "all".into(),
                engine: DbEngine::Mysql,
                dedicated: None,
            },
            Operation::RevokeDb {
                database: "alice_shop".into(),
                user: "alice_web".into(),
                engine: DbEngine::Mysql,
                dedicated: None,
            },
            Operation::SetDbRemoteHosts {
                username: "alice".into(),
                name: "alice_web".into(),
                cidrs: vec!["203.0.113.7/32".into()],
                engine: DbEngine::Mysql,
                dedicated: None,
            },
            Operation::SetDbRemoteAccess {
                cidrs: vec!["203.0.113.7/32".into()],
                engine: DbEngine::Mysql,
            },
            Operation::DropDbUser {
                name: "alice_web".into(),
                engine: DbEngine::Mysql,
                dedicated: None,
            },
            Operation::AllowTenantDatabases {
                username: "alice".into(),
                engine: DbEngine::Mysql,
            },
            Operation::AttachDomain {
                username: "alice".into(),
                domain: "example.com".into(),
                config: "server {}".into(),
            },
            Operation::DetachDomain {
                username: "alice".into(),
                domain: "example.com".into(),
            },
            Operation::KeepDomains {
                username: "alice".into(),
                domains: vec!["example.com".into()],
            },
            Operation::DeleteTenant {
                tenant_id: id(),
                username: "alice".into(),
                uid: 10_000,
                purge_data: true,
            },
            Operation::SetRuntimeApp {
                app_id: id(),
                tenant_id: id(),
                username: "alice".into(),
                uid: 10_000,
                kind: "node".into(),
                version: "24".into(),
                name: "api".into(),
                dir: "api".into(),
                command: vec!["node".into(), "server.js".into()],
                install: true,
                port: 3000,
                memory_mb: 256,
                env: None,
                image: None,
            },
            Operation::RemoveRuntimeApp {
                tenant_id: id(),
                username: "alice".into(),
                name: "api".into(),
            },
            Operation::RunGitDeploy {
                deploy_id: id(),
                tenant_id: id(),
                username: "alice".into(),
                uid: 10_000,
                name: "site".into(),
                repo_url: "https://git.example.com/alice/site.git".into(),
                branch: "main".into(),
                dest: "site".into(),
                build_kind: Some("node".into()),
                build_version: Some("24".into()),
                build_steps: vec![vec!["npm".into(), "ci".into()]],
                restart_app: None,
                token: None,
            },
            Operation::RemoveGitDeploy {
                tenant_id: id(),
                username: "alice".into(),
                name: "site".into(),
            },
            Operation::ReadTraffic {
                tenant_id: id(),
                username: "alice".into(),
                since_ms: 0,
                top: 10,
            },
            Operation::ReadWafEvents {
                tenant_id: id(),
                hosts: vec!["a.example".into()],
                since_ms: 0,
                limit: 50,
            },
        ] {
            let wire = serde_json::to_value(&op).unwrap();
            assert_eq!(wire["type"], op.type_name());
            assert!(Operation::TYPE_NAMES.contains(&op.type_name()));
        }
        assert_eq!(
            Operation::TYPE_NAMES.len(),
            98,
            "update TYPE_NAMES and type_name() together with the enum"
        );
    }

    #[test]
    fn blocked_addresses_are_made_canonical_or_refused() {
        for (given, want) in [
            ("203.0.113.9", "203.0.113.9"),
            (" 203.0.113.9/32 ", "203.0.113.9"),
            ("203.0.113.77/24", "203.0.113.0/24"),
            ("10.1.2.3/8", "10.0.0.0/8"),
            ("2001:DB8::1", "2001:db8::1"),
            ("2001:db8:1:2::5/64", "2001:db8:1:2::/64"),
        ] {
            assert_eq!(normalize_ip_block(given).as_deref(), Ok(want), "{given}");
        }
        for bad in [
            "",
            "x",
            "1.2.3",
            "1.2.3.4/",
            "1.2.3.4/33",
            "1.2.3.4/-1",
            "1.2.3.4/ 8",
            "1.2.3.4/x",
            "0.0.0.0",
            "127.0.0.1",
            "::1",
            "224.0.0.1",
            "ff02::1",
            "0.0.0.0/0",
            "10.0.0.0/7",
            "2001:db8::/15",
            "1.2.3.4/8/8",
            "1.2.3.4 ",
        ] {
            let r = normalize_ip_block(bad);
            // A trailing blank is trimmed, so `1.2.3.4 ` is fine; everything else is refused.
            if bad == "1.2.3.4 " {
                assert_eq!(r.as_deref(), Ok("1.2.3.4"));
            } else {
                assert!(r.is_err(), "{bad:?} -> {r:?}");
            }
        }
    }

    #[test]
    fn skeleton_files_are_checked_for_paths_sizes_and_clashes() {
        let f = |p: &str, c: &str| SkeletonFile {
            path: p.into(),
            content: c.into(),
        };
        assert!(
            check_skeleton(&[f("index.html", "x"), f("css/a.css", ""), f(".htaccess", "")]).is_ok()
        );
        for bad in [
            "", "/a", "../a", "a/../b", "a//b", ".", "a/", "a b", "a\\b", "é",
        ] {
            assert!(check_skeleton(&[f(bad, "x")]).is_err(), "{bad:?}");
        }
        assert!(check_skeleton(&[f("a", "1"), f("a", "2")]).is_err());
        assert!(check_skeleton(&[f("a", "1"), f("a/b", "2")]).is_err());
        assert!(check_skeleton(&[f("a/b", "1"), f("a", "2")]).is_err());
        assert!(check_skeleton(&[f("a", "1"), f("ab/c", "2")]).is_ok());
        assert!(check_skeleton(&[f("a", &"x".repeat(SKELETON_MAX_FILE_BYTES + 1))]).is_err());
        assert!(check_skeleton(&[f("a", "x\0")]).is_err());
        let heavy: Vec<_> = (0..3)
            .map(|i| f(&format!("f{i}"), &"x".repeat(SKELETON_MAX_FILE_BYTES)))
            .collect();
        assert!(check_skeleton(&heavy).is_err());
        // The shipped default has to fit in one operation, with room to spare.
        let worst = Operation::SetUpTenant {
            tenant_id: Uuid::nil(),
            username: "x".repeat(32),
            uid: 59_999,
            disk_bytes: Some(u64::MAX),
            inodes: Some(u64::MAX),
            cpu_millis: Some(u32::MAX),
            memory_bytes: Some(u64::MAX),
            pids: Some(u32::MAX),
            skeleton: (0..SKELETON_MAX_FILES)
                .map(|i| {
                    f(
                        &format!("{i:02}{}", "n".repeat(60)),
                        &"\"".repeat(SKELETON_MAX_TOTAL_BYTES / SKELETON_MAX_FILES),
                    )
                })
                .collect(),
        };
        // Quotes double in JSON: the worst case still fits the ledger (64 KiB) and the helper (64 KiB).
        assert!(serde_json::to_string(&worst).unwrap().len() < 56 * 1024);
    }

    #[test]
    fn a_database_password_is_never_shown_and_the_ledger_form_has_none() {
        let op = Operation::UpsertDbUser {
            username: "alice".into(),
            name: "alice_web".into(),
            password: Some(DbPassword("Sup3r-secret-pass".into())),
            engine: DbEngine::Mysql,
            dedicated: None,
        };
        assert!(!format!("{op:?}").contains("secret"));
        let wire = serde_json::to_string(&op).unwrap();
        assert!(wire.contains("Sup3r-secret-pass"), "the wire has it");
        let ledger = Operation::UpsertDbUser {
            username: "alice".into(),
            name: "alice_web".into(),
            password: None,
            engine: DbEngine::Mysql,
            dedicated: None,
        };
        assert!(!serde_json::to_string(&ledger).unwrap().contains("password"));
        let back: Operation =
            serde_json::from_str(&serde_json::to_string(&ledger).unwrap()).unwrap();
        assert_eq!(back, ledger);
    }

    #[test]
    fn a_private_key_is_never_shown_and_the_ledger_form_has_none() {
        let with_key = CustomCertificate {
            host: "example.com".into(),
            certificate: "CERT".into(),
            private_key: Some(
                "-----BEGIN PRIVATE KEY-----\nSECRET\n-----END PRIVATE KEY-----".into(),
            ),
        };
        let op = Operation::SetCustomCertificates {
            username: "alice".into(),
            items: vec![with_key.clone()],
        };
        assert!(!format!("{op:?}").contains("SECRET"), "{op:?}");
        assert!(format!("{op:?}").contains("has_key: true"));
        // On the wire the key is there; without one the field is left out altogether.
        assert!(serde_json::to_string(&op).unwrap().contains("SECRET"));
        let bare = CustomCertificate {
            private_key: None,
            ..with_key
        };
        let json = serde_json::to_value(&bare).unwrap();
        assert!(json.get("private_key").is_none(), "{json}");
        assert_eq!(
            serde_json::from_value::<CustomCertificate>(json).unwrap(),
            bare
        );
    }

    #[test]
    fn failures_map_to_the_right_status() {
        let id = id();
        let retry =
            OperationResult::failure(id, AgentError::new(ErrorCode::Internal, "x").retryable());
        assert_eq!(retry.status, ResultStatus::Retryable);
        let fail = OperationResult::failure(id, AgentError::new(ErrorCode::InvalidArguments, "x"));
        assert_eq!(fail.status, ResultStatus::Failed);
        assert!(fail.output.is_none() && fail.error.is_some());
        let ok = OperationResult::success(id, Output::Pong { nonce: 1 });
        assert!(ok.error.is_none() && ok.output.is_some());
        assert_eq!(
            AgentError::new(ErrorCode::Expired, "late").to_string(),
            "Expired: late"
        );
    }
}
