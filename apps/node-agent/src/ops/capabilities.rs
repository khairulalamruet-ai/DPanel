//! What this server can do: the answer to `report_capabilities` (T02.13).
//!
//! Everything is read from the kernel's own files (`/proc`, `/sys`), a
//! `statvfs` call, and two read-only questions to the Docker daemon the agent is
//! configured for. Nothing is executed. A fact that cannot be read is reported
//! as absent or `false`, never guessed.

use std::{path::Path, time::Duration};

use builtpanel_node_protocol::{AgentError, Capabilities, DockerMode, ErrorCode};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::UnixStream,
};

/// Longest the Docker daemon may take to answer.
const DOCKER_TIMEOUT: Duration = Duration::from_secs(3);
/// Largest answer read from the Docker daemon.
const DOCKER_MAX_BYTES: u64 = 1 << 20;
/// Longest string reported (kernel release, Docker version).
const MAX_TEXT: usize = 128;

/// Where to look. Defaults describe a real server; tests point at fixtures.
#[derive(Debug, Clone)]
pub struct Sources {
    /// The agent's data directory; disk figures are for its filesystem.
    pub data_dir: std::path::PathBuf,
    /// `/proc`.
    pub proc_dir: std::path::PathBuf,
    /// `/sys`.
    pub sys_dir: std::path::PathBuf,
    /// The Docker daemon's Unix socket, if one is configured.
    pub docker_socket: Option<std::path::PathBuf>,
    /// Where tenant directories live; quota support is looked up for it.
    pub tenants_root: Option<std::path::PathBuf>,
}

impl Sources {
    /// The real locations, with the Docker socket from `DOCKER_HOST`
    /// (`unix:///path`), which the systemd unit sets to the agent's own
    /// rootless daemon.
    #[must_use]
    pub fn system(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_owned(),
            proc_dir: "/proc".into(),
            sys_dir: "/sys".into(),
            docker_socket: std::env::var("DOCKER_HOST")
                .ok()
                .and_then(|host| host.strip_prefix("unix://").map(Into::into)),
            tenants_root: Some(
                std::env::var_os("BUILTPANEL_AGENT_TENANTS_ROOT")
                    .map_or_else(|| "/srv/tenants".into(), Into::into),
            ),
        }
    }
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn clip(text: &str) -> String {
    text.trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TEXT)
        .collect()
}

/// `MemTotal` from `/proc/meminfo` text, in bytes.
#[must_use]
pub fn parse_mem_total(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// Whether `/proc/self/status` text shows a kernel built with seccomp.
#[must_use]
pub fn parse_seccomp(status: &str) -> bool {
    status.lines().any(|l| l.starts_with("Seccomp:"))
}

/// What the Docker daemon says about itself (`GET /info`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DockerFacts {
    /// Engine version.
    pub version: Option<String>,
    /// Rootless or rootful.
    pub mode: Option<DockerMode>,
}

/// Reads the interesting parts of Docker's `/info` JSON.
#[must_use]
pub fn parse_docker_info(body: &str) -> DockerFacts {
    let Ok(info) = serde_json::from_str::<serde_json::Value>(body) else {
        return DockerFacts::default();
    };
    let version = info
        .get("ServerVersion")
        .and_then(|v| v.as_str())
        .map(clip)
        .filter(|v| !v.is_empty());
    let rootless = info
        .get("SecurityOptions")
        .and_then(|v| v.as_array())
        .is_some_and(|opts| {
            opts.iter()
                .filter_map(|o| o.as_str())
                .any(|o| o.split(',').any(|part| part == "name=rootless"))
        });
    DockerFacts {
        mode: version.as_ref().map(|_| {
            if rootless {
                DockerMode::Rootless
            } else {
                DockerMode::Rootful
            }
        }),
        version,
    }
}

/// Splits an HTTP/1.x response into status code and body. Understands a plain
/// body and chunked transfer encoding (what the Docker daemon uses).
fn http_body(raw: &[u8]) -> Option<(u16, String)> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text.split_once("\r\n\r\n")?;
    let status: u16 = head
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    let chunked = head
        .lines()
        .any(|l| l.to_ascii_lowercase().replace(' ', "") == "transfer-encoding:chunked");
    if !chunked {
        return Some((status, body.to_owned()));
    }
    let mut out = String::new();
    let mut rest = body;
    loop {
        let (size_line, after) = rest.split_once("\r\n")?;
        let size = usize::from_str_radix(size_line.split(';').next()?.trim(), 16).ok()?;
        if size == 0 {
            return Some((status, out));
        }
        out.push_str(after.get(..size)?);
        rest = after.get(size..)?.strip_prefix("\r\n")?;
    }
}

/// Asks the Docker daemon on `socket` for `/info`. `None` when there is no
/// daemon, it does not answer in time, or the answer is not usable.
async fn docker_info(socket: &Path) -> Option<DockerFacts> {
    let ask = async {
        let mut stream = UnixStream::connect(socket).await.ok()?;
        stream
            .write_all(b"GET /info HTTP/1.1\r\nHost: docker\r\nConnection: close\r\n\r\n")
            .await
            .ok()?;
        let mut raw = Vec::new();
        stream
            .take(DOCKER_MAX_BYTES)
            .read_to_end(&mut raw)
            .await
            .ok()?;
        let (status, body) = http_body(&raw)?;
        (status == 200).then(|| parse_docker_info(&body))
    };
    tokio::time::timeout(DOCKER_TIMEOUT, ask)
        .await
        .ok()
        .flatten()
}

/// Collects the report.
///
/// # Errors
/// Only when the basics (memory, disk) cannot be read at all, which means the
/// agent is not running on a usable Linux server; retryable.
pub async fn collect(sources: &Sources) -> Result<Capabilities, AgentError> {
    let unreadable = |what: &str| {
        AgentError::new(ErrorCode::Internal, format!("cannot read {what}")).retryable()
    };
    let memory_bytes = read(&sources.proc_dir.join("meminfo"))
        .as_deref()
        .and_then(parse_mem_total)
        .ok_or_else(|| unreadable("memory size"))?;
    let disk = rustix::fs::statvfs(&sources.data_dir).map_err(|_| unreadable("disk size"))?;
    let docker = match &sources.docker_socket {
        Some(socket) => docker_info(socket).await.unwrap_or_default(),
        None => DockerFacts::default(),
    };
    let flag = |path: &Path| read(path).map(|s| s.trim().to_owned());
    // Unprivileged user namespaces: the kernel limit must be above zero and the
    // Debian/Ubuntu switch, where it exists, must be on.
    let userns_limit = flag(&sources.proc_dir.join("sys/user/max_user_namespaces"))
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > 0);
    let userns_switch = flag(
        &sources
            .proc_dir
            .join("sys/kernel/unprivileged_userns_clone"),
    )
    .is_none_or(|v| v == "1");
    Ok(Capabilities {
        agent_version: crate::AGENT_VERSION.to_owned(),
        cpu_cores: std::thread::available_parallelism()
            .map_or(1, |n| u32::try_from(n.get()).unwrap_or(u32::MAX)),
        memory_bytes,
        disk_total_bytes: disk.f_blocks.saturating_mul(disk.f_frsize),
        // What an unprivileged process may use (root's reserve is not ours).
        disk_free_bytes: disk.f_bavail.saturating_mul(disk.f_frsize),
        kernel: read(&sources.proc_dir.join("sys/kernel/osrelease"))
            .as_deref()
            .map(clip)
            .unwrap_or_default(),
        docker_version: docker.version,
        docker_mode: docker.mode,
        cgroup_v2: sources
            .sys_dir
            .join("fs/cgroup/cgroup.controllers")
            .exists(),
        seccomp: read(&sources.proc_dir.join("self/status"))
            .as_deref()
            .is_some_and(parse_seccomp),
        apparmor: flag(&sources.sys_dir.join("module/apparmor/parameters/enabled"))
            .is_some_and(|v| v == "Y"),
        user_namespaces: userns_limit && userns_switch,
        storage_quota: sources.tenants_root.as_deref().and_then(|root| {
            let mountinfo = read(&sources.proc_dir.join("self/mountinfo"))?;
            builtpanel_priv_helper::quota::detect(&mountinfo, root).map(|b| b.name().to_owned())
        }),
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn memory_is_read_from_meminfo() {
        let text = "MemTotal:        8131540 kB\nMemFree:          210124 kB\n";
        assert_eq!(parse_mem_total(text), Some(8_131_540 * 1024));
        assert_eq!(parse_mem_total("MemFree: 1 kB\n"), None);
        assert_eq!(parse_mem_total("MemTotal: lots kB\n"), None);
        assert_eq!(parse_mem_total(""), None);
    }

    #[test]
    fn docker_facts_come_from_the_daemon() {
        let rootless = r#"{"ServerVersion":"29.9.3","CgroupVersion":"2",
            "SecurityOptions":["name=seccomp,profile=builtin","name=rootless","name=cgroupns"]}"#;
        assert_eq!(
            parse_docker_info(rootless),
            DockerFacts {
                version: Some("29.9.3".into()),
                mode: Some(DockerMode::Rootless)
            }
        );
        let rootful = r#"{"ServerVersion":"29.9.3","SecurityOptions":["name=apparmor","name=seccomp,profile=builtin"]}"#;
        assert_eq!(parse_docker_info(rootful).mode, Some(DockerMode::Rootful));
        // "rootless" inside another option's value is not the rootless flag.
        let tricky =
            r#"{"ServerVersion":"1","SecurityOptions":["name=seccomp,profile=name=rootless-ish"]}"#;
        assert_eq!(parse_docker_info(tricky).mode, Some(DockerMode::Rootful));
        assert_eq!(parse_docker_info("not json"), DockerFacts::default());
        assert_eq!(parse_docker_info("{}"), DockerFacts::default());
        // Hostile text is clipped and cleaned.
        let long = format!(r#"{{"ServerVersion":"{}\u0007"}}"#, "9".repeat(500));
        assert_eq!(parse_docker_info(&long).version.unwrap().len(), MAX_TEXT);
    }

    #[test]
    fn http_answers_are_split_plain_and_chunked() {
        let plain = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"a\":1}";
        assert_eq!(http_body(plain), Some((200, "{\"a\":1}".into())));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3\r\n:1}\r\n0\r\n\r\n";
        assert_eq!(http_body(chunked), Some((200, "{\"a\":1}".into())));
        let denied = b"HTTP/1.1 403 Forbidden\r\n\r\nno";
        assert_eq!(http_body(denied).unwrap().0, 403);
        assert_eq!(http_body(b"garbage"), None);
        assert_eq!(
            http_body(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n"),
            None
        );
        assert_eq!(
            http_body(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nff\r\nshort"),
            None
        );
    }

    fn fixture(files: &[(&str, &str)]) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("builtpanel-caps-{}", uuid::Uuid::now_v7().simple()));
        for (path, text) in files {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        root
    }

    #[tokio::test]
    async fn a_report_is_built_from_kernel_files() {
        let root = fixture(&[
            ("proc/meminfo", "MemTotal:        4000000 kB\n"),
            ("proc/sys/kernel/osrelease", "6.8.0-31-generic\n"),
            ("proc/self/status", "Name:\tagent\nSeccomp:\t2\n"),
            ("proc/sys/user/max_user_namespaces", "63000\n"),
            ("proc/sys/kernel/unprivileged_userns_clone", "1\n"),
            ("sys/fs/cgroup/cgroup.controllers", "cpu memory pids\n"),
            ("sys/module/apparmor/parameters/enabled", "Y\n"),
        ]);
        let sources = Sources {
            data_dir: root.clone(),
            proc_dir: root.join("proc"),
            sys_dir: root.join("sys"),
            docker_socket: Some(root.join("no-docker.sock")),
            tenants_root: Some("/srv/tenants".into()),
        };
        let caps = collect(&sources).await.unwrap();
        assert_eq!(caps.memory_bytes, 4_000_000 * 1024);
        assert_eq!(caps.kernel, "6.8.0-31-generic");
        assert!(caps.cpu_cores >= 1);
        assert!(caps.disk_total_bytes > 0 && caps.disk_free_bytes <= caps.disk_total_bytes);
        assert!(caps.cgroup_v2 && caps.seccomp && caps.apparmor && caps.user_namespaces);
        // No daemon behind the socket: absent, not an error.
        assert_eq!((caps.docker_version, caps.docker_mode), (None, None));

        // The Debian switch turned off, AppArmor off, no cgroup v2, no seccomp.
        let bare = fixture(&[
            ("proc/meminfo", "MemTotal: 1 kB\n"),
            ("proc/sys/user/max_user_namespaces", "63000\n"),
            ("proc/sys/kernel/unprivileged_userns_clone", "0\n"),
            ("sys/module/apparmor/parameters/enabled", "N\n"),
            ("proc/self/status", "Name:\tagent\n"),
        ]);
        let caps = collect(&Sources {
            data_dir: bare.clone(),
            proc_dir: bare.join("proc"),
            sys_dir: bare.join("sys"),
            docker_socket: None,
            tenants_root: None,
        })
        .await
        .unwrap();
        assert!(!caps.cgroup_v2 && !caps.seccomp && !caps.apparmor && !caps.user_namespaces);
        assert_eq!(caps.kernel, "");

        // Storage quotas: only on XFS mounted with project quotas, below the tenants directory.
        let quota_root = fixture(&[
            ("proc/meminfo", "MemTotal: 1 kB\n"),
            (
                "proc/self/mountinfo",
                "22 1 252:1 / / rw - ext4 /dev/vda1 rw\n40 22 7:0 / /srv/tenants rw,relatime - xfs /dev/loop0 rw,prjquota\n",
            ),
        ]);
        let with = |tenants: &str| Sources {
            data_dir: quota_root.clone(),
            proc_dir: quota_root.join("proc"),
            sys_dir: quota_root.join("sys"),
            docker_socket: None,
            tenants_root: Some(tenants.into()),
        };
        assert_eq!(
            collect(&with("/srv/tenants"))
                .await
                .unwrap()
                .storage_quota
                .as_deref(),
            Some("xfs")
        );
        assert_eq!(
            collect(&with("/srv/other")).await.unwrap().storage_quota,
            None,
            "that is the ext4 root"
        );
        let mut no_root = with("/srv/tenants");
        no_root.tenants_root = None;
        assert_eq!(collect(&no_root).await.unwrap().storage_quota, None);
        let _ = std::fs::remove_dir_all(&quota_root);

        // Nothing readable at all is a retryable error.
        let empty = fixture(&[("proc/placeholder", "")]);
        let err = collect(&Sources {
            data_dir: empty.clone(),
            proc_dir: empty.join("proc"),
            sys_dir: empty.join("sys"),
            docker_socket: None,
            tenants_root: None,
        })
        .await
        .unwrap_err();
        assert!(err.retryable);
        for dir in [root, bare, empty] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[tokio::test]
    async fn the_docker_daemon_is_asked_over_its_socket() {
        let dir = fixture(&[("placeholder", "")]);
        let socket = dir.join("docker.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        tokio::spawn(async move {
            let (mut conn, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 512];
            let n = conn.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..n]).starts_with("GET /info HTTP/1.1\r\n"));
            let body = r#"{"ServerVersion":"29.9.3","SecurityOptions":["name=rootless"]}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            conn.write_all(reply.as_bytes()).await.unwrap();
        });
        let facts = docker_info(&socket).await.unwrap();
        assert_eq!(facts.version.as_deref(), Some("29.9.3"));
        assert_eq!(facts.mode, Some(DockerMode::Rootless));
        assert!(docker_info(&dir.join("missing.sock")).await.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
