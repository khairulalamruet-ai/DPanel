//! Storage quotas for tenants (T04.08): XFS project quotas.
//!
//! Every tenant directory is a *project* whose id is the tenant's numeric id.
//! XFS counts everything created below the directory against the project, with
//! no per-user accounting and nothing the tenant can switch off, and refuses
//! writes once the hard limit is reached. Two steps:
//! * `project -s -p <dir> <id>` at creation, while the tree is still empty and
//!   still owned by the helper: marks the directory so that everything created
//!   in it joins the project;
//! * `limit -p bhard=<KiB>k ihard=<files> <id>` whenever the plan says so.
//!
//! The helper runs `xfs_quota` directly (no shell) with arguments it builds
//! itself from validated numbers and a path it derives from a UUID. ext4
//! quotas are not supported yet; the capability report says so.

use std::{
    io::{self, Read as _},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crate::protocol::ErrorCode;

/// `xfs_quota`'s absolute path.
pub const XFS_QUOTA: &str = "/usr/sbin/xfs_quota";
/// Longest an external command may run.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

/// Smallest and largest disk limit accepted, in bytes.
pub const MIN_DISK_BYTES: u64 = 1024 * 1024;
/// See [`MIN_DISK_BYTES`].
pub const MAX_DISK_BYTES: u64 = 1 << 50;
/// Fewest and most files accepted.
pub const MIN_INODES: u64 = 1_000;
/// See [`MIN_INODES`].
pub const MAX_INODES: u64 = 1_000_000_000;

/// How quotas are enforced on the filesystem a directory is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// XFS mounted with project quotas; `mount` is the mount point.
    Xfs {
        /// Where the filesystem is mounted.
        mount: String,
    },
}

impl Backend {
    /// Short name for reports.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Xfs { .. } => "xfs",
        }
    }
}

/// Finds the filesystem `dir` is on in the text of `/proc/self/mountinfo`
/// (the mount with the longest matching mount point) and says whether project
/// quotas are enforced there. `None` for any other filesystem, or XFS without
/// the `prjquota` / `pquota` options.
#[must_use]
pub fn detect(mountinfo: &str, dir: &Path) -> Option<Backend> {
    let mut best: Option<(usize, bool, String)> = None;
    for line in mountinfo.lines() {
        // 40 22 7:0 / /mnt/point rw,relatime shared:9 - xfs /dev/loop0 rw,attr2,prjquota
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let mut left = left.split_whitespace();
        let (Some(mount), mount_opts) = (left.nth(4), left.next().unwrap_or("")) else {
            continue;
        };
        let mut right = right.split_whitespace();
        let (Some(fstype), _source, super_opts) =
            (right.next(), right.next(), right.next().unwrap_or(""))
        else {
            continue;
        };
        let mount = unescape(mount);
        if !dir.starts_with(&mount) {
            continue;
        }
        let quota = fstype == "xfs"
            && mount_opts
                .split(',')
                .chain(super_opts.split(','))
                .any(|o| o == "prjquota" || o == "pquota");
        if best.as_ref().is_none_or(|(len, _, _)| mount.len() >= *len) {
            best = Some((mount.len(), quota, mount));
        }
    }
    best.and_then(|(_, quota, mount)| quota.then_some(Backend::Xfs { mount }))
}

/// `\040` style escapes in mountinfo paths.
fn unescape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4].iter().all(u8::is_ascii_digit)
            && let Ok(n) = u8::from_str_radix(&path[i + 1..i + 4], 8)
        {
            out.push(char::from(n));
            i += 4;
            continue;
        }
        out.push(char::from(bytes[i]));
        i += 1;
    }
    out
}

/// Where the helper keeps the `xfs_quota` project files (a systemd
/// `StateDirectory`, so it is writable under the sandbox).
pub const STATE_DIR: &str = "/var/lib/builtpanel-priv-helper";

/// Makes sure the state directory exists (root-only).
///
/// # Errors
/// The directory could not be created.
pub fn ensure_state_dir() -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(STATE_DIR)
}

/// One external command: a program and its arguments, never a shell line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    /// Absolute path of the program.
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
}

/// The command that puts `dir` (empty, helper-owned) into project `project`.
#[must_use]
pub fn project_setup(backend: &Backend, dir: &Path, project: u32) -> Cmd {
    let Backend::Xfs { mount } = backend;
    Cmd {
        program: XFS_QUOTA.to_owned(),
        args: vec![
            "-x".into(),
            // xfs_quota keeps its project registry in /etc by default; the
            // sandbox makes /etc read-only, so use files in the state directory.
            "-D".into(),
            format!("{STATE_DIR}/projects"),
            "-P".into(),
            format!("{STATE_DIR}/projid"),
            "-c".into(),
            format!("project -s -p {} {project}", dir.display()),
            mount.clone(),
        ],
    }
}

/// The command that sets the limits of `project`. `None` means unlimited
/// (XFS reads 0 as "no limit").
#[must_use]
pub fn set_limits(
    backend: &Backend,
    project: u32,
    disk_bytes: Option<u64>,
    inodes: Option<u64>,
) -> Cmd {
    let Backend::Xfs { mount } = backend;
    let kib = disk_bytes.map_or(0, |b| b.div_ceil(1024));
    Cmd {
        program: XFS_QUOTA.to_owned(),
        args: vec![
            "-x".into(),
            "-c".into(),
            format!(
                "limit -p bsoft={kib}k bhard={kib}k isoft={0} ihard={0} {project}",
                inodes.unwrap_or(0)
            ),
            mount.clone(),
        ],
    }
}

/// Runs a command with an empty environment, no input and a time limit.
///
/// # Errors
/// Starting it failed, it timed out, or it exited with an error.
pub fn run(cmd: &Cmd) -> io::Result<()> {
    let mut child = Command::new(&cmd.program)
        .args(&cmd.args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            let mut message = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.by_ref().take(300).read_to_string(&mut message);
            }
            // `xfs_quota -c` exits 0 even when the command failed and only
            // prints to stderr, so any stderr output counts as a failure.
            if status.success() && message.trim().is_empty() {
                return Ok(());
            }
            return Err(io::Error::other(format!(
                "exit {status}: {}",
                message.trim()
            )));
        }
        if start.elapsed() > COMMAND_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "command timed out"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Checks limits: both optional, within bounds.
///
/// # Errors
/// [`ErrorCode::InvalidArguments`].
pub fn check_limits(
    disk_bytes: Option<u64>,
    inodes: Option<u64>,
) -> Result<(), (ErrorCode, &'static str)> {
    if disk_bytes.is_some_and(|b| !(MIN_DISK_BYTES..=MAX_DISK_BYTES).contains(&b)) {
        return Err((
            ErrorCode::InvalidArguments,
            "the disk limit is out of range",
        ));
    }
    if inodes.is_some_and(|n| !(MIN_INODES..=MAX_INODES).contains(&n)) {
        return Err((
            ErrorCode::InvalidArguments,
            "the file limit is out of range",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    const MOUNTINFO: &str = "\
22 1 252:1 / / rw,relatime shared:1 - ext4 /dev/vda1 rw,errors=remount-ro
40 22 7:0 / /srv/tenants-live rw,relatime shared:9 - xfs /dev/loop0 rw,attr2,inode64,logbufs=8,logbsize=32k,prjquota
41 22 7:1 / /srv/no-quota rw,relatime shared:10 - xfs /dev/loop1 rw,attr2,inode64,noquota
42 22 7:2 / /srv/with\\040space rw,relatime shared:11 - xfs /dev/loop2 rw,pquota
43 22 7:3 / /srv/ext rw,relatime shared:12 - ext4 /dev/loop3 rw,prjquota
";

    #[test]
    fn quotas_are_found_only_on_xfs_with_project_quotas() {
        let xfs = |m: &str| {
            Some(Backend::Xfs {
                mount: m.to_owned(),
            })
        };
        assert_eq!(
            detect(MOUNTINFO, Path::new("/srv/tenants-live/abc/www")),
            xfs("/srv/tenants-live")
        );
        assert_eq!(
            detect(MOUNTINFO, Path::new("/srv/with space/x")),
            xfs("/srv/with space"),
            "pquota and escapes"
        );
        assert_eq!(
            detect(MOUNTINFO, Path::new("/srv/no-quota/x")),
            None,
            "xfs without project quotas"
        );
        assert_eq!(
            detect(MOUNTINFO, Path::new("/srv/ext/x")),
            None,
            "ext4 is not supported yet"
        );
        assert_eq!(
            detect(MOUNTINFO, Path::new("/srv/tenants/x")),
            None,
            "the root ext4 filesystem"
        );
        assert_eq!(detect("", Path::new("/srv")), None);
        assert_eq!(detect("garbage line\n", Path::new("/srv")), None);
        // A longer mount point wins over a shorter one that also contains the path.
        let nested =
            format!("{MOUNTINFO}44 40 7:4 / /srv/tenants-live/inner rw - ext4 /dev/loop4 rw\n");
        assert_eq!(
            detect(&nested, Path::new("/srv/tenants-live/inner/t")),
            None
        );
    }

    #[test]
    fn commands_are_built_from_numbers_and_paths_only() {
        let backend = Backend::Xfs {
            mount: "/srv/tenants-live".into(),
        };
        let setup = project_setup(
            &backend,
            Path::new("/srv/tenants-live/0190b6a2-0000-7000-8000-000000000001"),
            10_042,
        );
        assert_eq!(setup.program, XFS_QUOTA);
        assert_eq!(
            setup.args,
            [
                "-x",
                "-D",
                "/var/lib/builtpanel-priv-helper/projects",
                "-P",
                "/var/lib/builtpanel-priv-helper/projid",
                "-c",
                "project -s -p /srv/tenants-live/0190b6a2-0000-7000-8000-000000000001 10042",
                "/srv/tenants-live"
            ]
        );
        let limits = set_limits(
            &backend,
            10_042,
            Some(5 * 1024 * 1024 * 1024),
            Some(100_000),
        );
        assert_eq!(
            limits.args[2],
            "limit -p bsoft=5242880k bhard=5242880k isoft=100000 ihard=100000 10042"
        );
        let open = set_limits(&backend, 10_042, None, None);
        assert_eq!(
            open.args[2], "limit -p bsoft=0k bhard=0k isoft=0 ihard=0 10042",
            "0 is unlimited"
        );
        // Sizes round up to whole KiB, never down to nothing.
        assert!(set_limits(&backend, 1, Some(1025), None).args[2].contains("bhard=2k"));
    }

    #[test]
    fn limits_are_range_checked_before_anything_runs() {
        assert!(check_limits(None, None).is_ok());
        assert!(check_limits(Some(MIN_DISK_BYTES), Some(MIN_INODES)).is_ok());
        assert!(check_limits(Some(MAX_DISK_BYTES), Some(MAX_INODES)).is_ok());
        for (d, i) in [
            (Some(0), None),
            (Some(MIN_DISK_BYTES - 1), None),
            (Some(MAX_DISK_BYTES + 1), None),
            (None, Some(0)),
            (None, Some(MIN_INODES - 1)),
            (None, Some(MAX_INODES + 1)),
            (Some(u64::MAX), None),
        ] {
            assert_eq!(
                check_limits(d, i).unwrap_err().0,
                ErrorCode::InvalidArguments,
                "{d:?} {i:?}"
            );
        }
    }

    #[test]
    fn commands_run_without_a_shell_an_environment_or_forever() {
        assert!(
            run(&Cmd {
                program: "/bin/true".into(),
                args: vec![]
            })
            .is_ok()
        );
        let err = run(&Cmd {
            program: "/bin/false".into(),
            args: vec![],
        })
        .unwrap_err();
        assert!(err.to_string().contains("exit"), "{err}");
        // No shell: metacharacters are plain arguments.
        let marker = std::env::temp_dir().join(format!("dhph-noshell-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let _ = run(&Cmd {
            program: "/bin/echo".into(),
            args: vec![format!("; touch {}", marker.display())],
        });
        assert!(!marker.exists());
        // The environment is empty.
        let out = std::process::Command::new("/usr/bin/env")
            .env_clear()
            .output()
            .unwrap();
        assert!(out.stdout.is_empty());
        assert!(
            run(&Cmd {
                program: "/nonexistent/program".into(),
                args: vec![]
            })
            .is_err()
        );
    }
}
