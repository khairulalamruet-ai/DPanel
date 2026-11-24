//! What a container may look like. Everything is validated here, before
//! anything is sent to Docker, and the hardening is not optional.

use std::{collections::BTreeMap, net::IpAddr};

use bollard::models::{
    ContainerCreateBody, HostConfig, HostConfigLogConfig, Mount, MountType, MountVolumeOptions,
    PortBinding, RestartPolicy as DockerRestartPolicy, RestartPolicyNameEnum,
};

/// What a container may use. Applied at creation and changed later in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Memory in bytes (swap is set to the same, so there is none).
    pub memory_bytes: u64,
    /// CPU in thousandths of a core.
    pub cpu_millis: u32,
    /// Processes.
    pub pids_limit: u32,
}

impl Limits {
    /// Checks the three numbers.
    ///
    /// # Errors
    /// The first one out of range.
    pub fn validate(&self) -> Result<(), SpecError> {
        if !(MIN_MEMORY_BYTES..=MAX_MEMORY_BYTES).contains(&self.memory_bytes) {
            return Err(invalid("memory_bytes", "must be between 6 MiB and 1 TiB"));
        }
        if !(10..=256_000).contains(&self.cpu_millis) {
            return Err(invalid("cpu_millis", "must be between 10 and 256000"));
        }
        if !(1..=MAX_PIDS).contains(&self.pids_limit) {
            return Err(invalid("pids_limit", "must be between 1 and 32768"));
        }
        Ok(())
    }
}

/// Label every object this crate creates carries. Actions refuse objects
/// without it, so the agent can never touch a container it did not make.
pub const MANAGED_LABEL: &str = "builtpanel.managed";
/// Label naming the tenant a container belongs to.
pub const TENANT_LABEL: &str = "builtpanel.tenant";
/// Label naming the part a container plays in its stack.
pub const ROLE_LABEL: &str = "builtpanel.role";
/// Prefix reserved for our own labels.
pub const LABEL_PREFIX: &str = "builtpanel.";

/// Smallest memory limit Docker accepts (6 MiB).
pub const MIN_MEMORY_BYTES: u64 = 6 * 1024 * 1024;
/// Largest memory limit accepted (1 TiB).
pub const MAX_MEMORY_BYTES: u64 = 1 << 40;
/// Most processes a container may be allowed.
pub const MAX_PIDS: u32 = 32_768;

/// Why a specification was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {field}: {reason}")]
pub struct SpecError {
    /// Which part.
    pub field: &'static str,
    /// What is wrong with it.
    pub reason: String,
}

fn invalid(field: &'static str, reason: impl Into<String>) -> SpecError {
    SpecError {
        field,
        reason: reason.into(),
    }
}

/// An image pinned by digest: `registry/path@sha256:<64 hex>`. Tags are not
/// accepted, because a tag can be moved to different content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef(String);

impl ImageRef {
    /// Parses and validates a reference.
    ///
    /// # Errors
    /// Not of the form `name@sha256:<64 lowercase hex>`.
    pub fn parse(reference: &str) -> Result<Self, SpecError> {
        let (name, digest) = reference
            .split_once("@sha256:")
            .ok_or_else(|| invalid("image", "must be pinned: name@sha256:<digest>"))?;
        let name_ok = !name.is_empty()
            && name.len() <= 200
            && !name.starts_with(['/', '.', '-', ':'])
            && !name.contains("..")
            && !name.contains("//")
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "./_-:".contains(c));
        let digest_ok = digest.len() == 64
            && digest
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
        if !name_ok {
            return Err(invalid("image", "bad repository name"));
        }
        if !digest_ok {
            return Err(invalid("image", "digest must be 64 lowercase hex digits"));
        }
        Ok(Self(reference.to_owned()))
    }

    /// The reference text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Name of a container, volume or network we manage: `bp-` followed by up to
/// 60 characters of `a-z 0-9 _ . -`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Name(String);

impl Name {
    /// Parses and validates a name.
    ///
    /// # Errors
    /// Not of the form `bp-[a-z0-9][a-z0-9_.-]*`, or too long.
    pub fn parse(name: &str) -> Result<Self, SpecError> {
        let rest = name
            .strip_prefix("bp-")
            .ok_or_else(|| invalid("name", "must start with `bp-`"))?;
        let ok = (1..=60).contains(&rest.len())
            && rest.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            && rest
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c));
        if ok {
            Ok(Self(name.to_owned()))
        } else {
            Err(invalid(
                "name",
                "use bp- then a-z, 0-9, `_`, `.`, `-` (at most 60)",
            ))
        }
    }

    /// The name text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Linux capabilities a container may be given back. Everything else stays
/// dropped; there is no way to ask for `SYS_ADMIN`, `NET_ADMIN` and the like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    /// Change file ownership.
    Chown,
    /// Bypass file permission checks (as the container's root).
    DacOverride,
    /// Bypass ownership checks on files.
    Fowner,
    /// Change group id (drop privileges at start).
    Setgid,
    /// Change user id (drop privileges at start).
    Setuid,
    /// Bind ports below 1024 inside the container.
    NetBindService,
    /// Signal processes of other users inside the container.
    Kill,
}

impl Capability {
    fn docker_name(self) -> &'static str {
        match self {
            Self::Chown => "CHOWN",
            Self::DacOverride => "DAC_OVERRIDE",
            Self::Fowner => "FOWNER",
            Self::Setgid => "SETGID",
            Self::Setuid => "SETUID",
            Self::NetBindService => "NET_BIND_SERVICE",
            Self::Kill => "KILL",
        }
    }
}

/// What Docker does when the container's main process ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    /// Nothing.
    No,
    /// Start it again, unless it was stopped on purpose.
    UnlessStopped,
    /// Start it again after a failure, at most this many times.
    OnFailure(u8),
}

/// Which network the container joins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Network {
    /// No network at all.
    None,
    /// A network created with [`crate::DockerManager::create_network`].
    Named(Name),
}

/// A named volume mounted into the container. Host paths cannot be mounted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeMount {
    /// The volume.
    pub volume: Name,
    /// Absolute path inside the container.
    pub target: String,
    /// Mount read-only.
    pub read_only: bool,
}

/// A memory-backed scratch directory (noexec, nosuid, nodev).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tmpfs {
    /// Absolute path inside the container.
    pub target: String,
    /// Size limit in bytes (64 KiB ..= 1 GiB).
    pub size_bytes: u64,
}

/// A port published on the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishedPort {
    /// TCP port inside the container.
    pub container_port: u16,
    /// Port on the host; 1024 or above (the daemon is rootless, ADR 0004). 0 lets Docker pick a
    /// free one (only on the loopback address): read it back with `ContainerInfo::published`.
    pub host_port: u16,
    /// Address on the host to listen on. Loopback unless said otherwise.
    pub host_ip: IpAddr,
}

/// A container to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSpec {
    /// Its name.
    pub name: Name,
    /// The image, pinned by digest.
    pub image: ImageRef,
    /// Arguments for the image's entrypoint (empty: the image's default).
    pub command: Vec<String>,
    /// Replaces the image's entrypoint (empty: keep it).
    pub entrypoint: Vec<String>,
    /// The directory the process starts in (an absolute path; `None`: the image's).
    pub working_dir: Option<String>,
    /// Environment variables. Never secrets: those are mounted, not passed here.
    pub env: BTreeMap<String, String>,
    /// Numeric `uid:gid` to run as (default: what the image says).
    pub user: Option<(u32, u32)>,
    /// Memory limit in bytes (also the swap limit: no extra swap).
    pub memory_bytes: u64,
    /// CPU limit in thousandths of a core (500 = half a core).
    pub cpu_millis: u32,
    /// Most processes.
    pub pids_limit: u32,
    /// Keep the root filesystem writable (default is read-only).
    pub writable_root: bool,
    /// Memory-backed scratch directories.
    pub tmpfs: Vec<Tmpfs>,
    /// Named volumes.
    pub volumes: Vec<VolumeMount>,
    /// Published ports.
    pub ports: Vec<PublishedPort>,
    /// Network.
    pub network: Network,
    /// Capabilities given back after everything is dropped.
    pub cap_add: Vec<Capability>,
    /// Restart behaviour.
    pub restart: Restart,
    /// Extra labels (not in the reserved `builtpanel.` space).
    pub labels: BTreeMap<String, String>,
    /// The tenant the container belongs to, recorded as the reserved label
    /// `builtpanel.tenant` (a UUID, so it needs no further checking).
    pub tenant: Option<uuid::Uuid>,
    /// The part the container plays in its stack (`web`, `php`, ...), recorded
    /// as `builtpanel.role`: 1-32 characters of `a-z`, `0-9`, `_`.
    pub role: Option<String>,
}

impl ContainerSpec {
    /// A specification with conservative limits: 256 MiB, half a core, 128
    /// processes, read-only root, no network, no capabilities, no restart.
    #[must_use]
    pub fn new(name: Name, image: ImageRef) -> Self {
        Self {
            name,
            image,
            command: Vec::new(),
            entrypoint: Vec::new(),
            working_dir: None,
            env: BTreeMap::new(),
            user: None,
            memory_bytes: 256 * 1024 * 1024,
            cpu_millis: 500,
            pids_limit: 128,
            writable_root: false,
            tmpfs: Vec::new(),
            volumes: Vec::new(),
            ports: Vec::new(),
            network: Network::None,
            cap_add: Vec::new(),
            restart: Restart::No,
            labels: BTreeMap::new(),
            tenant: None,
            role: None,
        }
    }

    /// The container's limits as a [`Limits`] value.
    #[must_use]
    pub fn limits(&self) -> Limits {
        Limits {
            memory_bytes: self.memory_bytes,
            cpu_millis: self.cpu_millis,
            pids_limit: self.pids_limit,
        }
    }

    /// Checks every field.
    ///
    /// # Errors
    /// The first problem found.
    pub fn validate(&self) -> Result<(), SpecError> {
        self.limits().validate()?;
        if self.command.len() > 64
            || self.entrypoint.len() > 64
            || self
                .command
                .iter()
                .chain(&self.entrypoint)
                .any(|a| a.len() > 4096 || a.contains('\0'))
        {
            return Err(invalid(
                "command",
                "at most 64 arguments of 4096 bytes, no NUL",
            ));
        }
        if let Some(dir) = &self.working_dir
            && !(dir.starts_with('/')
                && dir.len() <= 255
                && !dir.split('/').any(|part| part == ".." || part == ".")
                && dir
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c)))
        {
            return Err(invalid("working_dir", "a plain absolute path"));
        }
        if self.env.len() > 128 {
            return Err(invalid("env", "at most 128 variables"));
        }
        for (key, value) in &self.env {
            let key_ok = !key.is_empty()
                && key.len() <= 128
                && key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !key_ok {
                return Err(invalid(
                    "env",
                    format!("bad variable name `{}`", key.escape_default()),
                ));
            }
            if value.len() > 8192 || value.contains('\0') {
                return Err(invalid(
                    "env",
                    format!("value of `{key}` is too long or has NUL"),
                ));
            }
        }
        let mut targets = std::collections::BTreeSet::new();
        for target in self
            .volumes
            .iter()
            .map(|v| v.target.as_str())
            .chain(self.tmpfs.iter().map(|t| t.target.as_str()))
        {
            check_target(target)?;
            if !targets.insert(target.trim_end_matches('/')) {
                return Err(invalid("mounts", format!("`{target}` is mounted twice")));
            }
        }
        if self.volumes.len() > 16 || self.tmpfs.len() > 8 {
            return Err(invalid("mounts", "at most 16 volumes and 8 tmpfs mounts"));
        }
        for t in &self.tmpfs {
            if !(64 * 1024..=1 << 30).contains(&t.size_bytes) {
                return Err(invalid("tmpfs", "size must be between 64 KiB and 1 GiB"));
            }
        }
        if self.ports.len() > 32 {
            return Err(invalid("ports", "at most 32 published ports"));
        }
        let mut host_ports = std::collections::BTreeSet::new();
        for p in &self.ports {
            if p.container_port == 0 {
                return Err(invalid("ports", "container port 0"));
            }
            if p.host_port == 0 {
                // "Any free port" is safe only where nobody outside can reach it.
                if !p.host_ip.is_loopback() {
                    return Err(invalid(
                        "ports",
                        "a port chosen by Docker must be on the loopback address",
                    ));
                }
                continue;
            }
            if p.host_port < 1024 {
                return Err(invalid("ports", "host ports below 1024 are not available"));
            }
            if !host_ports.insert((p.host_ip, p.host_port)) {
                return Err(invalid(
                    "ports",
                    format!("host port {} is used twice", p.host_port),
                ));
            }
        }
        if !self.ports.is_empty() && self.network == Network::None {
            return Err(invalid(
                "ports",
                "a container without a network cannot publish ports",
            ));
        }
        if let Restart::OnFailure(n) = self.restart
            && !(1..=10).contains(&n)
        {
            return Err(invalid("restart", "retry count must be between 1 and 10"));
        }
        if self.labels.len() > 32 {
            return Err(invalid("labels", "at most 32 labels"));
        }
        if let Some(role) = &self.role {
            let ok = (1..=32).contains(&role.len())
                && role.starts_with(|c: char| c.is_ascii_lowercase())
                && role
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !ok {
                return Err(invalid("role", "use 1-32 characters of a-z, 0-9 and `_`"));
            }
        }
        for (key, value) in &self.labels {
            let key_ok = !key.is_empty()
                && key.len() <= 128
                && !key.starts_with(LABEL_PREFIX)
                && key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c));
            if !key_ok {
                return Err(invalid(
                    "labels",
                    format!("bad label name `{}`", key.escape_default()),
                ));
            }
            if value.len() > 1024 || value.chars().any(char::is_control) {
                return Err(invalid("labels", format!("bad value for `{key}`")));
            }
        }
        Ok(())
    }

    /// Turns a valid specification into the Docker request body.
    ///
    /// Always, whatever the specification says: all capabilities dropped (then
    /// only the listed ones added), `no-new-privileges`, not privileged, own
    /// PID/IPC/UTS/user namespaces (never the host's), no devices, memory =
    /// memory+swap, a process limit, an init process, bounded logs, and the
    /// [`MANAGED_LABEL`].
    ///
    /// # Errors
    /// [`SpecError`] if the specification is not valid.
    pub fn to_create_body(&self) -> Result<ContainerCreateBody, SpecError> {
        self.validate()?;
        let memory =
            i64::try_from(self.memory_bytes).map_err(|_| invalid("memory_bytes", "too large"))?;
        let mut labels: std::collections::HashMap<String, String> =
            self.labels.clone().into_iter().collect();
        labels.insert(MANAGED_LABEL.to_owned(), "true".to_owned());
        if let Some(tenant) = self.tenant {
            labels.insert(TENANT_LABEL.to_owned(), tenant.to_string());
        }
        if let Some(role) = &self.role {
            labels.insert(ROLE_LABEL.to_owned(), role.clone());
        }

        let mut mounts: Vec<Mount> = self
            .volumes
            .iter()
            .map(|v| Mount {
                target: Some(v.target.clone()),
                source: Some(v.volume.as_str().to_owned()),
                typ: Some(MountType::VOLUME),
                read_only: Some(v.read_only),
                // Never copy the image's files, owners or modes into a tenant's
                // volume: the first mount of an empty volume would otherwise
                // replace the ownership set for the tenant with the image's.
                volume_options: Some(MountVolumeOptions {
                    no_copy: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        mounts.sort_by(|a, b| a.target.cmp(&b.target));
        let tmpfs = self
            .tmpfs
            .iter()
            .map(|t| {
                (
                    t.target.clone(),
                    format!("rw,noexec,nosuid,nodev,mode=1777,size={}", t.size_bytes),
                )
            })
            .collect::<std::collections::HashMap<_, _>>();

        let mut port_bindings = std::collections::HashMap::new();
        let mut exposed = Vec::new();
        for p in &self.ports {
            let key = format!("{}/tcp", p.container_port);
            exposed.push(key.clone());
            port_bindings
                .entry(key)
                .or_insert_with(|| Some(Vec::new()))
                .get_or_insert_with(Vec::new)
                .push(PortBinding {
                    host_ip: Some(p.host_ip.to_string()),
                    // An empty host port is "any free one".
                    host_port: Some(if p.host_port == 0 {
                        String::new()
                    } else {
                        p.host_port.to_string()
                    }),
                });
        }
        exposed.sort();
        exposed.dedup();

        let mut cap_add: Vec<Capability> = self.cap_add.clone();
        cap_add.sort();
        cap_add.dedup();

        let host_config = HostConfig {
            memory: Some(memory),
            memory_swap: Some(memory),
            nano_cpus: Some(i64::from(self.cpu_millis) * 1_000_000),
            pids_limit: Some(i64::from(self.pids_limit)),
            readonly_rootfs: Some(!self.writable_root),
            cap_drop: Some(vec!["ALL".to_owned()]),
            cap_add: Some(cap_add.iter().map(|c| c.docker_name().to_owned()).collect()),
            security_opt: Some(vec!["no-new-privileges:true".to_owned()]),
            privileged: Some(false),
            init: Some(true),
            mounts: Some(mounts),
            tmpfs: Some(tmpfs),
            port_bindings: Some(port_bindings),
            publish_all_ports: Some(false),
            network_mode: Some(match &self.network {
                Network::None => "none".to_owned(),
                Network::Named(name) => name.as_str().to_owned(),
            }),
            restart_policy: Some(match self.restart {
                Restart::No => DockerRestartPolicy {
                    name: Some(RestartPolicyNameEnum::NO),
                    maximum_retry_count: None,
                },
                Restart::UnlessStopped => DockerRestartPolicy {
                    name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                    maximum_retry_count: None,
                },
                Restart::OnFailure(n) => DockerRestartPolicy {
                    name: Some(RestartPolicyNameEnum::ON_FAILURE),
                    maximum_retry_count: Some(i64::from(n)),
                },
            }),
            log_config: Some(HostConfigLogConfig {
                typ: Some("json-file".to_owned()),
                config: Some(
                    [("max-size", "10m"), ("max-file", "3")]
                        .into_iter()
                        .map(|(k, v)| (k.to_owned(), v.to_owned()))
                        .collect(),
                ),
            }),
            ..Default::default()
        };
        Ok(ContainerCreateBody {
            image: Some(self.image.as_str().to_owned()),
            cmd: (!self.command.is_empty()).then(|| self.command.clone()),
            entrypoint: (!self.entrypoint.is_empty()).then(|| self.entrypoint.clone()),
            working_dir: self.working_dir.clone(),
            env: Some(self.env.iter().map(|(k, v)| format!("{k}={v}")).collect()),
            user: self.user.map(|(uid, gid)| format!("{uid}:{gid}")),
            labels: Some(labels.into_iter().collect()),
            exposed_ports: (!exposed.is_empty()).then_some(exposed),
            host_config: Some(host_config),
            tty: Some(false),
            open_stdin: Some(false),
            attach_stdin: Some(false),
            ..Default::default()
        })
    }
}

/// A mount point must be a plain absolute path that cannot shadow the
/// container's own kernel interfaces or its root.
fn check_target(target: &str) -> Result<(), SpecError> {
    let plain = target.starts_with('/')
        && target.len() <= 255
        && !target.split('/').any(|part| part == ".." || part == ".")
        && !target.contains("//")
        && target
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c));
    if !plain {
        return Err(invalid(
            "mounts",
            format!("`{}` is not a plain absolute path", target.escape_default()),
        ));
    }
    let trimmed = target.trim_end_matches('/');
    // The two places below `/etc` and `/usr` a volume may go: the web server's site
    // configuration and the PHP options directory, which is the point of a web
    // and a PHP container.
    // And the mail server's own configuration folder (T13.02), which is its volume.
    if trimmed == "/etc/nginx/conf.d"
        || trimmed == "/usr/local/etc/php/builtpanel.d"
        || trimmed == "/etc/stalwart"
    {
        return Ok(());
    }
    let forbidden = trimmed.is_empty()
        || [
            "/proc", "/sys", "/dev", "/etc", "/bin", "/sbin", "/usr", "/lib", "/lib64", "/boot",
        ]
        .iter()
        .any(|p| trimmed == *p || trimmed.starts_with(&format!("{p}/")));
    if forbidden {
        return Err(invalid(
            "mounts",
            format!("`{target}` may not be mounted over"),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    const DIGEST: &str = "a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec";

    fn image() -> ImageRef {
        ImageRef::parse(&format!("busybox@sha256:{DIGEST}")).unwrap()
    }

    fn spec() -> ContainerSpec {
        ContainerSpec::new(Name::parse("bp-test").unwrap(), image())
    }

    #[test]
    fn images_must_be_pinned_by_digest() {
        for ok in [
            format!("busybox@sha256:{DIGEST}"),
            format!("docker.io/library/nginx@sha256:{DIGEST}"),
            format!("registry.example.com:5000/team/app_1@sha256:{DIGEST}"),
        ] {
            assert!(ImageRef::parse(&ok).is_ok(), "{ok}");
        }
        for bad in [
            "busybox".to_owned(),
            "busybox:latest".to_owned(),
            "busybox:1.37".to_owned(),
            format!("busybox:1.37@sha256:{}", &DIGEST[..63]),
            format!("busybox@sha256:{}", DIGEST.to_uppercase()),
            format!("busybox@sha512:{DIGEST}"),
            format!("@sha256:{DIGEST}"),
            format!("../etc@sha256:{DIGEST}"),
            format!("Busybox@sha256:{DIGEST}"),
            format!("bus box@sha256:{DIGEST}"),
            format!("busybox@sha256:{DIGEST} --privileged"),
        ] {
            assert!(ImageRef::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn names_are_ours_and_plain() {
        for ok in ["bp-a", "bp-site_1.web-0", &format!("bp-{}", "a".repeat(60))] {
            assert!(Name::parse(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "bp-",
            "web",
            "bp-Web",
            "bp--x",
            "bp-a b",
            "bp-a/b",
            "bp-a;rm",
            &format!("bp-{}", "a".repeat(61)),
        ] {
            assert!(Name::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn every_container_is_hardened_whatever_the_specification_says() {
        let mut s = spec();
        s.writable_root = true;
        s.cap_add = vec![Capability::Setuid, Capability::Chown, Capability::Setuid];
        s.labels.insert("tenant".into(), "t1".into());
        let body = s.to_create_body().unwrap();
        let host = body.host_config.unwrap();
        assert_eq!(host.cap_drop, Some(vec!["ALL".to_owned()]));
        assert_eq!(
            host.cap_add,
            Some(vec!["CHOWN".to_owned(), "SETUID".to_owned()])
        );
        assert_eq!(
            host.security_opt,
            Some(vec!["no-new-privileges:true".to_owned()])
        );
        assert_eq!(host.privileged, Some(false));
        assert_eq!(host.init, Some(true));
        assert_eq!(
            host.memory, host.memory_swap,
            "no swap beyond the memory limit"
        );
        assert_eq!(host.memory, Some(256 * 1024 * 1024));
        assert_eq!(host.nano_cpus, Some(500_000_000));
        assert_eq!(host.pids_limit, Some(128));
        assert_eq!(host.network_mode.as_deref(), Some("none"));
        assert_eq!(host.readonly_rootfs, Some(false), "only when asked");
        // Nothing of the host is shared.
        assert!(host.pid_mode.is_none() && host.ipc_mode.is_none() && host.uts_mode.is_none());
        assert!(host.userns_mode.is_none() && host.devices.is_none() && host.binds.is_none());
        assert!(host.cgroupns_mode.is_none() && host.device_cgroup_rules.is_none());
        assert_eq!(host.log_config.unwrap().config.unwrap()["max-size"], "10m");
        let labels = body.labels.unwrap();
        assert_eq!(labels[MANAGED_LABEL], "true");
        assert_eq!(labels["tenant"], "t1");
        assert_eq!(body.image.as_deref(), Some(image().as_str()));
        assert_eq!((body.tty, body.open_stdin), (Some(false), Some(false)));
        // The default is a read-only root.
        assert_eq!(
            spec()
                .to_create_body()
                .unwrap()
                .host_config
                .unwrap()
                .readonly_rootfs,
            Some(true)
        );
    }

    #[test]
    fn mounts_are_named_volumes_and_tmpfs_only() {
        let mut s = spec();
        s.volumes.push(VolumeMount {
            volume: Name::parse("bp-data").unwrap(),
            target: "/var/www".into(),
            read_only: true,
        });
        s.tmpfs.push(Tmpfs {
            target: "/tmp".into(),
            size_bytes: 16 << 20,
        });
        let host = s.to_create_body().unwrap().host_config.unwrap();
        let mounts = host.mounts.unwrap();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].typ, Some(MountType::VOLUME));
        assert_eq!(mounts[0].source.as_deref(), Some("bp-data"));
        assert_eq!(mounts[0].read_only, Some(true));
        assert_eq!(
            mounts[0].volume_options.as_ref().and_then(|o| o.no_copy),
            Some(true),
            "an image must not rewrite the owner of a tenant's files"
        );
        assert_eq!(
            host.tmpfs.unwrap()["/tmp"],
            "rw,noexec,nosuid,nodev,mode=1777,size=16777216"
        );

        for bad in [
            "relative",
            "/",
            "/proc",
            "/proc/sys",
            "/sys/fs",
            "/dev/shm",
            "/etc",
            "/etc/nginx",
            "/etc/nginx/nginx.conf",
            "/etc/nginx/conf.d/x",
            "/usr/local",
            "/usr/local/etc/php",
            "/usr/local/etc/php/conf.d",
            "/a/../b",
            "/a//b",
            "/a b",
            "/a\nb",
            "/var/./x",
        ] {
            let mut s = spec();
            s.volumes.push(VolumeMount {
                volume: Name::parse("bp-data").unwrap(),
                target: bad.into(),
                read_only: false,
            });
            assert!(s.validate().is_err(), "{bad:?}");
        }
        // The web server's site configuration directory, and only that, is allowed.
        let mut s = spec();
        s.volumes.push(VolumeMount {
            volume: Name::parse("bp-data").unwrap(),
            target: "/etc/nginx/conf.d".into(),
            read_only: true,
        });
        assert!(s.validate().is_ok());
        let mut s = spec();
        s.volumes.push(VolumeMount {
            volume: Name::parse("bp-data").unwrap(),
            target: "/usr/local/etc/php/builtpanel.d".into(),
            read_only: true,
        });
        assert!(s.validate().is_ok());
        // The same place twice (also with a trailing slash).
        let mut s = spec();
        for target in ["/data", "/data/"] {
            s.tmpfs.push(Tmpfs {
                target: target.into(),
                size_bytes: 1 << 20,
            });
        }
        assert!(s.validate().is_err());
    }

    #[test]
    fn ports_need_a_network_and_stay_above_1023() {
        let port = |host_port| PublishedPort {
            container_port: 80,
            host_port,
            host_ip: "127.0.0.1".parse().unwrap(),
        };
        let mut s = spec();
        s.ports.push(port(8080));
        assert!(s.validate().is_err(), "no network");
        s.network = Network::Named(Name::parse("bp-net").unwrap());
        let body = s.to_create_body().unwrap();
        let bindings = body.host_config.unwrap().port_bindings.unwrap();
        let binding = &bindings["80/tcp"].as_ref().unwrap()[0];
        assert_eq!(binding.host_ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(binding.host_port.as_deref(), Some("8080"));
        assert_eq!(body.exposed_ports, Some(vec!["80/tcp".to_owned()]));

        s.ports = vec![port(80)];
        assert!(s.validate().is_err(), "privileged host port");
        s.ports = vec![port(8080), port(8080)];
        assert!(s.validate().is_err(), "same host port twice");
    }

    #[test]
    fn limits_environment_and_labels_are_checked() {
        let check = |edit: &dyn Fn(&mut ContainerSpec)| {
            let mut s = spec();
            edit(&mut s);
            s.validate()
        };
        assert!(check(&|_| {}).is_ok());
        assert!(check(&|s| s.memory_bytes = 1024).is_err());
        assert!(check(&|s| s.memory_bytes = u64::MAX).is_err());
        assert!(check(&|s| s.cpu_millis = 0).is_err());
        assert!(check(&|s| s.pids_limit = 0).is_err());
        assert!(check(&|s| s.pids_limit = 1_000_000).is_err());
        assert!(check(&|s| s.restart = Restart::OnFailure(0)).is_err());
        assert!(check(&|s| s.restart = Restart::OnFailure(3)).is_ok());
        assert!(
            check(&|s| {
                s.env.insert("PATH".into(), "/bin".into());
            })
            .is_ok()
        );
        assert!(
            check(&|s| {
                s.env.insert("BAD KEY".into(), "x".into());
            })
            .is_err()
        );
        assert!(
            check(&|s| {
                s.env.insert("A=B".into(), "x".into());
            })
            .is_err()
        );
        assert!(
            check(&|s| {
                s.env.insert("1A".into(), "x".into());
            })
            .is_err()
        );
        assert!(
            check(&|s| {
                s.env.insert("A".into(), "x\0y".into());
            })
            .is_err()
        );
        assert!(check(&|s| s.command = vec!["a\0b".into()]).is_err());
        assert!(
            check(&|s| {
                s.labels.insert("builtpanel.managed".into(), "false".into());
            })
            .is_err()
        );
        assert!(
            check(&|s| {
                s.labels.insert("Bad Label".into(), "x".into());
            })
            .is_err()
        );
        assert!(
            check(&|s| {
                s.labels.insert("ok.label".into(), "x\ny".into());
            })
            .is_err()
        );
    }
}
