//! The stack that runs for one tenant (T04.06).
//!
//! A *stack template* says which containers make a tenant's site, how the
//! plan's resources are divided between them and what they mount. Rendering it
//! for a tenant ([`render`]) gives ready container specifications for the
//! docker-manager crate, which adds its own hardening on top (all capabilities
//! dropped, no new privileges, read-only root, limits, no host mounts).
//!
//! Three rules are enforced here, and checked by the security checklist in the
//! tests:
//! * **Only allowed images.** Every image comes from the allow-list
//!   (`deploy/stack/images.toml`): pinned by digest, listed by a reviewed change.
//! * **Never root.** Every container runs as the tenant's own numeric id, which
//!   is always 10000 or higher.
//! * **Nothing shared.** Names, the network and the volumes are the tenant's
//!   own (`bp-<username>-...`); no container publishes a port, mounts a host
//!   path, or joins another tenant's network.

use std::{collections::BTreeMap, fmt};

use builtpanel_docker_manager::{
    Capability, ContainerSpec, ImageRef, Limits, Name, Network, PublishedPort, Restart, SpecError,
    Tmpfs, VolumeMount,
};
use builtpanel_domain::{TenantUsername, Uid};
use serde::Deserialize;
use uuid::Uuid;

/// The embedded allow-list.
pub const EMBEDDED_ALLOWLIST: &str = include_str!("../../../deploy/stack/images.toml");

/// What an unlimited plan resource is capped to for the stack as a whole: a
/// tenant that is "unlimited" is still not allowed to exhaust a node.
pub const UNLIMITED_CPU_MILLIS: u32 = 4_000;
/// See [`UNLIMITED_CPU_MILLIS`].
pub const UNLIMITED_MEMORY_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// See [`UNLIMITED_CPU_MILLIS`].
pub const UNLIMITED_PIDS: u32 = 1_024;

/// The part a container plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The web server in front.
    Web,
    /// PHP-FPM behind it.
    Php,
    /// The node's shared database server (not part of a tenant's stack).
    Db,
    /// The node's phpMyAdmin (not part of a tenant's stack).
    Pma,
    /// The node's PostgreSQL server (not part of a tenant's stack; T07.12).
    Pg,
    /// The node's mail server (not part of a tenant's stack; T13.02).
    Mail,
    /// The node's webmail, Roundcube (not part of a tenant's stack; T13.09).
    Webmail,
    /// The node's SFTP and FTPS server (not part of a tenant's stack).
    Sftp,
    /// The node's malware scanner, run for one scan at a time (not part of a tenant's stack).
    Clamav,
    /// A tenant's own Redis (a cache add-on, not part of the default stack).
    Redis,
    /// A tenant's own Memcached (a cache add-on, not part of the default stack).
    Memcached,
    /// WP-CLI with the PHP extensions WordPress needs, run once per install or change of a
    /// WordPress site (not part of a tenant's stack).
    Wpcli,
    /// The node's image vulnerability scanner, run for one image at a time (not part of a
    /// tenant's stack).
    Trivy,
    /// A tenant's own Node.js application (T12.07; not part of the default stack).
    Node,
    /// A tenant's own Python application (T12.08; not part of the default stack).
    Python,
    /// Git, run once per deploy of a tenant's site from a repository (T12.09; not part of the
    /// default stack).
    Git,
}

impl Role {
    /// The name used in labels and in the allow-list.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Php => "php",
            Self::Db => "db",
            Self::Pma => "pma",
            Self::Pg => "pg",
            Self::Mail => "mail",
            Self::Webmail => "webmail",
            Self::Sftp => "sftp",
            Self::Clamav => "clamav",
            Self::Trivy => "trivy",
            Self::Wpcli => "wpcli",
            Self::Redis => "redis",
            Self::Memcached => "memcached",
            Self::Node => "node",
            Self::Python => "python",
            Self::Git => "git",
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a stack could not be rendered or an allow-list not read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StackError {
    /// The allow-list text is not valid.
    #[error("image allow-list: {0}")]
    Allowlist(String),
    /// The allow-list has no image for a role the template needs.
    #[error("no allowed image for the `{0}` role")]
    NoImage(Role),
    /// An image that is not in the allow-list was asked for.
    #[error("the image for `{0}` is not on the allow-list")]
    ImageNotAllowed(Role),
    /// No image runs this PHP version.
    #[error("PHP {0} is not available")]
    UnknownPhpVersion(String),
    /// No image runs this version of a runtime.
    #[error("{0} {1} is not available")]
    UnknownRuntimeVersion(Role, String),
    /// The plan leaves too little of a resource to run the stack.
    #[error("the plan's {0} is too small to run a stack")]
    PlanTooSmall(&'static str),
    /// A rendered specification is not valid.
    #[error(transparent)]
    Spec(#[from] SpecError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAllowlist {
    image: Vec<RawImage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawImage {
    role: Role,
    /// The PHP version a `php` image runs (`8.4`).
    version: Option<String>,
    reference: String,
    /// The moving tag the pin was taken from (`8.4-fpm-alpine`): what to ask a registry about
    /// to learn whether a newer image exists.
    tag: Option<String>,
    /// Our own image for the same role (and version), tried first; `reference`
    /// is what is used when it cannot be pulled.
    preferred: Option<String>,
    #[serde(default)]
    note: String,
}

/// The images that may run, by role, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allowlist {
    by_role: BTreeMap<Role, Vec<ImageRef>>,
    /// The PHP versions that may run, with their images, default first.
    php: Vec<(String, ImageRef)>,
    /// The versions of the runtimes tenants run their own applications on (Node.js, Python),
    /// with their images, newest first.
    runtimes: Vec<(Role, String, ImageRef)>,
    /// For a PHP version with an image of our own: the official one to fall back to.
    fallbacks: Vec<(ImageRef, ImageRef)>,
    /// Every image listed, as written.
    entries: Vec<ImageEntry>,
}

/// One image of the list, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageEntry {
    /// The role it plays.
    pub role: Role,
    /// The pinned reference (`name@sha256:…`).
    pub reference: ImageRef,
    /// The moving tag it was pinned from, when one is known (`name:tag` is then the thing to
    /// ask a registry about).
    pub tag: Option<String>,
    /// What it is.
    pub note: String,
}

/// Whether `text` is the version of a runtime as the panel writes it: `24`, `3.13`.
#[must_use]
pub fn is_runtime_version(text: &str) -> bool {
    (1..=6).contains(&text.len())
        && !text.starts_with('.')
        && !text.ends_with('.')
        && text.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

/// Whether `text` is a PHP version as the panel writes it: `8.4`.
#[must_use]
pub fn is_php_version(text: &str) -> bool {
    text.split_once('.').is_some_and(|(major, minor)| {
        (1..=2).contains(&major.len())
            && (1..=2).contains(&minor.len())
            && major.bytes().all(|b| b.is_ascii_digit())
            && minor.bytes().all(|b| b.is_ascii_digit())
    })
}

impl Allowlist {
    /// Reads an allow-list. Every reference must be pinned by digest.
    ///
    /// # Errors
    /// [`StackError::Allowlist`] for TOML errors, tags, duplicates or a missing role.
    pub fn parse(text: &str) -> Result<Self, StackError> {
        let raw: RawAllowlist = toml::from_str(text)
            .map_err(|e| StackError::Allowlist(e.to_string().replace('\n', " ")))?;
        let mut by_role: BTreeMap<Role, Vec<ImageRef>> = BTreeMap::new();
        let mut php: Vec<(String, ImageRef)> = Vec::new();
        let mut runtimes: Vec<(Role, String, ImageRef)> = Vec::new();
        let mut fallbacks: Vec<(ImageRef, ImageRef)> = Vec::new();
        let mut entries: Vec<ImageEntry> = Vec::new();
        for image in raw.image {
            let reference = ImageRef::parse(&image.reference).map_err(|e| {
                StackError::Allowlist(format!(
                    "{} ({}): {e}",
                    image.role,
                    image.note.chars().take(40).collect::<String>()
                ))
            })?;
            if let Some(tag) = &image.tag
                && (tag.is_empty()
                    || tag.len() > 128
                    || !tag
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)))
            {
                return Err(StackError::Allowlist(format!(
                    "{}: a tag that is not valid",
                    image.role
                )));
            }
            entries.push(ImageEntry {
                role: image.role,
                reference: reference.clone(),
                tag: image.tag.clone(),
                note: image.note.clone(),
            });
            let preferred = image
                .preferred
                .as_deref()
                .map(ImageRef::parse)
                .transpose()
                .map_err(|e| StackError::Allowlist(format!("preferred image: {e}")))?;
            // Ours and the official one are two images: the same one twice would make the
            // fallback point at itself.
            if preferred.as_ref() == Some(&reference) {
                return Err(StackError::Allowlist(format!(
                    "{} lists the same image twice",
                    image.role
                )));
            }
            let mut own_image: Option<ImageRef> = None;
            match (image.role, image.version) {
                (Role::Php, Some(version)) if is_php_version(&version) => {
                    if php.iter().any(|(v, _)| *v == version) {
                        return Err(StackError::Allowlist(format!(
                            "php {version} is listed twice"
                        )));
                    }
                    match preferred {
                        // Ours first; the official one when ours cannot be pulled.
                        Some(own) => {
                            php.push((version, own.clone()));
                            fallbacks.push((own.clone(), reference.clone()));
                            own_image = Some(own);
                        }
                        None => php.push((version, reference.clone())),
                    }
                }
                (Role::Php, _) => {
                    return Err(StackError::Allowlist(
                        "a php image needs its version, like \"8.4\"".to_owned(),
                    ));
                }
                (role @ (Role::Node | Role::Python), Some(version))
                    if is_runtime_version(&version) =>
                {
                    if runtimes.iter().any(|(r, v, _)| *r == role && *v == version) {
                        return Err(StackError::Allowlist(format!(
                            "{role} {version} is listed twice"
                        )));
                    }
                    match preferred {
                        Some(own) => {
                            runtimes.push((role, version, own.clone()));
                            fallbacks.push((own.clone(), reference.clone()));
                            own_image = Some(own);
                        }
                        None => runtimes.push((role, version, reference.clone())),
                    }
                }
                (Role::Node | Role::Python, _) => {
                    return Err(StackError::Allowlist(
                        "a node or python image needs its version, like \"24\" or \"3.13\""
                            .to_owned(),
                    ));
                }
                (_, Some(_)) => {
                    return Err(StackError::Allowlist(format!(
                        "only php images have a version ({})",
                        image.role
                    )));
                }
                (_, None) => {
                    // Any other role: ours first, the official one when ours cannot be pulled.
                    if let Some(own) = preferred {
                        fallbacks.push((own.clone(), reference.clone()));
                        own_image = Some(own);
                    }
                }
            }
            let list = by_role.entry(image.role).or_default();
            if list.contains(&reference) || own_image.as_ref().is_some_and(|o| list.contains(o)) {
                return Err(StackError::Allowlist(format!(
                    "{} lists the same image twice",
                    image.role
                )));
            }
            // Our own image first, the official one after it: both may run.
            list.extend(own_image);
            list.push(reference);
        }
        for role in [Role::Web, Role::Php] {
            if !by_role.contains_key(&role) {
                return Err(StackError::Allowlist(format!(
                    "no image for the `{role}` role"
                )));
            }
        }
        Ok(Self {
            by_role,
            php,
            runtimes,
            fallbacks,
            entries,
        })
    }

    /// The list shipped with this build.
    ///
    /// # Errors
    /// [`StackError::Allowlist`] (a build mistake; tested).
    pub fn embedded() -> Result<Self, StackError> {
        Self::parse(EMBEDDED_ALLOWLIST)
    }

    /// Every image listed, in the order written.
    #[must_use]
    pub fn entries(&self) -> &[ImageEntry] {
        &self.entries
    }

    /// The image new stacks use for a role.
    ///
    /// # Errors
    /// [`StackError::NoImage`].
    pub fn current(&self, role: Role) -> Result<&ImageRef, StackError> {
        self.by_role
            .get(&role)
            .and_then(|list| list.first())
            .ok_or(StackError::NoImage(role))
    }

    /// The PHP versions that may run, the one new accounts start with first.
    #[must_use]
    pub fn php_versions(&self) -> Vec<&str> {
        self.php.iter().map(|(v, _)| v.as_str()).collect()
    }

    /// The version new accounts start with.
    ///
    /// # Errors
    /// [`StackError::NoImage`].
    pub fn default_php_version(&self) -> Result<&str, StackError> {
        self.php
            .first()
            .map(|(v, _)| v.as_str())
            .ok_or(StackError::NoImage(Role::Php))
    }

    /// The image that runs a PHP version.
    ///
    /// # Errors
    /// [`StackError::UnknownPhpVersion`].
    pub fn php_image(&self, version: &str) -> Result<&ImageRef, StackError> {
        self.php
            .iter()
            .find(|(v, _)| v == version)
            .map(|(_, image)| image)
            .ok_or_else(|| StackError::UnknownPhpVersion(version.chars().take(10).collect()))
    }

    /// The versions of a runtime (`node`, `python`) that may run, the newest first.
    #[must_use]
    pub fn runtime_versions(&self, role: Role) -> Vec<&str> {
        self.runtimes
            .iter()
            .filter(|(r, _, _)| *r == role)
            .map(|(_, v, _)| v.as_str())
            .collect()
    }

    /// The image that runs a version of a runtime.
    ///
    /// # Errors
    /// [`StackError::UnknownRuntimeVersion`].
    pub fn runtime_image(&self, role: Role, version: &str) -> Result<&ImageRef, StackError> {
        self.runtimes
            .iter()
            .find(|(r, v, _)| *r == role && v == version)
            .map(|(_, _, image)| image)
            .ok_or_else(|| {
                StackError::UnknownRuntimeVersion(role, version.chars().take(10).collect())
            })
    }

    /// The official image to use when `image` (one of ours) cannot be pulled.
    #[must_use]
    pub fn fallback_for(&self, image: &ImageRef) -> Option<&ImageRef> {
        self.fallbacks
            .iter()
            .find(|(own, _)| own == image)
            .map(|(_, official)| official)
    }

    /// Whether `image` may run in `role`.
    #[must_use]
    pub fn allows(&self, role: Role, image: &ImageRef) -> bool {
        self.by_role
            .get(&role)
            .is_some_and(|list| list.contains(image))
    }
}

/// What the plan gives the whole stack. `None` means unlimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanResources {
    /// CPU in thousandths of a core.
    pub cpu_millis: Option<u32>,
    /// Memory in bytes.
    pub memory_bytes: Option<u64>,
    /// Processes.
    pub pids: Option<u32>,
}

/// The tenant a stack is rendered for.
#[derive(Debug, Clone)]
pub struct Tenant<'a> {
    /// Its id.
    pub id: Uuid,
    /// Its system name.
    pub username: &'a TenantUsername,
    /// Its numeric id (also its group id).
    pub uid: Uid,
    /// What its plan allows.
    pub resources: PlanResources,
    /// The PHP version of its main PHP container (`None`: the one new accounts
    /// start with).
    pub php_version: Option<&'a str>,
    /// More PHP versions that run next to it, for domains that ask for another
    /// version. Each gets its own container; the PHP share of the plan is
    /// divided between all of them.
    pub extra_php: &'a [&'a str],
}

/// One container of the template.
struct ServiceTemplate {
    role: Role,
    /// Share of the plan's resources, in percent (the shares add up to 100).
    share: u32,
    min_memory_bytes: u64,
    min_pids: u32,
    command: &'static [&'static str],
    /// Memory-backed scratch directories: path and size.
    tmpfs: &'static [(&'static str, u64)],
    /// Where the files volume is mounted, and whether read-only.
    files_at: &'static str,
    files_read_only: bool,
    /// Where the web server configuration volume is mounted (read-only), if the
    /// service reads one.
    conf_at: Option<&'static str>,
    /// Where the php.ini options volume is mounted (read-only), if the service
    /// reads one.
    ini_at: Option<&'static str>,
    /// Environment the service always gets (no secrets: fixed paths).
    env: &'static [(&'static str, &'static str)],
}

/// Where PHP containers read the account's options.
const PHP_INI_DIR: &str = "/usr/local/etc/php/builtpanel.d";

const MIB: u64 = 1024 * 1024;

/// The web + PHP template. nginx is the unprivileged build (port 8080), so it
/// needs no capability; PHP-FPM listens on 9000 inside the tenant's network.
const WEB_PHP: &[ServiceTemplate] = &[
    ServiceTemplate {
        role: Role::Web,
        share: 25,
        min_memory_bytes: 32 * MIB,
        min_pids: 16,
        command: &[],
        tmpfs: &[
            ("/tmp", 16 * MIB),
            ("/var/cache/nginx", 64 * MIB),
            ("/run", MIB),
        ],
        files_at: "/var/www/html",
        files_read_only: true,
        conf_at: Some("/etc/nginx/conf.d"),
        ini_at: None,
        env: &[],
    },
    ServiceTemplate {
        role: Role::Php,
        share: 75,
        min_memory_bytes: 64 * MIB,
        min_pids: 32,
        command: &[],
        tmpfs: &[("/tmp", 64 * MIB), ("/run", MIB)],
        files_at: "/var/www/html",
        files_read_only: false,
        conf_at: None,
        // The options the account chose are read from a directory of their own,
        // next to the ones the image has (a leading `:` keeps the image's).
        ini_at: Some(PHP_INI_DIR),
        env: &[("PHP_INI_SCAN_DIR", ":/usr/local/etc/php/builtpanel.d")],
    },
];

/// A rendered stack: the tenant's network, volumes and containers.
#[derive(Debug, Clone)]
pub struct Stack {
    /// The tenant's private network.
    pub network: Name,
    /// The tenant's volumes (the site files, and the web server's configuration).
    pub volumes: Vec<Name>,
    /// Containers, in the order to start them (PHP before the web server).
    pub services: Vec<ContainerSpec>,
}

fn name(tenant: &TenantUsername, suffix: &str) -> Result<Name, SpecError> {
    Name::parse(&format!("bp-{tenant}-{suffix}"))
}

/// The port the tenant's web container listens on (the unprivileged nginx).
pub const WEB_PORT: u16 = 8080;

/// Where the web server (Caddy) reaches a tenant's web container: its name on
/// the tenant's network and its port.
///
/// # Errors
/// [`SpecError`] (cannot happen for a valid tenant name).
pub fn web_upstream(tenant: &TenantUsername) -> Result<String, SpecError> {
    Ok(format!("{}:{WEB_PORT}", name(tenant, "web")?.as_str()))
}

/// Name of a tenant's private network.
///
/// # Errors
/// [`SpecError`] (cannot happen for a valid tenant name).
pub fn network_name(tenant: &TenantUsername) -> Result<Name, SpecError> {
    name(tenant, "net")
}

/// Name of a tenant's files volume.
///
/// # Errors
/// [`SpecError`] (cannot happen for a valid tenant name).
pub fn files_volume_name(tenant: &TenantUsername) -> Result<Name, SpecError> {
    name(tenant, "www")
}

/// The port the tenant's PHP-FPM container listens on.
pub const PHP_PORT: u16 = 9000;

/// Where the tenant's web container reaches its PHP: the PHP container's name on
/// the tenant's network and its port.
///
/// # Errors
/// [`SpecError`] (cannot happen for a valid tenant name).
pub fn php_upstream(tenant: &TenantUsername) -> Result<String, SpecError> {
    Ok(format!(
        "{}:{PHP_PORT}",
        name(tenant, Role::Php.as_str())?.as_str()
    ))
}

/// Name of a tenant's php.ini options volume (a directory the node agent writes
/// and the PHP containers only read).
///
/// # Errors
/// [`SpecError`] (cannot happen for a valid tenant name).
pub fn php_ini_volume_name(tenant: &TenantUsername) -> Result<Name, SpecError> {
    name(tenant, "phpini")
}

/// Name of a tenant's web container.
///
/// # Errors
/// [`SpecError`] (cannot happen for a valid tenant name).
pub fn web_container_name(tenant: &TenantUsername) -> Result<Name, SpecError> {
    name(tenant, Role::Web.as_str())
}

/// Name of a tenant's main PHP container (where its cron jobs run).
///
/// # Errors
/// [`SpecError`] (cannot happen for a valid tenant name).
pub fn php_container_name(tenant: &TenantUsername) -> Result<Name, SpecError> {
    name(tenant, Role::Php.as_str())
}

/// Name of a tenant's web server configuration volume: the site files of
/// nginx's `conf.d`, written by the node agent and only read by the container.
///
/// # Errors
/// [`SpecError`] (cannot happen for a valid tenant name).
pub fn conf_volume_name(tenant: &TenantUsername) -> Result<Name, SpecError> {
    name(tenant, "conf")
}

/// A service's share of the plan, or why the plan is too small for it.
fn service_limits(
    template: &ServiceTemplate,
    resources: &PlanResources,
    copies: u64,
) -> Result<Limits, StackError> {
    let cpu = resources.cpu_millis.unwrap_or(UNLIMITED_CPU_MILLIS);
    let memory = resources.memory_bytes.unwrap_or(UNLIMITED_MEMORY_BYTES);
    let pids = resources.pids.unwrap_or(UNLIMITED_PIDS);
    // The share is divided between the containers that play the part.
    let share = u64::from(template.share);
    let copies = copies.max(1);
    let limits = Limits {
        memory_bytes: memory * share / 100 / copies,
        pids_limit: u32::try_from(u64::from(pids) * share / 100 / copies).unwrap_or(u32::MAX),
        cpu_millis: u32::try_from(u64::from(cpu) * share / 100 / copies).unwrap_or(u32::MAX),
    };
    if limits.memory_bytes < template.min_memory_bytes {
        return Err(StackError::PlanTooSmall("memory"));
    }
    if limits.pids_limit < template.min_pids {
        return Err(StackError::PlanTooSmall("process limit"));
    }
    if limits.cpu_millis < 10 {
        return Err(StackError::PlanTooSmall("CPU"));
    }
    Ok(limits)
}

/// Name of the container of an extra PHP version: `bp-<user>-php-8-3`.
///
/// # Errors
/// [`SpecError`] (a bad version or tenant name).
pub fn php_extra_name(tenant: &TenantUsername, version: &str) -> Result<Name, SpecError> {
    if !is_php_version(version) {
        return Err(SpecError {
            field: "name",
            reason: "not a PHP version".to_owned(),
        });
    }
    name(tenant, &format!("php-{}", version.replace('.', "-")))
}

/// Where the web server reaches the PHP of a site: the main PHP container when
/// the site uses the account's version, the container of its own version otherwise.
///
/// # Errors
/// [`SpecError`].
pub fn php_upstream_for(
    tenant: &TenantUsername,
    site_version: Option<&str>,
    account_version: &str,
) -> Result<String, SpecError> {
    match site_version {
        Some(v) if v != account_version => Ok(format!(
            "{}:{PHP_PORT}",
            php_extra_name(tenant, v)?.as_str()
        )),
        _ => php_upstream(tenant),
    }
}

/// The limits each container of a tenant's stack gets from a plan, by container
/// name, in start order (the main PHP container, the extra ones, the web server).
/// What [`render`] puts in the specifications, for changing a running stack when
/// the plan changes.
///
/// # Errors
/// [`StackError`]: the plan is too small to run the stack.
pub fn plan_limits(
    username: &TenantUsername,
    resources: &PlanResources,
    extra_php: &[&str],
) -> Result<Vec<(Name, Limits)>, StackError> {
    let php_copies = 1 + extra_php.len() as u64;
    let mut out = Vec::new();
    for template in WEB_PHP.iter().rev() {
        let copies = if template.role == Role::Php {
            php_copies
        } else {
            1
        };
        let limits = service_limits(template, resources, copies)?;
        out.push((name(username, template.role.as_str())?, limits));
        if template.role == Role::Php {
            for version in extra_php {
                out.push((php_extra_name(username, version)?, limits));
            }
        }
    }
    Ok(out)
}

/// Renders the web + PHP stack for a tenant from the allow-list.
///
/// # Errors
/// [`StackError`]: no image for a role, a plan too small to run the stack, or a
/// specification the docker-manager crate refuses.
pub fn render(tenant: &Tenant<'_>, allowlist: &Allowlist) -> Result<Stack, StackError> {
    let network = network_name(tenant.username)?;
    let files = files_volume_name(tenant.username)?;
    let conf = conf_volume_name(tenant.username)?;
    let ini = php_ini_volume_name(tenant.username)?;
    let user = Some((tenant.uid.get(), tenant.uid.get()));
    let account_version = match tenant.php_version {
        Some(v) => v,
        None => allowlist.default_php_version()?,
    };
    let php_copies = 1 + tenant.extra_php.len() as u64;
    let mut services = Vec::new();
    // PHP first, so the web server never starts without its backend.
    for template in WEB_PHP.iter().rev() {
        // The containers this part makes: one, or (PHP) one per version.
        let mut makes: Vec<(Name, ImageRef)> = Vec::new();
        match template.role {
            Role::Web => {
                let image = allowlist.current(Role::Web)?.clone();
                makes.push((name(tenant.username, "web")?, image));
            }
            Role::Php => {
                makes.push((
                    name(tenant.username, "php")?,
                    allowlist.php_image(account_version)?.clone(),
                ));
                for version in tenant.extra_php {
                    if *version == account_version
                        || tenant.extra_php.iter().filter(|v| *v == version).count() > 1
                    {
                        return Err(StackError::UnknownPhpVersion(format!(
                            "{version} is listed twice"
                        )));
                    }
                    makes.push((
                        php_extra_name(tenant.username, version)?,
                        allowlist.php_image(version)?.clone(),
                    ));
                }
            }
            // The database server is the node's, not a tenant's.
            Role::Db
            | Role::Pma
            | Role::Pg
            | Role::Mail
            | Role::Webmail
            | Role::Sftp
            | Role::Clamav
            | Role::Trivy
            | Role::Wpcli
            | Role::Redis
            | Role::Memcached
            | Role::Node
            | Role::Python
            | Role::Git => {
                continue;
            }
        }
        let copies = if template.role == Role::Php {
            php_copies
        } else {
            1
        };
        let limits = service_limits(template, &tenant.resources, copies)?;
        for (container, image) in makes {
            if !allowlist.allows(template.role, &image) {
                return Err(StackError::ImageNotAllowed(template.role));
            }
            let mut spec = ContainerSpec::new(container, image);
            spec.command = template.command.iter().map(|a| (*a).to_owned()).collect();
            spec.user = user;
            spec.memory_bytes = limits.memory_bytes;
            spec.cpu_millis = limits.cpu_millis;
            spec.pids_limit = limits.pids_limit;
            spec.tmpfs = template
                .tmpfs
                .iter()
                .map(|(target, size)| Tmpfs {
                    target: (*target).to_owned(),
                    size_bytes: *size,
                })
                .collect();
            spec.volumes = vec![VolumeMount {
                volume: files.clone(),
                target: template.files_at.to_owned(),
                read_only: template.files_read_only,
            }];
            if let Some(target) = template.conf_at {
                spec.volumes.push(VolumeMount {
                    volume: conf.clone(),
                    target: target.to_owned(),
                    read_only: true,
                });
            }
            if let Some(target) = template.ini_at {
                spec.volumes.push(VolumeMount {
                    volume: ini.clone(),
                    target: target.to_owned(),
                    read_only: true,
                });
            }
            spec.env = template
                .env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect();
            spec.network = Network::Named(network.clone());
            // The web server of the node reaches a site's web container on the loopback address
            // of the machine (a port Docker picks; the node agent reads it back). Nobody outside
            // the machine can reach it, and containers cannot see the host's loopback.
            if template.role == Role::Web {
                spec.ports = vec![PublishedPort {
                    container_port: WEB_PORT,
                    host_port: 0,
                    host_ip: std::net::IpAddr::from([127, 0, 0, 1]),
                }];
            }
            spec.restart = Restart::UnlessStopped;
            spec.cap_add = Vec::<Capability>::new();
            spec.tenant = Some(tenant.id);
            spec.role = Some(template.role.as_str().to_owned());
            spec.validate()?;
            services.push(spec);
        }
    }
    Ok(Stack {
        network,
        volumes: vec![files, conf, ini],
        services,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use builtpanel_docker_manager::{MANAGED_LABEL, ROLE_LABEL, TENANT_LABEL};
    use builtpanel_domain::UidRange;

    use super::*;

    fn allowlist() -> Allowlist {
        Allowlist::embedded().unwrap()
    }

    fn uid(n: u32) -> Uid {
        UidRange::DEFAULT.allocate(10_000..10_000 + n).unwrap()
    }

    fn render_for(name: &str, n: u32, resources: PlanResources) -> Result<Stack, StackError> {
        let username = TenantUsername::parse(name).unwrap();
        render(
            &Tenant {
                id: Uuid::now_v7(),
                username: &username,
                uid: uid(n),
                resources,
                php_version: None,
                extra_php: &[],
            },
            &allowlist(),
        )
    }

    const BASIC: PlanResources = PlanResources {
        cpu_millis: Some(500),
        memory_bytes: Some(512 * MIB),
        pids: Some(128),
    };

    #[test]
    fn plan_limits_are_exactly_what_a_new_stack_gets() {
        for resources in [
            BASIC,
            PlanResources {
                cpu_millis: None,
                memory_bytes: None,
                pids: None,
            },
        ] {
            let stack = render_for("alice", 1, resources).unwrap();
            let username = TenantUsername::parse("alice").unwrap();
            let limits = plan_limits(&username, &resources, &[]).unwrap();
            assert_eq!(limits.len(), stack.services.len());
            for ((name, limit), spec) in limits.iter().zip(&stack.services) {
                assert_eq!(name, &spec.name, "same order: PHP first");
                assert_eq!(*limit, spec.limits());
            }
        }
        let tiny = PlanResources {
            memory_bytes: Some(8 * MIB),
            ..BASIC
        };
        let username = TenantUsername::parse("alice").unwrap();
        assert!(matches!(
            plan_limits(&username, &tiny, &[]),
            Err(StackError::PlanTooSmall("memory"))
        ));
    }

    #[test]
    fn the_shipped_allowlist_is_valid_and_pinned() {
        let list = allowlist();
        for role in [Role::Web, Role::Php] {
            let image = list.current(role).unwrap();
            assert!(image.as_str().contains("@sha256:"), "{image:?}");
            assert!(list.allows(role, image));
        }
        assert!(
            !list.allows(Role::Web, list.current(Role::Php).unwrap()),
            "roles are separate"
        );
    }

    #[test]
    fn allowlists_refuse_tags_duplicates_unknown_keys_and_missing_roles() {
        let digest = "a".repeat(64);
        let good = |role: &str, reference: &str| {
            format!("[[image]]\nrole = \"{role}\"\nreference = \"{reference}\"\n")
        };
        let web = good("web", &format!("nginx@sha256:{digest}"));
        let php = format!(
            "[[image]]\nrole = \"php\"\nversion = \"8.4\"\nreference = \"php@sha256:{digest}\"\n"
        );
        assert!(Allowlist::parse(&format!("{web}{php}")).is_ok());
        for (text, needle) in [
            (format!("{}{php}", good("web", "nginx:latest")), "pinned"),
            (format!("{web}{web}{php}"), "twice"),
            (web.clone(), "no image for the `php`"),
            (
                format!(
                    "{web}{php}[[image]]\nrole = \"bogus\"\nreference = \"x@sha256:{digest}\"\n"
                ),
                "bogus",
            ),
            (
                format!(
                    "{web}{php}[[image]]\nrole = \"web\"\nreference = \"x@sha256:{digest}\"\nroot = true\n"
                ),
                "root",
            ),
            ("not toml [".to_owned(), "allow-list"),
            (String::new(), "image"),
        ] {
            let err = Allowlist::parse(&text).unwrap_err().to_string();
            assert!(err.contains(needle), "{needle}: {err}");
        }
    }

    /// The security checklist of the container template: everything the review
    /// of T04.06 looks for, as assertions over what is rendered.
    #[test]
    fn the_security_checklist_holds_for_every_plan_and_tenant() {
        let list = allowlist();
        let plans = [
            BASIC,
            PlanResources {
                cpu_millis: None,
                memory_bytes: None,
                pids: None,
            },
            PlanResources {
                cpu_millis: Some(10_000),
                memory_bytes: Some(16 * 1024 * MIB),
                pids: Some(4096),
            },
            PlanResources {
                cpu_millis: Some(100),
                memory_bytes: Some(192 * MIB),
                pids: Some(64),
            },
        ];
        for (n, resources) in plans.into_iter().enumerate() {
            let username = format!("tenant{n}");
            let stack = render_for(&username, n as u32, resources).unwrap();
            let owner = TenantUsername::parse(&username).unwrap();
            let prefix = format!("bp-{owner}-");
            // The tenant's own network and volume, named after the tenant.
            assert_eq!(stack.network.as_str(), format!("{prefix}net"));
            assert_eq!(stack.volumes.len(), 3);
            assert!(stack.volumes[0].as_str().starts_with(&prefix));
            assert_eq!(stack.services.len(), 2);
            let (mut memory, mut pids, mut cpu) = (0_u64, 0_u64, 0_u64);
            for spec in &stack.services {
                // 1. Never root: the tenant's own id, and that id is at least 10000.
                let (u, g) = spec.user.expect("a user is always set");
                assert_eq!(u, g);
                assert_eq!(u, 10_000 + n as u32);
                assert!(u >= 10_000);
                // 2. Only allowed images, pinned.
                let role = if spec.role.as_deref() == Some("web") {
                    Role::Web
                } else {
                    Role::Php
                };
                assert!(list.allows(role, &spec.image), "{:?}", spec.image);
                // 3. Nothing added back, nothing published, nothing from the host.
                assert!(spec.cap_add.is_empty(), "no capabilities");
                // The web container publishes one port, on the loopback address only (the
                // node's web server comes in that way); nothing else publishes anything.
                if role == Role::Web {
                    assert_eq!(spec.ports.len(), 1);
                    assert!(spec.ports[0].host_ip.is_loopback(), "{:?}", spec.ports);
                } else {
                    assert!(spec.ports.is_empty(), "no published ports");
                }
                assert!(!spec.writable_root, "read-only root");
                // No secrets in the environment: the one variable there is a fixed path.
                assert!(
                    spec.env
                        .iter()
                        .all(|(k, v)| k == "PHP_INI_SCAN_DIR" && v.starts_with(":/usr/")),
                    "{:?}",
                    spec.env
                );
                // 4. Names, network and volumes are the tenant's own.
                assert!(
                    spec.name.as_str().starts_with(&prefix),
                    "{}",
                    spec.name.as_str()
                );
                assert_eq!(spec.network, Network::Named(stack.network.clone()));
                assert!(
                    spec.volumes
                        .iter()
                        .all(|v| v.volume.as_str().starts_with(&prefix))
                );
                assert!(spec.tmpfs.iter().all(|t| t.size_bytes <= 64 * MIB));
                // 5. Limits are set, and recorded as labels with the tenant.
                assert!(
                    spec.memory_bytes >= 32 * MIB && spec.pids_limit >= 16 && spec.cpu_millis >= 10
                );
                assert_eq!(spec.restart, Restart::UnlessStopped);
                memory += spec.memory_bytes;
                pids += u64::from(spec.pids_limit);
                cpu += u64::from(spec.cpu_millis);
                // 6. What docker-manager sends is the hardened request.
                let body = spec.to_create_body().unwrap();
                let host = body.host_config.unwrap();
                assert_eq!(host.cap_drop, Some(vec!["ALL".to_owned()]));
                assert_eq!(
                    host.security_opt,
                    Some(vec!["no-new-privileges:true".to_owned()])
                );
                assert_eq!(
                    (host.privileged, host.readonly_rootfs),
                    (Some(false), Some(true))
                );
                assert!(host.binds.is_none() && host.pid_mode.is_none() && host.devices.is_none());
                assert_eq!(body.user.as_deref(), Some(&*format!("{u}:{u}")));
                let labels = body.labels.unwrap();
                assert_eq!(labels[MANAGED_LABEL], "true");
                assert_eq!(labels[TENANT_LABEL], spec.tenant.unwrap().to_string());
                assert_eq!(labels[ROLE_LABEL], spec.role.clone().unwrap());
            }
            // The two containers together stay within what the plan gives.
            let cap = |value: Option<u64>, unlimited: u64| value.unwrap_or(unlimited);
            assert!(memory <= cap(resources.memory_bytes, UNLIMITED_MEMORY_BYTES));
            assert!(pids <= cap(resources.pids.map(u64::from), u64::from(UNLIMITED_PIDS)));
            assert!(
                cpu <= cap(
                    resources.cpu_millis.map(u64::from),
                    u64::from(UNLIMITED_CPU_MILLIS)
                )
            );
        }
    }

    #[test]
    fn php_starts_before_the_web_server_and_only_php_can_write_the_files() {
        let stack = render_for("alice", 0, BASIC).unwrap();
        let roles: Vec<&str> = stack
            .services
            .iter()
            .map(|s| s.role.as_deref().unwrap())
            .collect();
        assert_eq!(roles, ["php", "web"]);
        assert!(!stack.services[0].volumes[0].read_only);
        assert!(
            stack.services[1].volumes[0].read_only,
            "the web server only reads the site"
        );
        // The web server's configuration comes from a volume of its own, read-only;
        // PHP's options come from another, read-only too, and a variable says
        // where to look for them.
        assert_eq!(stack.services[0].volumes.len(), 2);
        let ini = &stack.services[0].volumes[1];
        assert_eq!(
            (ini.volume.as_str(), ini.target.as_str(), ini.read_only),
            ("bp-alice-phpini", "/usr/local/etc/php/builtpanel.d", true)
        );
        assert_eq!(
            stack.services[0]
                .env
                .get("PHP_INI_SCAN_DIR")
                .map(String::as_str),
            Some(":/usr/local/etc/php/builtpanel.d")
        );
        assert!(stack.services[1].env.is_empty());
        let conf = &stack.services[1].volumes[1];
        assert_eq!(
            (conf.volume.as_str(), conf.target.as_str(), conf.read_only),
            ("bp-alice-conf", "/etc/nginx/conf.d", true)
        );
        // Resources follow the 75 / 25 split.
        assert_eq!(stack.services[0].memory_bytes, 384 * MIB);
        assert_eq!(stack.services[1].memory_bytes, 128 * MIB);
        assert_eq!(
            (stack.services[0].pids_limit, stack.services[1].pids_limit),
            (96, 32)
        );
        assert_eq!(
            (stack.services[0].cpu_millis, stack.services[1].cpu_millis),
            (375, 125)
        );
    }

    #[test]
    fn a_plan_too_small_to_run_the_stack_is_refused() {
        let tiny_memory = PlanResources {
            memory_bytes: Some(64 * MIB),
            ..BASIC
        };
        assert_eq!(
            render_for("alice", 0, tiny_memory).unwrap_err(),
            StackError::PlanTooSmall("memory")
        );
        let tiny_pids = PlanResources {
            pids: Some(20),
            ..BASIC
        };
        assert_eq!(
            render_for("alice", 0, tiny_pids).unwrap_err(),
            StackError::PlanTooSmall("process limit")
        );
        let tiny_cpu = PlanResources {
            cpu_millis: Some(20),
            ..BASIC
        };
        assert_eq!(
            render_for("alice", 0, tiny_cpu).unwrap_err(),
            StackError::PlanTooSmall("CPU")
        );
    }

    #[test]
    fn two_tenants_share_nothing() {
        let a = render_for("alice", 0, BASIC).unwrap();
        let b = render_for("bobby", 1, BASIC).unwrap();
        assert_ne!(a.network, b.network);
        assert_ne!(a.volumes, b.volumes);
        let names = |s: &Stack| {
            s.services
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
        };
        assert!(names(&a).iter().all(|n| !names(&b).contains(n)));
        assert_ne!(a.services[0].user, b.services[0].user);
    }

    #[test]
    fn php_versions_come_from_the_list_and_extra_ones_get_their_own_container() {
        let list = allowlist();
        assert_eq!(list.php_versions(), ["8.4", "8.5", "8.3", "8.2"]);
        assert_eq!(list.default_php_version().unwrap(), "8.4");
        assert_eq!(
            list.php_image("8.4").unwrap(),
            list.current(Role::Php).unwrap(),
            "the default version is the current PHP image"
        );
        for version in list.php_versions() {
            assert!(
                list.php_image(version)
                    .unwrap()
                    .as_str()
                    .contains("@sha256:")
            );
        }
        for bad in ["8.9", "7.4", "8", "8.4.1", "", "x.y", "8.4; rm"] {
            assert!(
                matches!(list.php_image(bad), Err(StackError::UnknownPhpVersion(_))),
                "{bad}"
            );
        }

        let username = TenantUsername::parse("alice").unwrap();
        let stack = render(
            &Tenant {
                id: Uuid::now_v7(),
                username: &username,
                uid: uid(0),
                resources: BASIC,
                php_version: Some("8.5"),
                extra_php: &["8.3", "8.2"],
            },
            &list,
        )
        .unwrap();
        let names: Vec<&str> = stack.services.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "bp-alice-php",
                "bp-alice-php-8-3",
                "bp-alice-php-8-2",
                "bp-alice-web"
            ]
        );
        assert_eq!(stack.services[0].image, *list.php_image("8.5").unwrap());
        assert_eq!(stack.services[1].image, *list.php_image("8.3").unwrap());
        assert_eq!(stack.services[2].image, *list.php_image("8.2").unwrap());
        // The PHP share (75 % of 512 MiB) is divided between the three PHP containers.
        for php in &stack.services[..3] {
            assert_eq!(php.memory_bytes, 128 * MIB);
            assert_eq!(php.role.as_deref(), Some("php"));
        }
        assert_eq!(stack.services[3].memory_bytes, 128 * MIB);
        // What plan_limits says is what was rendered.
        let limits = plan_limits(&username, &BASIC, &["8.3", "8.2"]).unwrap();
        for ((name, limit), spec) in limits.iter().zip(&stack.services) {
            assert_eq!((name, *limit), (&spec.name, spec.limits()));
        }
        // The same version twice, an unknown one, or the account's own again are refused.
        for extra in [&["8.3", "8.3"][..], &["8.9"], &["8.5"]] {
            assert!(
                render(
                    &Tenant {
                        id: Uuid::now_v7(),
                        username: &username,
                        uid: uid(0),
                        resources: BASIC,
                        php_version: Some("8.5"),
                        extra_php: extra,
                    },
                    &list,
                )
                .is_err(),
                "{extra:?}"
            );
        }
        // A site on another version is sent to that version's container.
        assert_eq!(
            php_upstream_for(&username, Some("8.3"), "8.5").unwrap(),
            "bp-alice-php-8-3:9000"
        );
        for same in [Some("8.5"), None] {
            assert_eq!(
                php_upstream_for(&username, same, "8.5").unwrap(),
                "bp-alice-php:9000"
            );
        }
        assert!(php_extra_name(&username, "8.3; x").is_err());
    }

    #[test]
    fn our_own_php_image_comes_first_and_the_official_one_is_its_fallback() {
        let digest = "a".repeat(64);
        let (own_d, official_d) = ("b".repeat(64), "c".repeat(64));
        let web = format!("[[image]]\nrole = \"web\"\nreference = \"nginx@sha256:{digest}\"\n");
        let php = format!(
            "[[image]]\nrole = \"php\"\nversion = \"8.4\"\npreferred = \"ghcr.io/o/builtpanel-php@sha256:{own_d}\"\nreference = \"php@sha256:{official_d}\"\n"
        );
        let list = Allowlist::parse(&format!("{web}{php}")).unwrap();
        let own = list.php_image("8.4").unwrap().clone();
        let official = list.fallback_for(&own).unwrap().clone();
        assert!(own.as_str().starts_with("ghcr.io/o/builtpanel-php@"));
        assert!(official.as_str().starts_with("php@"));
        assert_eq!(list.current(Role::Php).unwrap(), &own);
        // Both may run (a container made from the fallback is not replaced).
        assert!(list.allows(Role::Php, &own) && list.allows(Role::Php, &official));
        assert_eq!(list.fallback_for(&official), None);
        // A tag instead of a digest is refused.
        let err = Allowlist::parse(&format!(
            "{web}{}",
            php.replace("ghcr.io/o/builtpanel-php@", "ghcr.io/o/builtpanel-php:")
        ))
        .unwrap_err()
        .to_string();
        assert!(err.contains("preferred"), "{err}");
    }

    #[test]
    fn every_role_may_have_an_image_of_ours_with_the_official_one_as_its_fallback() {
        let (digest, own_d) = ("a".repeat(64), "b".repeat(64));
        let php = format!(
            "[[image]]\nrole = \"php\"\nversion = \"8.4\"\nreference = \"php@sha256:{digest}\"\n"
        );
        let web = format!(
            "[[image]]\nrole = \"web\"\npreferred = \"ghcr.io/o/builtpanel-web@sha256:{own_d}\"\nreference = \"nginx@sha256:{digest}\"\n"
        );
        let db = format!(
            "[[image]]\nrole = \"db\"\npreferred = \"ghcr.io/o/builtpanel-db@sha256:{own_d}\"\nreference = \"mariadb@sha256:{digest}\"\n"
        );
        let list = Allowlist::parse(&format!("{web}{db}{php}")).unwrap();
        for (role, own_name, official_name) in [
            (Role::Web, "ghcr.io/o/builtpanel-web@", "nginx@"),
            (Role::Db, "ghcr.io/o/builtpanel-db@", "mariadb@"),
        ] {
            // New stacks use ours; the official one is its fallback; both may run.
            let own = list.current(role).unwrap().clone();
            assert!(own.as_str().starts_with(own_name), "{role}");
            let official = list.fallback_for(&own).unwrap().clone();
            assert!(official.as_str().starts_with(official_name), "{role}");
            assert!(list.allows(role, &own) && list.allows(role, &official));
            assert_eq!(list.fallback_for(&official), None);
        }
        // The same image twice is still refused.
        let twice = format!(
            "[[image]]\nrole = \"web\"\npreferred = \"nginx@sha256:{digest}\"\nreference = \"nginx@sha256:{digest}\"\n{php}"
        );
        assert!(
            Allowlist::parse(&twice)
                .unwrap_err()
                .to_string()
                .contains("twice")
        );
    }

    #[test]
    fn a_php_image_needs_its_version_and_only_php_has_one() {
        let digest = "a".repeat(64);
        let web = format!("[[image]]\nrole = \"web\"\nreference = \"nginx@sha256:{digest}\"\n");
        let php = |extra: &str, d: &str| {
            format!("[[image]]\nrole = \"php\"\n{extra}reference = \"php@sha256:{d}\"\n")
        };
        let other = "b".repeat(64);
        assert!(Allowlist::parse(&format!("{web}{}", php("version = \"8.4\"\n", &digest))).is_ok());
        for (text, needle) in [
            (format!("{web}{}", php("", &digest)), "needs its version"),
            (
                format!("{web}{}", php("version = \"eight\"\n", &digest)),
                "needs its version",
            ),
            (
                format!(
                    "{web}{}{}",
                    php("version = \"8.4\"\n", &digest),
                    php("version = \"8.4\"\n", &other)
                ),
                "twice",
            ),
            (
                format!(
                    "{}[[image]]\nrole = \"web\"\nversion = \"1.2\"\nreference = \"nginx@sha256:{digest}\"\n",
                    php("version = \"8.4\"\n", &digest),
                ),
                "only php",
            ),
        ] {
            let err = Allowlist::parse(&text).unwrap_err().to_string();
            assert!(err.contains(needle), "{needle}: {err}");
        }
    }
}
