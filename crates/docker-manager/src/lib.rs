//! The only way the node agent talks to Docker (T02.14).
//!
//! A deliberately small surface over the Docker Engine API:
//!
//! * pull an image **by digest**;
//! * create a container from a validated [`ContainerSpec`], always hardened
//!   (see [`ContainerSpec::to_create_body`]);
//! * start, stop, remove, inspect it and read its resource usage;
//! * create and remove named volumes and networks.
//!
//! There is **no free-form exec** (only the fixed nginx commands and, for the
//! database container, [`DockerManager::dump_database`], whose only inputs are
//! two checked identifiers and a password), no attach, no host bind mount, no privileged mode, no
//! host namespaces and no way to pass raw Docker options: such a request cannot
//! be expressed with these types. Every object created carries the
//! [`MANAGED_LABEL`], and every action first checks that label, so a container,
//! volume or network that this crate did not create is never touched.
//!
//! The daemon is the agent's own rootless Docker (ADR 0004), reached through
//! its Unix socket.

pub mod spec;

use std::{collections::HashMap, path::Path, time::Duration};

use bollard::{
    Docker,
    errors::Error as BollardError,
    exec::{CreateExecOptions, StartExecResults},
    models::{
        ContainerUpdateBody, EndpointSettings, NetworkConnectRequest, NetworkCreateRequest,
        VolumeCreateRequest,
    },
    query_parameters::{
        CreateContainerOptionsBuilder, CreateImageOptionsBuilder, ListContainersOptionsBuilder,
        RemoveContainerOptionsBuilder, StatsOptionsBuilder, StopContainerOptionsBuilder,
    },
};
use futures_util::StreamExt as _;

pub use crate::spec::{
    Capability, ContainerSpec, ImageRef, Limits, MANAGED_LABEL, Name, Network, PublishedPort,
    ROLE_LABEL, Restart, SpecError, TENANT_LABEL, Tmpfs, VolumeMount,
};

/// Longest a single API call may take (pulls have their own, longer limit).
const API_TIMEOUT_SECS: u64 = 30;
/// Longest an image pull may take.
pub const PULL_TIMEOUT: Duration = Duration::from_secs(600);

/// Longest a command run inside a container may take.
const EXEC_TIMEOUT: Duration = Duration::from_secs(20);
/// Most output kept from such a command.
const MAX_EXEC_OUTPUT: usize = 8 * 1024;

/// The commands the manager runs inside a web container. Each is a fixed list of
/// words; nothing a request says ever becomes part of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NginxCommand {
    /// Check the configuration (`nginx -t`).
    Test,
    /// Load the configuration without dropping connections (`nginx -s reload`).
    Reload,
}

impl NginxCommand {
    fn argv(self) -> &'static [&'static str] {
        match self {
            Self::Test => &["nginx", "-t"],
            Self::Reload => &["nginx", "-s", "reload"],
        }
    }
}

/// The signals the manager sends to a container: only the ones it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// PHP-FPM: load the configuration again, finishing the requests in progress.
    Usr2,
}

impl Signal {
    fn name(self) -> &'static str {
        match self {
            Self::Usr2 => "USR2",
        }
    }
}

/// How a command inside a container ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    /// Exit code (`-1` when Docker did not say).
    pub exit_code: i64,
    /// What it printed (stdout and stderr together), clipped.
    pub output: String,
}

/// Most bytes of log text one read returns.
const MAX_LOG_BYTES: usize = 1 << 20;

/// One line of a container's log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// When Docker saw it (Unix milliseconds).
    pub at_ms: i64,
    /// Written to standard error (otherwise standard output).
    pub stderr: bool,
    /// The line, without its time.
    pub text: String,
}

/// What a log read found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRead {
    /// Oldest first.
    pub lines: Vec<LogLine>,
    /// More was there than one read returns.
    pub truncated: bool,
}

fn usage_from(stats: &bollard::models::ContainerStatsResponse) -> ContainerUsage {
    let memory = stats.memory_stats.clone().unwrap_or_default();
    ContainerUsage {
        memory_bytes: memory.usage.unwrap_or(0),
        memory_limit_bytes: memory.limit.unwrap_or(0),
        cpu_total_nanos: stats
            .cpu_stats
            .as_ref()
            .and_then(|c| c.cpu_usage.as_ref())
            .and_then(|u| u.total_usage)
            .unwrap_or(0),
        pids: stats
            .pids_stats
            .as_ref()
            .and_then(|p| p.current)
            .unwrap_or(0),
        net_rx_bytes: stats
            .networks
            .as_ref()
            .map_or(0, |n| n.values().map(|i| i.rx_bytes.unwrap_or(0)).sum()),
        net_tx_bytes: stats
            .networks
            .as_ref()
            .map_or(0, |n| n.values().map(|i| i.tx_bytes.unwrap_or(0)).sum()),
        io_read_bytes: blkio_bytes(stats.blkio_stats.as_ref(), "read"),
        io_write_bytes: blkio_bytes(stats.blkio_stats.as_ref(), "write"),
    }
}

fn blkio_bytes(blkio: Option<&bollard::models::ContainerBlkioStats>, op: &str) -> u64 {
    blkio
        .and_then(|b| b.io_service_bytes_recursive.as_ref())
        .map_or(0, |entries| {
            entries
                .iter()
                .filter(|e| e.op.as_deref().is_some_and(|o| o.eq_ignore_ascii_case(op)))
                .map(|e| e.value.unwrap_or(0))
                .sum()
        })
}

/// Splits Docker's `2026-11-15T07:00:00.123456789Z text` into milliseconds and text.
fn split_timestamp(line: &str) -> (i64, &str) {
    let Some((stamp, text)) = line.split_once(' ') else {
        return (0, line);
    };
    match parse_stamp(stamp) {
        Some(at) => (at, text),
        None => (0, line),
    }
}

fn parse_stamp(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let rest = rest.strip_suffix('Z')?;
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    let (clock, fraction) = rest.split_once('.').unwrap_or((rest, "0"));
    let mut c = clock.split(':');
    let (h, mi, sec): (i64, i64, i64) = (
        c.next()?.parse().ok()?,
        c.next()?.parse().ok()?,
        c.next()?.parse().ok()?,
    );
    let millis: i64 = format!("{:0<3}", &fraction[..fraction.len().min(3)])
        .parse()
        .ok()?;
    // Days since 1970-01-01 (the civil-from-days algorithm, the other way round).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + mi) * 60_000 + sec * 1000 + millis)
}

/// What went wrong.
#[derive(Debug, thiserror::Error)]
pub enum DockerError {
    /// The request itself is not acceptable.
    #[error(transparent)]
    Invalid(#[from] SpecError),
    /// No such container, image, volume or network.
    #[error("not found")]
    NotFound,
    /// The object exists but was not created by this crate.
    #[error("not managed by Built Panel")]
    NotManaged,
    /// Something with that name exists already, or the object is in use.
    #[error("conflict: {0}")]
    Conflict(String),
    /// The Docker daemon cannot be reached or did not answer in time.
    #[error("docker is unavailable: {0}")]
    Unavailable(String),
    /// Docker refused or failed.
    #[error("docker error: {0}")]
    Api(String),
}

impl DockerError {
    /// Whether trying again later may work.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

/// An image as it was asked for: Podman writes the registry in front (`docker.io/...`).
fn plain_image(image: String) -> String {
    match image.strip_prefix("docker.io/") {
        Some(rest) => rest.strip_prefix("library/").unwrap_or(rest).to_owned(),
        None => image,
    }
}

fn map_error(err: BollardError) -> DockerError {
    match err {
        BollardError::DockerResponseServerError {
            status_code: 404, ..
        } => DockerError::NotFound,
        BollardError::DockerResponseServerError {
            status_code: 409,
            message,
        } => DockerError::Conflict(clip(&message)),
        // Podman answers a taken name, or the removal of a running container, with 500
        // where Docker says 409.
        BollardError::DockerResponseServerError {
            status_code: 500,
            message,
        } if message.contains("is already in use")
            || (message.contains("cannot remove container") && message.contains("running")) =>
        {
            DockerError::Conflict(clip(&message))
        }
        BollardError::DockerResponseServerError {
            status_code,
            message,
        } => DockerError::Api(format!("{status_code}: {}", clip(&message))),
        BollardError::RequestTimeoutError => DockerError::Unavailable("timed out".into()),
        other @ (BollardError::IOError { .. }
        | BollardError::HyperResponseError { .. }
        | BollardError::SocketNotFoundError(_)) => {
            DockerError::Unavailable(clip(&other.to_string()))
        }
        other => DockerError::Api(clip(&other.to_string())),
    }
}

fn clip(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(300).collect()
}

/// A container's state, as far as callers need it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerInfo {
    /// Docker's id.
    pub id: String,
    /// Our name.
    pub name: String,
    /// The image reference it was created from.
    pub image: String,
    /// `created`, `running`, `exited`, `paused`, `restarting`, `dead`, ...
    pub status: String,
    /// Whether the main process is running.
    pub running: bool,
    /// Exit code of the last run, if it has ended.
    pub exit_code: Option<i64>,
    /// How many times the engine has restarted it by its restart policy.
    pub restart_count: i64,
    /// Labels.
    pub labels: HashMap<String, String>,
    /// The limits in force (zeros if Docker reports none).
    pub limits: Limits,
    /// The ports published on the host, as they are now (a port that was left to Docker is the
    /// one it chose).
    pub published: Vec<PublishedPort>,
}

/// A container of an account, as listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantContainer {
    /// Its name (`bp-<account>-<role>`).
    pub name: String,
    /// What it is for (`web`, `php`, ...).
    pub role: String,
    /// The image it runs.
    pub image: String,
    /// `running`, `exited`, `restarting`, `created`, ...
    pub state: String,
    /// Docker's words ("Up 3 hours").
    pub status: String,
    /// When it was made (Unix seconds).
    pub created: i64,
}

/// A point-in-time reading of a container's resource usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerUsage {
    /// Memory in use, in bytes.
    pub memory_bytes: u64,
    /// The memory limit, in bytes.
    pub memory_limit_bytes: u64,
    /// CPU time used since the container started, in nanoseconds.
    pub cpu_total_nanos: u64,
    /// Processes right now.
    pub pids: u64,
    /// Bytes received on all the container's network interfaces since it started.
    pub net_rx_bytes: u64,
    /// Bytes sent.
    pub net_tx_bytes: u64,
    /// Bytes read from block devices since it started.
    pub io_read_bytes: u64,
    /// Bytes written to block devices.
    pub io_write_bytes: u64,
}

/// Containers read at once by [`DockerManager::usage_of_running`].
const USAGE_PARALLEL: usize = 4;

/// The Docker actions the agent may take.
#[derive(Clone)]
pub struct DockerManager {
    docker: Docker,
    /// Whether the engine is Kiri (asked once, on the first container): Kiri ignores the
    /// `bind` options of a `local` volume, so a tenant's directory is mounted directly.
    engine: std::sync::Arc<tokio::sync::OnceCell<Engine>>,
    /// The OCI runtime every container is made with (`kata` for a node that runs each one in its
    /// own light virtual machine); `None` is the engine's own.
    runtime: Option<String>,
}

/// The engines behind the Docker-compatible socket that differ in ways that matter here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    Docker,
    Kiri,
    Podman,
}

/// The environment variable that names the container runtime of a node (set by the installer).
pub const RUNTIME_ENV: &str = "BUILTPANEL_CONTAINER_RUNTIME";

impl DockerManager {
    /// Connects to the daemon behind a Unix socket. Nothing is sent yet; use
    /// [`DockerManager::version`] to check that it answers.
    ///
    /// # Errors
    /// [`DockerError::Unavailable`] when the client cannot be set up.
    pub fn connect(socket: &Path) -> Result<Self, DockerError> {
        let path = socket
            .to_str()
            .ok_or_else(|| DockerError::Unavailable("the socket path is not valid UTF-8".into()))?;
        let docker =
            Docker::connect_with_unix(path, API_TIMEOUT_SECS, bollard::API_DEFAULT_VERSION)
                .map_err(map_error)?;
        // A runtime name is plain; anything else is ignored rather than passed to the engine.
        let runtime = std::env::var(RUNTIME_ENV).ok().filter(|r| {
            !r.is_empty()
                && r.len() <= 32
                && r.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        });
        Ok(Self {
            docker,
            engine: std::sync::Arc::default(),
            runtime,
        })
    }

    /// The runtime containers are made with, when it is not the engine's own.
    #[must_use]
    pub fn runtime(&self) -> Option<&str> {
        self.runtime.as_deref()
    }

    /// The daemon's version, which also proves it is reachable.
    ///
    /// # Errors
    /// [`DockerError::Unavailable`] or [`DockerError::Api`].
    pub async fn version(&self) -> Result<String, DockerError> {
        let version = self.docker.version().await.map_err(map_error)?;
        Ok(version.version.unwrap_or_default())
    }

    /// Pulls an image by digest. Returns when the image is present locally.
    ///
    /// # Errors
    /// [`DockerError::NotFound`] if the registry does not have it,
    /// [`DockerError::Unavailable`] on time-out, [`DockerError::Api`] otherwise.
    pub async fn pull_image(&self, image: &ImageRef) -> Result<(), DockerError> {
        let options = CreateImageOptionsBuilder::default()
            .from_image(image.as_str())
            .build();
        let pull = async {
            let mut progress = self.docker.create_image(Some(options), None, None);
            while let Some(step) = progress.next().await {
                step.map_err(map_error)?;
            }
            Ok::<(), DockerError>(())
        };
        tokio::time::timeout(PULL_TIMEOUT, pull)
            .await
            .map_err(|_| DockerError::Unavailable("the image pull timed out".into()))??;
        // The pull stream can end without an error for an image that is not there.
        self.docker
            .inspect_image(image.as_str())
            .await
            .map(|_| ())
            .map_err(map_error)
    }

    /// The digest a registry gives `image` (`name:tag`) now: what a pull of it would fetch.
    /// Asks through the daemon; nothing is downloaded.
    ///
    /// # Errors
    /// [`DockerError::Conflict`] for a name that is not plain, [`DockerError::NotFound`] when
    /// the registry has no such image, an API error.
    pub async fn registry_digest(&self, image: &str) -> Result<String, DockerError> {
        let plain = !image.is_empty()
            && image.len() <= 300
            && image
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "./_-:".contains(c));
        if !plain || image.contains("..") {
            return Err(DockerError::Conflict("not a plain image name".into()));
        }
        let found = self
            .docker
            .inspect_registry_image(image, None)
            .await
            .map_err(map_error)?;
        Ok(found.descriptor.digest.unwrap_or_default())
    }

    /// Creates a container (not started). Returns Docker's id.
    ///
    /// # Errors
    /// [`DockerError::Invalid`] for a bad specification, [`DockerError::Conflict`]
    /// if the name is taken, [`DockerError::NotFound`] if the image, a volume or
    /// the network is missing.
    pub async fn create_container(&self, spec: &ContainerSpec) -> Result<String, DockerError> {
        let mut body = spec.to_create_body()?;
        if let Some(runtime) = &self.runtime {
            body.host_config
                .get_or_insert_with(Default::default)
                .runtime = Some(runtime.clone());
        }
        match self.engine().await {
            Engine::Docker => {}
            // Kiri ignores the `bind` options of a `local` volume. Podman honours them but then hands
            // the directory to the container's user on first use (a tenant's directory would stop
            // being the agent's); and it labels nothing for SELinux, so a confined container could
            // not read the directory at all.
            Engine::Kiri => self.bind_volumes_directly(&mut body).await?,
            Engine::Podman => {
                self.bind_volumes_directly(&mut body).await?;
                if let Some(host) = body.host_config.as_mut() {
                    host.security_opt
                        .get_or_insert_with(Vec::new)
                        .push("label=disable".to_owned());
                }
            }
        }
        let options = CreateContainerOptionsBuilder::default()
            .name(spec.name.as_str())
            .build();
        let created = self
            .docker
            .create_container(Some(options), body)
            .await
            .map_err(map_error)?;
        Ok(created.id)
    }

    /// Which engine answers (asked once, on the first container).
    async fn engine(&self) -> Engine {
        *self
            .engine
            .get_or_init(|| async {
                let Ok(v) = self.docker.version().await else {
                    return Engine::Docker;
                };
                if v.platform.as_ref().is_some_and(|p| p.name == "Kiri") {
                    Engine::Kiri
                } else if v
                    .components
                    .iter()
                    .flatten()
                    .any(|c| c.name == "Podman Engine")
                {
                    Engine::Podman
                } else {
                    Engine::Docker
                }
            })
            .await
    }

    /// On Kiri a volume that is a tenant's directory (made by [`Self::create_bind_volume`]:
    /// ours, with `o=bind` and a plain absolute `device`) becomes a bind mount of that same
    /// directory, because Kiri would otherwise give the container an empty volume of its own.
    async fn bind_volumes_directly(
        &self,
        body: &mut bollard::models::ContainerCreateBody,
    ) -> Result<(), DockerError> {
        use bollard::models::MountType;
        let Some(mounts) = body.host_config.as_mut().and_then(|h| h.mounts.as_mut()) else {
            return Ok(());
        };
        for mount in mounts
            .iter_mut()
            .filter(|m| m.typ == Some(MountType::VOLUME))
        {
            let Some(name) = mount.source.clone() else {
                continue;
            };
            let volume = self.docker.inspect_volume(&name).await.map_err(map_error)?;
            let is_ours = volume.labels.get(MANAGED_LABEL).map(String::as_str) == Some("true");
            if !is_ours || volume.options.get("o").map(String::as_str) != Some("bind") {
                continue;
            }
            let device = volume.options.get("device").cloned().unwrap_or_default();
            let plain = Path::new(&device).is_absolute()
                && Path::new(&device).components().all(|c| {
                    matches!(
                        c,
                        std::path::Component::RootDir | std::path::Component::Normal(_)
                    )
                });
            if !plain {
                return Err(DockerError::Conflict(
                    "the volume does not point to a plain directory".into(),
                ));
            }
            mount.typ = Some(MountType::BIND);
            mount.source = Some(device);
            mount.volume_options = None;
        }
        Ok(())
    }

    /// Starts a container we created. Starting a running container is fine.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], [`DockerError::Api`].
    pub async fn start(&self, name: &Name) -> Result<(), DockerError> {
        self.require_managed_container(name).await?;
        self.docker
            .start_container(name.as_str(), None)
            .await
            .map_err(map_error)
    }

    /// Stops a container we created: SIGTERM, then SIGKILL after `grace`.
    /// Stopping a stopped container is fine.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], [`DockerError::Api`].
    pub async fn stop(&self, name: &Name, grace: Duration) -> Result<(), DockerError> {
        self.require_managed_container(name).await?;
        let seconds = i32::try_from(grace.as_secs().min(300)).unwrap_or(300);
        let options = StopContainerOptionsBuilder::default().t(seconds).build();
        self.docker
            .stop_container(name.as_str(), Some(options))
            .await
            .map_err(map_error)
    }

    /// Gives a container we created another name (it keeps running; on its networks the new
    /// name is what resolves from then on). For swapping a new container in place of an old one.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], [`DockerError::Conflict`] when the
    /// new name is taken, [`DockerError::Api`].
    pub async fn rename(&self, from: &Name, to: &Name) -> Result<(), DockerError> {
        self.require_managed_container(from).await?;
        let options = bollard::query_parameters::RenameContainerOptionsBuilder::default()
            .name(to.as_str())
            .build();
        self.docker
            .rename_container(from.as_str(), options)
            .await
            .map_err(map_error)
    }

    /// Removes a container we created (killing it first if `force`). Its
    /// anonymous volumes go with it; named volumes stay.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`],
    /// [`DockerError::Conflict`] if it is running and `force` is false.
    pub async fn remove(&self, name: &Name, force: bool) -> Result<(), DockerError> {
        self.require_managed_container(name).await?;
        let options = RemoveContainerOptionsBuilder::default()
            .force(force)
            .v(true)
            .build();
        self.docker
            .remove_container(name.as_str(), Some(options))
            .await
            .map_err(map_error)
    }

    /// Describes a container we created.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`].
    pub async fn inspect(&self, name: &Name) -> Result<ContainerInfo, DockerError> {
        self.require_managed_container(name).await
    }

    /// Sends a signal to the main process of a running container of ours.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], [`DockerError::Conflict`]
    /// when it is not running, or an API error.
    pub async fn signal(&self, name: &Name, signal: Signal) -> Result<(), DockerError> {
        let info = self.require_managed_container(name).await?;
        if !info.running {
            return Err(DockerError::Conflict("the container is not running".into()));
        }
        self.docker
            .kill_container(
                name.as_str(),
                Some(
                    bollard::query_parameters::KillContainerOptionsBuilder::default()
                        .signal(signal.name())
                        .build(),
                ),
            )
            .await
            .map_err(map_error)
    }

    /// Runs one of the fixed nginx commands inside a running web container of
    /// ours. No argument comes from the caller: the command is the variant.
    ///
    /// # Errors
    /// [`DockerError::NotFound`] (no such container), [`DockerError::NotManaged`],
    /// [`DockerError::Conflict`] when the container is not running, or an API error.
    pub async fn run_nginx(
        &self,
        name: &Name,
        command: NginxCommand,
    ) -> Result<CommandResult, DockerError> {
        let info = self.require_managed_container(name).await?;
        if !info.running {
            return Err(DockerError::Conflict("the container is not running".into()));
        }
        let created = self
            .docker
            .create_exec(
                name.as_str(),
                CreateExecOptions {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    cmd: Some(command.argv().to_vec()),
                    ..Default::default()
                },
            )
            .await
            .map_err(map_error)?;
        let started = self
            .docker
            .start_exec(&created.id, None)
            .await
            .map_err(map_error)?;
        let mut output = String::new();
        if let StartExecResults::Attached {
            output: mut stream, ..
        } = started
        {
            let read = async {
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(map_error)?;
                    if output.len() < MAX_EXEC_OUTPUT {
                        output.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
                    }
                }
                Ok::<(), DockerError>(())
            };
            tokio::time::timeout(EXEC_TIMEOUT, read)
                .await
                .map_err(|_| DockerError::Unavailable("the command timed out".into()))??;
        }
        output.truncate(output.floor_char_boundary(MAX_EXEC_OUTPUT));
        let done = self
            .docker
            .inspect_exec(&created.id)
            .await
            .map_err(map_error)?;
        Ok(CommandResult {
            exit_code: done.exit_code.unwrap_or(-1),
            output,
        })
    }

    /// Runs a cron job's command line (`sh -c`) inside a running container of ours, as the
    /// container's own user, in `workdir`, stopped after `timeout`. The command is the
    /// tenant's own, but it runs **inside the tenant's sandboxed container** (no new
    /// privileges, read-only root, limits, its own network), never on the host.
    ///
    /// Returns the exit code (`124` or `137` when the time ran out) and the end of what the
    /// command printed.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], [`DockerError::Conflict`]
    /// (not running, or a command with a NUL), an API error.
    pub async fn run_user_command(
        &self,
        container: &Name,
        command: &str,
        workdir: &str,
        timeout: Duration,
    ) -> Result<CommandResult, DockerError> {
        if command.is_empty() || command.contains('\0') || !workdir.starts_with('/') {
            return Err(DockerError::Conflict("bad command".into()));
        }
        let info = self.require_managed_container(container).await?;
        if !info.running {
            return Err(DockerError::Conflict("the container is not running".into()));
        }
        // `timeout` ends the process inside the container; the wait below is a second belt.
        let seconds = timeout.as_secs().max(1).to_string();
        let created = self
            .docker
            .create_exec(
                container.as_str(),
                CreateExecOptions {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    working_dir: Some(workdir.to_owned()),
                    cmd: Some(vec![
                        "timeout".to_owned(),
                        "-s".to_owned(),
                        "KILL".to_owned(),
                        seconds,
                        "sh".to_owned(),
                        "-c".to_owned(),
                        command.to_owned(),
                    ]),
                    ..Default::default()
                },
            )
            .await
            .map_err(map_error)?;
        let started = self
            .docker
            .start_exec(&created.id, None)
            .await
            .map_err(map_error)?;
        let mut output = String::new();
        if let StartExecResults::Attached {
            output: mut stream, ..
        } = started
        {
            let read = async {
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(map_error)?;
                    output.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
                    // Keep the end: the last lines say how it went.
                    if output.len() > 2 * MAX_EXEC_OUTPUT {
                        let cut = output.ceil_char_boundary(output.len() - MAX_EXEC_OUTPUT);
                        output.drain(..cut);
                    }
                }
                Ok::<(), DockerError>(())
            };
            tokio::time::timeout(timeout + Duration::from_secs(10), read)
                .await
                .map_err(|_| DockerError::Unavailable("the command timed out".into()))??;
        }
        if output.len() > MAX_EXEC_OUTPUT {
            let cut = output.ceil_char_boundary(output.len() - MAX_EXEC_OUTPUT);
            output.drain(..cut);
        }
        let done = self
            .docker
            .inspect_exec(&created.id)
            .await
            .map_err(map_error)?;
        Ok(CommandResult {
            exit_code: done.exit_code.unwrap_or(-1),
            output,
        })
    }

    /// Runs a container of ours to its end: creates it, starts it, waits until it stops (or
    /// `limit` passes, when it is killed), reads what it printed and removes it. For one-off
    /// jobs such as a malware scan. Returns the exit code (`137` when it had to be killed) and
    /// the end of what it printed (stdout and stderr together, about 1 MiB at most).
    ///
    /// # Errors
    /// [`DockerError::Conflict`] when the name is taken, an API error.
    pub async fn run_to_end(
        &self,
        spec: &ContainerSpec,
        limit: Duration,
    ) -> Result<CommandResult, DockerError> {
        self.create_container(spec).await?;
        let result = async {
            self.start(&spec.name).await?;
            let deadline = tokio::time::Instant::now() + limit;
            let mut killed = false;
            loop {
                let info = self.inspect(&spec.name).await?;
                if !info.running {
                    break Ok::<Option<i64>, DockerError>(info.exit_code);
                }
                if tokio::time::Instant::now() >= deadline {
                    killed = true;
                    self.stop(&spec.name, Duration::from_secs(5)).await?;
                    break Ok(None);
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            .map(|code| (code, killed))
        }
        .await;
        let read = match &result {
            Ok(_) => self.logs(&spec.name, 0, 200_000).await.ok(),
            Err(_) => None,
        };
        let _ = self.remove(&spec.name, true).await;
        let (code, killed) = result?;
        let mut output = String::new();
        if let Some(read) = read {
            for line in read.lines {
                output.push_str(&line.text);
                output.push('\n');
            }
        }
        Ok(CommandResult {
            exit_code: if killed { 137 } else { code.unwrap_or(-1) },
            output,
        })
    }

    /// Dumps one database with `mariadb-dump` run inside the database container
    /// and sends what it prints to `sink`, a piece at a time. The command is fixed;
    /// only `database` and `user` (plain identifiers, checked here) and the
    /// password (passed in the command's environment, never its arguments) vary.
    ///
    /// Returns the exit code and what the command said on stderr (clipped).
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], [`DockerError::Conflict`]
    /// (not running, or a name that is not a plain identifier), an API error, or
    /// [`DockerError::Unavailable`] when the receiver is gone or `limit` passes.
    pub async fn dump_database(
        &self,
        container: &Name,
        database: &str,
        user: &str,
        password: &str,
        sink: tokio::sync::mpsc::Sender<Vec<u8>>,
        limit: Duration,
    ) -> Result<(i64, String), DockerError> {
        let plain = |s: &str| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        };
        if !plain(database) || !plain(user) || password.is_empty() || password.contains('\0') {
            return Err(DockerError::Conflict("bad dump arguments".into()));
        }
        let info = self.require_managed_container(container).await?;
        if !info.running {
            return Err(DockerError::Conflict("the container is not running".into()));
        }
        let cmd: Vec<String> = [
            "mariadb-dump",
            "--host=127.0.0.1",
            "--single-transaction",
            "--quick",
            "--routines",
            "--triggers",
            "--events",
            "--hex-blob",
            "--skip-lock-tables",
            "--default-character-set=utf8mb4",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain([format!("--user={user}"), database.to_owned()])
        .collect();
        let created = self
            .docker
            .create_exec(
                container.as_str(),
                CreateExecOptions {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    cmd: Some(cmd),
                    env: Some(vec![format!("MYSQL_PWD={password}")]),
                    ..Default::default()
                },
            )
            .await
            .map_err(map_error)?;
        let started = self
            .docker
            .start_exec(&created.id, None)
            .await
            .map_err(map_error)?;
        let mut stderr = String::new();
        if let StartExecResults::Attached {
            output: mut stream, ..
        } = started
        {
            let read = async {
                while let Some(chunk) = stream.next().await {
                    match chunk.map_err(map_error)? {
                        bollard::container::LogOutput::StdOut { message } => {
                            sink.send(message.to_vec()).await.map_err(|_| {
                                DockerError::Unavailable("nothing is taking the dump".into())
                            })?;
                        }
                        bollard::container::LogOutput::StdErr { message }
                            if stderr.len() < MAX_EXEC_OUTPUT =>
                        {
                            stderr.push_str(&String::from_utf8_lossy(&message));
                        }
                        _ => {}
                    }
                }
                Ok::<(), DockerError>(())
            };
            tokio::time::timeout(limit, read)
                .await
                .map_err(|_| DockerError::Unavailable("the dump took too long".into()))??;
        }
        stderr.truncate(stderr.floor_char_boundary(MAX_EXEC_OUTPUT));
        let done = self
            .docker
            .inspect_exec(&created.id)
            .await
            .map_err(map_error)?;
        Ok((done.exit_code.unwrap_or(-1), stderr))
    }

    /// Dumps one database with `pg_dump` run inside the database container
    /// and sends what it prints to `sink`, a piece at a time. The command is fixed;
    /// only `database` and `user` (plain identifiers, checked here) and the
    /// password (passed in the command's environment, never its arguments) vary.
    ///
    /// Returns the exit code and what the command said on stderr (clipped).
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], [`DockerError::Conflict`]
    /// (not running, or a name that is not a plain identifier), an API error, or
    /// [`DockerError::Unavailable`] when the receiver is gone or `limit` passes.
    pub async fn dump_pg_database(
        &self,
        container: &Name,
        database: &str,
        user: &str,
        password: &str,
        sink: tokio::sync::mpsc::Sender<Vec<u8>>,
        limit: Duration,
    ) -> Result<(i64, String), DockerError> {
        let plain = |s: &str| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        };
        if !plain(database) || !plain(user) || password.is_empty() || password.contains('\0') {
            return Err(DockerError::Conflict("bad dump arguments".into()));
        }
        let info = self.require_managed_container(container).await?;
        if !info.running {
            return Err(DockerError::Conflict("the container is not running".into()));
        }
        // A plain-text dump the importer can read back: no owners and no privileges (the
        // server it goes to has other roles), `COPY` for the rows.
        let cmd: Vec<String> = [
            "pg_dump",
            "--host=127.0.0.1",
            "--no-owner",
            "--no-privileges",
            "--encoding=UTF8",
            "--no-password",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain([format!("--username={user}"), format!("--dbname={database}")])
        .collect();
        let created = self
            .docker
            .create_exec(
                container.as_str(),
                CreateExecOptions {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    cmd: Some(cmd),
                    env: Some(vec![format!("PGPASSWORD={password}")]),
                    ..Default::default()
                },
            )
            .await
            .map_err(map_error)?;
        let started = self
            .docker
            .start_exec(&created.id, None)
            .await
            .map_err(map_error)?;
        let mut stderr = String::new();
        if let StartExecResults::Attached {
            output: mut stream, ..
        } = started
        {
            let read = async {
                while let Some(chunk) = stream.next().await {
                    match chunk.map_err(map_error)? {
                        bollard::container::LogOutput::StdOut { message } => {
                            sink.send(message.to_vec()).await.map_err(|_| {
                                DockerError::Unavailable("nothing is taking the dump".into())
                            })?;
                        }
                        bollard::container::LogOutput::StdErr { message }
                            if stderr.len() < MAX_EXEC_OUTPUT =>
                        {
                            stderr.push_str(&String::from_utf8_lossy(&message));
                        }
                        _ => {}
                    }
                }
                Ok::<(), DockerError>(())
            };
            tokio::time::timeout(limit, read)
                .await
                .map_err(|_| DockerError::Unavailable("the dump took too long".into()))??;
        }
        stderr.truncate(stderr.floor_char_boundary(MAX_EXEC_OUTPUT));
        let done = self
            .docker
            .inspect_exec(&created.id)
            .await
            .map_err(map_error)?;
        Ok((done.exit_code.unwrap_or(-1), stderr))
    }

    /// The end of a container's log: at most `tail` lines at or after `since_ms` (Unix
    /// milliseconds; `0` for any), oldest first, each with its time and stream. Only
    /// containers of ours. About 1 MiB of text is kept, the newest; the answer says when older lines were dropped.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], an API error.
    pub async fn logs(
        &self,
        name: &Name,
        since_ms: i64,
        tail: u32,
    ) -> Result<LogRead, DockerError> {
        let mut lines: std::collections::VecDeque<LogLine> = std::collections::VecDeque::new();
        let mut bytes = 0usize;
        let mut truncated = false;
        self.visit_logs(name, since_ms, tail, |at_ms, stderr, text| {
            bytes += text.len();
            lines.push_back(LogLine {
                at_ms,
                stderr,
                text: text.to_owned(),
            });
            // The newest lines are the ones worth keeping.
            while bytes > MAX_LOG_BYTES {
                truncated = true;
                match lines.pop_front() {
                    Some(old) => bytes -= old.text.len(),
                    None => break,
                }
            }
        })
        .await?;
        let mut lines: Vec<LogLine> = lines.into();
        // stdout and stderr arrive as they were written; two streams are put in time order.
        lines.sort_by_key(|l| l.at_ms);
        Ok(LogRead { lines, truncated })
    }

    /// Calls `visit(at_ms, is_stderr, text)` for each line of a container's log at or after
    /// `since_ms`, as they arrive, keeping none of them (for counting over a long log). At
    /// most `tail` lines are read (the newest). Only containers of ours.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], an API error.
    pub async fn visit_logs(
        &self,
        name: &Name,
        since_ms: i64,
        tail: u32,
        mut visit: impl FnMut(i64, bool, &str) + Send,
    ) -> Result<(), DockerError> {
        self.require_managed_container(name).await?;
        let options = bollard::query_parameters::LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .timestamps(true)
            .since(i32::try_from((since_ms / 1000).max(0)).unwrap_or(i32::MAX))
            .tail(&tail.to_string())
            .build();
        let mut stream = self.docker.logs(name.as_str(), Some(options));
        let mut pending: [String; 2] = [String::new(), String::new()];
        while let Some(chunk) = stream.next().await {
            let (is_err, message) = match chunk.map_err(map_error)? {
                bollard::container::LogOutput::StdOut { message } => (0, message),
                bollard::container::LogOutput::StdErr { message } => (1, message),
                _ => continue,
            };
            pending[is_err].push_str(&String::from_utf8_lossy(&message));
            while let Some(end) = pending[is_err].find('\n') {
                let line: String = pending[is_err].drain(..=end).collect();
                let line = line.trim_end_matches(['\n', '\r']);
                let (at_ms, text) = split_timestamp(line);
                if at_ms < since_ms {
                    continue;
                }
                visit(at_ms, is_err == 1, text);
            }
        }
        Ok(())
    }

    /// Reads a running container's resource usage once.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], [`DockerError::Api`].
    pub async fn usage(&self, name: &Name) -> Result<ContainerUsage, DockerError> {
        self.require_managed_container(name).await?;
        let options = StatsOptionsBuilder::default()
            .stream(false)
            .one_shot(true)
            .build();
        let mut stream = self.docker.stats(name.as_str(), Some(options));
        let stats = stream
            .next()
            .await
            .ok_or_else(|| DockerError::Api("no statistics returned".into()))?
            .map_err(map_error)?;
        Ok(usage_from(&stats))
    }

    /// The usage of every running container of ours, read a few at a time (one call to list
    /// them, then one statistics read each, without the checks [`DockerManager::usage`] does).
    /// A container that stops meanwhile is left out.
    ///
    /// # Errors
    /// [`DockerError::Api`] when the containers cannot be listed.
    pub async fn usage_of_running(&self) -> Result<Vec<(String, ContainerUsage)>, DockerError> {
        use futures_util::stream::StreamExt as _;
        let filters: HashMap<String, Vec<String>> = [
            ("label".to_owned(), vec![format!("{}=true", MANAGED_LABEL)]),
            ("status".to_owned(), vec!["running".to_owned()]),
        ]
        .into();
        let options = ListContainersOptionsBuilder::default()
            .filters(&filters)
            .build();
        let names: Vec<String> = self
            .docker
            .list_containers(Some(options))
            .await
            .map_err(map_error)?
            .into_iter()
            .filter_map(|c| c.names.and_then(|n| n.into_iter().next()))
            .map(|n| n.trim_start_matches('/').to_owned())
            .collect();
        let mut found: Vec<(String, ContainerUsage)> = futures_util::stream::iter(names)
            .map(|name| async move {
                let options = StatsOptionsBuilder::default()
                    .stream(false)
                    .one_shot(true)
                    .build();
                let mut stream = self.docker.stats(&name, Some(options));
                let stats = stream.next().await?.ok()?;
                Some((name, usage_from(&stats)))
            })
            .buffer_unordered(USAGE_PARALLEL)
            .filter_map(|r| async move { r })
            .collect()
            .await;
        found.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(found)
    }

    /// Changes the limits of a container we created, running or not, without
    /// restarting it. Memory and swap are set to the same value, so a container
    /// never gets swap.
    ///
    /// # Errors
    /// [`DockerError::Invalid`] for numbers out of range, [`DockerError::NotFound`],
    /// [`DockerError::NotManaged`], [`DockerError::Api`] (for example when the new
    /// memory limit is below what the container already uses).
    pub async fn update_limits(&self, name: &Name, limits: &Limits) -> Result<(), DockerError> {
        limits.validate()?;
        self.require_managed_container(name).await?;
        // Validated above: at most 1 TiB, so this always fits.
        let memory = i64::try_from(limits.memory_bytes).unwrap_or(i64::MAX);
        let body = ContainerUpdateBody {
            memory: Some(memory),
            memory_swap: Some(memory),
            nano_cpus: Some(i64::from(limits.cpu_millis) * 1_000_000),
            pids_limit: Some(i64::from(limits.pids_limit)),
            ..Default::default()
        };
        self.docker
            .update_container(name.as_str(), body)
            .await
            .map_err(map_error)
    }

    /// Names of all containers we created (running or not), sorted.
    ///
    /// # Errors
    /// [`DockerError::Unavailable`] or [`DockerError::Api`].
    pub async fn list(&self) -> Result<Vec<String>, DockerError> {
        let filters: HashMap<String, Vec<String>> =
            [("label".to_owned(), vec![format!("{}=true", MANAGED_LABEL)])].into();
        let options = ListContainersOptionsBuilder::default()
            .all(true)
            .filters(&filters)
            .build();
        let mut names: Vec<String> = self
            .docker
            .list_containers(Some(options))
            .await
            .map_err(map_error)?
            .into_iter()
            .filter_map(|c| c.names.and_then(|n| n.into_iter().next()))
            .map(|n| n.trim_start_matches('/').to_owned())
            .collect();
        names.sort();
        Ok(names)
    }

    /// The containers of one account (the label `builtpanel.tenant` is its id), running or not.
    ///
    /// # Errors
    /// [`DockerError::Api`].
    pub async fn list_tenant(&self, tenant_id: &str) -> Result<Vec<TenantContainer>, DockerError> {
        let filters: HashMap<String, Vec<String>> = [(
            "label".to_owned(),
            vec![
                format!("{}=true", MANAGED_LABEL),
                format!("{}={tenant_id}", TENANT_LABEL),
            ],
        )]
        .into();
        let options = ListContainersOptionsBuilder::default()
            .all(true)
            .filters(&filters)
            .build();
        let mut found: Vec<TenantContainer> = self
            .docker
            .list_containers(Some(options))
            .await
            .map_err(map_error)?
            .into_iter()
            .filter_map(|c| {
                let name = c
                    .names
                    .and_then(|n| n.into_iter().next())?
                    .trim_start_matches('/')
                    .to_owned();
                let labels = c.labels.unwrap_or_default();
                Some(TenantContainer {
                    name,
                    role: labels.get(ROLE_LABEL).cloned().unwrap_or_default(),
                    image: plain_image(c.image.unwrap_or_default()),
                    state: c.state.map(|s| s.to_string()).unwrap_or_default(),
                    status: c.status.unwrap_or_default(),
                    created: c.created.unwrap_or(0),
                })
            })
            .collect();
        found.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(found)
    }

    /// Restarts a container of ours: SIGTERM, SIGKILL after `grace`, then start.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], [`DockerError::Api`].
    pub async fn restart(&self, name: &Name, grace: Duration) -> Result<(), DockerError> {
        self.stop(name, grace).await?;
        self.start(name).await
    }

    /// Creates a named volume. Creating one that already exists (and is ours)
    /// is fine.
    ///
    /// # Errors
    /// [`DockerError::NotManaged`] if a volume of that name exists that we did
    /// not create.
    pub async fn create_volume(&self, name: &Name) -> Result<(), DockerError> {
        match self.docker.inspect_volume(name.as_str()).await {
            Ok(existing) => {
                return if existing.labels.get(MANAGED_LABEL).map(String::as_str) == Some("true") {
                    Ok(())
                } else {
                    Err(DockerError::NotManaged)
                };
            }
            Err(err) => match map_error(err) {
                DockerError::NotFound => {}
                other => return Err(other),
            },
        }
        self.docker
            .create_volume(VolumeCreateRequest {
                name: Some(name.as_str().to_owned()),
                labels: Some([(MANAGED_LABEL.to_owned(), "true".to_owned())].into()),
                ..Default::default()
            })
            .await
            .map(|_| ())
            .map_err(map_error)
    }

    /// Creates a named volume whose data lives in a host directory (the
    /// tenant's directory, prepared by the privileged helper). The directory
    /// must be an absolute, plain path *below* `allowed_root` (a path with
    /// `..`, a symlink-looking trick or anything outside the root is refused),
    /// so a request can never make Docker expose an arbitrary host path.
    /// Creating one that already exists with the same directory is fine.
    ///
    /// # Errors
    /// [`DockerError::Invalid`] for a path outside the root,
    /// [`DockerError::NotManaged`] or [`DockerError::Conflict`] if the volume
    /// exists but is not ours or points somewhere else.
    pub async fn create_bind_volume(
        &self,
        name: &Name,
        host_dir: &Path,
        allowed_root: &Path,
    ) -> Result<(), DockerError> {
        let device = bind_device(host_dir, allowed_root)?;
        match self.docker.inspect_volume(name.as_str()).await {
            Ok(existing) => {
                if existing.labels.get(MANAGED_LABEL).map(String::as_str) != Some("true") {
                    return Err(DockerError::NotManaged);
                }
                return if existing.options.get("device").map(String::as_str)
                    == Some(device.as_str())
                {
                    Ok(())
                } else {
                    Err(DockerError::Conflict(
                        "the volume points to another directory".into(),
                    ))
                };
            }
            Err(err) => match map_error(err) {
                DockerError::NotFound => {}
                other => return Err(other),
            },
        }
        self.docker
            .create_volume(VolumeCreateRequest {
                name: Some(name.as_str().to_owned()),
                driver: Some("local".to_owned()),
                driver_opts: Some(
                    [
                        ("type".to_owned(), "none".to_owned()),
                        ("o".to_owned(), "bind".to_owned()),
                        ("device".to_owned(), device),
                    ]
                    .into(),
                ),
                labels: Some([(MANAGED_LABEL.to_owned(), "true".to_owned())].into()),
                ..Default::default()
            })
            .await
            .map(|_| ())
            .map_err(map_error)
    }

    /// Removes a named volume we created, with its data.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`],
    /// [`DockerError::Conflict`] while a container uses it.
    pub async fn remove_volume(&self, name: &Name) -> Result<(), DockerError> {
        let volume = self
            .docker
            .inspect_volume(name.as_str())
            .await
            .map_err(map_error)?;
        if volume.labels.get(MANAGED_LABEL).map(String::as_str) != Some("true") {
            return Err(DockerError::NotManaged);
        }
        self.docker
            .remove_volume(
                name.as_str(),
                None::<bollard::query_parameters::RemoveVolumeOptions>,
            )
            .await
            .map_err(map_error)
    }

    /// Creates a bridge network. With `internal`, containers on it can talk to
    /// each other but not to the outside.
    ///
    /// # Errors
    /// [`DockerError::Conflict`] if the name is taken.
    pub async fn create_network(&self, name: &Name, internal: bool) -> Result<(), DockerError> {
        // Docker refuses a second network of the same name; Podman's compatible API makes a
        // twin instead, so the name is checked here for both.
        match self.docker.inspect_network(name.as_str(), None).await {
            Ok(_) => return Err(DockerError::Conflict("the network exists".into())),
            Err(BollardError::DockerResponseServerError {
                status_code: 404, ..
            }) => {}
            Err(other) => return Err(map_error(other)),
        }
        self.docker
            .create_network(NetworkCreateRequest {
                name: name.as_str().to_owned(),
                driver: Some("bridge".to_owned()),
                internal: Some(internal),
                labels: Some([(MANAGED_LABEL.to_owned(), "true".to_owned())].into()),
                ..Default::default()
            })
            .await
            .map(|_| ())
            .map_err(map_error)
    }

    /// Puts a running or stopped container of ours on a network of ours, under an
    /// extra name (`alias`) that only that network's members see. Doing it again is
    /// fine.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], [`DockerError::Invalid`]
    /// for a bad alias, or an API error.
    pub async fn connect_network(
        &self,
        container: &Name,
        network: &Name,
        alias: &str,
    ) -> Result<(), DockerError> {
        let plain = !alias.is_empty()
            && alias.len() <= 63
            && alias
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !plain {
            return Err(DockerError::Invalid(SpecError {
                field: "alias",
                reason: "a network alias is lower-case letters, digits and dashes".into(),
            }));
        }
        self.require_managed_container(container).await?;
        let net = self
            .docker
            .inspect_network(network.as_str(), None)
            .await
            .map_err(map_error)?;
        if net
            .labels
            .as_ref()
            .and_then(|l| l.get(MANAGED_LABEL))
            .map(String::as_str)
            != Some("true")
        {
            return Err(DockerError::NotManaged);
        }
        match self
            .docker
            .connect_network(
                network.as_str(),
                NetworkConnectRequest {
                    container: container.as_str().to_owned(),
                    endpoint_config: Some(EndpointSettings {
                        aliases: Some(vec![alias.to_owned()]),
                        ..Default::default()
                    }),
                },
            )
            .await
        {
            Ok(()) => Ok(()),
            // Already on it.
            Err(BollardError::DockerResponseServerError {
                status_code: 403 | 409,
                message,
            }) if message.contains("already exists") || message.contains("already connected") => {
                Ok(())
            }
            Err(e) => Err(map_error(e)),
        }
    }

    /// Takes a container of ours off one of our networks. Not being on it is fine.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], or an API error.
    pub async fn disconnect_network(
        &self,
        container: &Name,
        network: &Name,
    ) -> Result<(), DockerError> {
        self.require_managed_container(container).await?;
        let net = self
            .docker
            .inspect_network(network.as_str(), None)
            .await
            .map_err(map_error)?;
        if net
            .labels
            .as_ref()
            .and_then(|l| l.get(MANAGED_LABEL))
            .map(String::as_str)
            != Some("true")
        {
            return Err(DockerError::NotManaged);
        }
        match self
            .docker
            .disconnect_network(
                network.as_str(),
                bollard::models::NetworkDisconnectRequest {
                    container: container.as_str().to_owned(),
                    force: Some(false),
                },
            )
            .await
        {
            Ok(()) => Ok(()),
            // Not connected to it.
            Err(BollardError::DockerResponseServerError { message, .. })
                if message.contains("not connected") =>
            {
                Ok(())
            }
            Err(e) => Err(map_error(e)),
        }
    }

    /// The address range (CIDR) of one of our networks.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`], or an API error
    /// (also when the network has no range).
    pub async fn network_subnet(&self, network: &Name) -> Result<String, DockerError> {
        let net = self
            .docker
            .inspect_network(network.as_str(), None)
            .await
            .map_err(map_error)?;
        if net
            .labels
            .as_ref()
            .and_then(|l| l.get(MANAGED_LABEL))
            .map(String::as_str)
            != Some("true")
        {
            return Err(DockerError::NotManaged);
        }
        net.ipam
            .and_then(|i| i.config)
            .and_then(|c| {
                c.into_iter()
                    .filter_map(|c| c.subnet)
                    .find(|s| !s.contains(':'))
            })
            .ok_or_else(|| DockerError::Api("the network has no address range".into()))
    }

    /// Creates the network unless it already exists and is ours.
    ///
    /// # Errors
    /// [`DockerError::NotManaged`] if a network of that name exists that we did
    /// not create.
    pub async fn ensure_network(&self, name: &Name, internal: bool) -> Result<(), DockerError> {
        match self.docker.inspect_network(name.as_str(), None).await {
            Ok(existing) => {
                let ours = existing
                    .labels
                    .as_ref()
                    .and_then(|l| l.get(MANAGED_LABEL))
                    .map(String::as_str)
                    == Some("true");
                return if ours {
                    Ok(())
                } else {
                    Err(DockerError::NotManaged)
                };
            }
            Err(err) => match map_error(err) {
                DockerError::NotFound => {}
                other => return Err(other),
            },
        }
        self.create_network(name, internal).await
    }

    /// Removes a network we created.
    ///
    /// # Errors
    /// [`DockerError::NotFound`], [`DockerError::NotManaged`],
    /// [`DockerError::Conflict`] while a container is attached.
    pub async fn remove_network(&self, name: &Name) -> Result<(), DockerError> {
        let network = self
            .docker
            .inspect_network(name.as_str(), None)
            .await
            .map_err(map_error)?;
        let ours = network
            .labels
            .as_ref()
            .and_then(|l| l.get(MANAGED_LABEL))
            .map(String::as_str)
            == Some("true");
        if !ours {
            return Err(DockerError::NotManaged);
        }
        self.docker
            .remove_network(name.as_str())
            .await
            .map_err(map_error)
    }

    async fn require_managed_container(&self, name: &Name) -> Result<ContainerInfo, DockerError> {
        // Podman reports a container that is just ending as `stopped`, a state Docker's API
        // does not have and the client cannot read; it is `exited` a moment later.
        let mut tries = 0;
        let c = loop {
            match self.docker.inspect_container(name.as_str(), None).await {
                Ok(c) => break c,
                Err(e) if tries < 20 && e.to_string().contains("unknown variant `stopped`") => {
                    tries += 1;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(e) => return Err(map_error(e)),
            }
        };
        let config = c.config.unwrap_or_default();
        let labels = config.labels.unwrap_or_default();
        if labels.get(MANAGED_LABEL).map(String::as_str) != Some("true") {
            return Err(DockerError::NotManaged);
        }
        let state = c.state.unwrap_or_default();
        let running = state.running.unwrap_or(false);
        let mut published = Vec::new();
        for (key, bindings) in c.network_settings.and_then(|n| n.ports).unwrap_or_default() {
            let Some(container_port) = key.strip_suffix("/tcp").and_then(|p| p.parse::<u16>().ok())
            else {
                continue;
            };
            for binding in bindings.unwrap_or_default() {
                if let (Some(ip), Some(port)) = (
                    binding
                        .host_ip
                        .and_then(|i| i.parse::<std::net::IpAddr>().ok()),
                    binding.host_port.and_then(|p| p.parse::<u16>().ok()),
                ) {
                    published.push(PublishedPort {
                        container_port,
                        host_port: port,
                        host_ip: ip,
                    });
                }
            }
        }
        Ok(ContainerInfo {
            restart_count: c.restart_count.unwrap_or(0),
            published,
            id: c.id.unwrap_or_default(),
            name: c
                .name
                .unwrap_or_default()
                .trim_start_matches('/')
                .to_owned(),
            image: plain_image(config.image.unwrap_or_default()),
            status: state.status.map(|s| s.to_string()).unwrap_or_default(),
            running,
            exit_code: (!running).then_some(state.exit_code).flatten(),
            labels: labels.into_iter().collect(),
            limits: {
                let host = c.host_config.unwrap_or_default();
                Limits {
                    memory_bytes: host.memory.and_then(|m| u64::try_from(m).ok()).unwrap_or(0),
                    cpu_millis: host
                        .nano_cpus
                        .and_then(|n| u32::try_from(n / 1_000_000).ok())
                        .unwrap_or(0),
                    pids_limit: host
                        .pids_limit
                        .and_then(|p| u32::try_from(p).ok())
                        .unwrap_or(0),
                }
            },
        })
    }
}

/// Checks that `dir` is an absolute path strictly below `root`, made of plain
/// components only, and returns it as text for Docker.
fn bind_device(dir: &Path, root: &Path) -> Result<String, DockerError> {
    use std::path::Component;
    let plain = |p: &Path| {
        p.is_absolute()
            && p.components()
                .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
    };
    let bad = |reason: &str| {
        DockerError::Invalid(SpecError {
            field: "host_dir",
            reason: reason.to_owned(),
        })
    };
    if !plain(dir) || !plain(root) {
        return Err(bad("must be an absolute path without `.` or `..`"));
    }
    if dir == root || !dir.starts_with(root) {
        return Err(bad("must be inside the allowed directory"));
    }
    let text = dir.to_str().ok_or_else(|| bad("must be valid UTF-8"))?;
    // `Path` quietly drops `.` and repeated or trailing slashes when it walks the
    // components; what Docker receives is the text, so the text must already be
    // in its plain form.
    let normal: std::path::PathBuf = dir.components().collect();
    if normal.to_str() != Some(text) {
        return Err(bad(
            "must be written plainly (no `.`, `//` or trailing `/`)",
        ));
    }
    if text.chars().any(|c| c.is_control() || c == ',' || c == '=') {
        return Err(bad("contains a character Docker options cannot carry"));
    }
    Ok(text.to_owned())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn bind_directories_must_be_plain_paths_inside_the_root() {
        let root = Path::new("/srv/tenants");
        assert_eq!(
            bind_device(Path::new("/srv/tenants/0190b6a2/www"), root).unwrap(),
            "/srv/tenants/0190b6a2/www"
        );
        for bad in [
            "/srv/tenants",
            "/srv",
            "/etc",
            "/srv/tenants/../etc",
            "/srv/tenants/./x",
            "srv/tenants/x",
            "/srv/tenantsX/x",
            "/srv/tenants/a,b",
            "/srv/tenants/a=b",
            "/srv/tenants/a\nb",
        ] {
            assert!(bind_device(Path::new(bad), root).is_err(), "{bad:?}");
        }
        assert!(bind_device(Path::new("/srv/tenants/x"), Path::new("/srv/../etc")).is_err());
    }
}
