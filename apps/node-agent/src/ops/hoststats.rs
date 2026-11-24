//! The machine's live numbers and its processes (Server: Resource usage, Process manager).
//!
//! Everything is read from `/proc` as the agent's own user (no shell, no helper): the readings are
//! plain text parsed by small pure functions. Only signalling a process goes through the root helper.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use builtpanel_node_protocol::{
    AgentError, DiskIo, DiskUsage, HostProcess, HostProcessDetail, HostSignal, NetIo, Operation,
    Output, ServerStats, SignalOutcome,
};
use builtpanel_priv_helper::protocol::Action;

use super::{sqlimport::failed, tenant::TenantContext};

/// Kernel clock ticks per second (`USER_HZ`: 100 on every Linux the panel supports).
const TICKS: f64 = 100.0;
/// How long the two readings of a rate are apart.
const SAMPLE: Duration = Duration::from_millis(300);
/// Most processes listed.
const MAX_PROCESSES: usize = 2000;
/// How long a process gets to end after SIGTERM.
const GRACE: Duration = Duration::from_secs(10);

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

// ----------------------------------------------------------------------------------- stats

/// `(busy, total)` ticks of every `cpu` line of `/proc/stat`: the first is all processors.
fn cpu_ticks(stat: &str) -> Vec<(u64, u64)> {
    stat.lines()
        .filter(|l| l.starts_with("cpu"))
        .map(|l| {
            let n: Vec<u64> = l
                .split_whitespace()
                .skip(1)
                .filter_map(|v| v.parse().ok())
                .collect();
            let total: u64 = n.iter().take(8).sum();
            // idle and iowait are the two quiet columns
            let quiet = n.get(3).copied().unwrap_or(0) + n.get(4).copied().unwrap_or(0);
            (total.saturating_sub(quiet), total)
        })
        .collect()
}

#[allow(clippy::cast_precision_loss)]
fn percent(before: (u64, u64), after: (u64, u64)) -> f64 {
    let total = after.1.saturating_sub(before.1);
    if total == 0 {
        return 0.0;
    }
    let busy = after.0.saturating_sub(before.0);
    (busy as f64 * 100.0 / total as f64).clamp(0.0, 100.0)
}

/// A share or a load as whole hundredths.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn x100(v: f64) -> u32 {
    (v * 100.0).round().clamp(0.0, f64::from(u32::MAX)) as u32
}

fn load(text: &str) -> [u32; 3] {
    let mut it = text
        .split_whitespace()
        .map(|v| x100(v.parse::<f64>().unwrap_or(0.0)));
    [
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    ]
}

fn meminfo(text: &str, key: &str) -> u64 {
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        * 1024
}

/// Whole disks and partitions of `/proc/diskstats` (not loop or RAM devices), counters since boot.
fn disk_io(text: &str) -> Vec<DiskIo> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let name = *f.get(2)?;
            if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("zram") {
                return None;
            }
            let n = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            Some(DiskIo {
                name: name.to_owned(),
                read_count: n(3),
                write_count: n(7),
                read_bytes: n(5) * 512,
                write_bytes: n(9) * 512,
                read_ms: n(6),
                write_ms: n(10),
            })
        })
        .collect()
}

fn net_io(text: &str) -> Vec<NetIo> {
    text.lines()
        .filter_map(|l| {
            let (name, rest) = l.split_once(':')?;
            let f: Vec<u64> = rest
                .split_whitespace()
                .map(|v| v.parse().unwrap_or(0))
                .collect();
            if f.len() < 11 {
                return None;
            }
            Some(NetIo {
                name: name.trim().to_owned(),
                bytes_received: f[0],
                packets_received: f[1],
                errors_in: f[2],
                bytes_sent: f[8],
                packets_sent: f[9],
                errors_out: f[10],
            })
        })
        .collect()
}

/// The device and file system type mounted on `/`.
fn root_mount(mounts: &str) -> (String, String) {
    mounts
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.get(1) == Some(&"/")).then(|| (f[0].to_owned(), f.get(2).unwrap_or(&"").to_string()))
        })
        .next_back()
        .unwrap_or_default()
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0))
}

fn take_stats() -> ServerStats {
    let before = cpu_ticks(&read("/proc/stat"));
    std::thread::sleep(SAMPLE);
    let after = cpu_ticks(&read("/proc/stat"));
    let cpu_percent = before
        .first()
        .zip(after.first())
        .map_or(0, |(b, a)| x100(percent(*b, *a)));
    let cpu_cores_percent = before
        .iter()
        .zip(&after)
        .skip(1)
        .map(|(b, a)| x100(percent(*b, *a)))
        .collect();
    let mem = read("/proc/meminfo");
    let total = meminfo(&mem, "MemTotal");
    let (device, filesystem) = root_mount(&read("/proc/self/mounts"));
    let disk = rustix::fs::statvfs("/").map_or(
        DiskUsage {
            device: device.clone(),
            filesystem: filesystem.clone(),
            total_bytes: 0,
            used_bytes: 0,
            free_bytes: 0,
        },
        |s| {
            let unit = s.f_frsize;
            DiskUsage {
                device: device.clone(),
                filesystem: filesystem.clone(),
                total_bytes: s.f_blocks.saturating_mul(unit),
                used_bytes: s.f_blocks.saturating_sub(s.f_bfree).saturating_mul(unit),
                free_bytes: s.f_bavail.saturating_mul(unit),
            }
        },
    );
    ServerStats {
        taken_at_ms: unix_ms(),
        load_x100: load(&read("/proc/loadavg")),
        cpu_percent_x100: cpu_percent,
        cpu_cores_percent_x100: cpu_cores_percent,
        memory_total_bytes: total,
        memory_used_bytes: total.saturating_sub(meminfo(&mem, "MemAvailable")),
        swap_total_bytes: meminfo(&mem, "SwapTotal"),
        swap_used_bytes: meminfo(&mem, "SwapTotal").saturating_sub(meminfo(&mem, "SwapFree")),
        disk,
        disk_io: disk_io(&read("/proc/diskstats")),
        net_io: net_io(&read("/proc/net/dev")),
    }
}

/// `report_server_stats`.
///
/// # Errors
/// [`AgentError`]: only when the reading is cancelled.
pub async fn report_server_stats(op: Operation) -> Result<Output, AgentError> {
    let Operation::ReportServerStats = op else {
        return Err(super::wrong_operation());
    };
    let stats = tokio::task::spawn_blocking(take_stats)
        .await
        .map_err(|_| failed("the reading was cancelled").retryable())?;
    Ok(Output::ServerStats {
        stats: Box::new(stats),
    })
}

// ----------------------------------------------------------------------------------- processes

/// What `/proc/<pid>/stat` says. The command name sits in parentheses and may hold spaces and
/// parentheses of its own, so the fields are counted from the last `)`.
#[derive(Debug, PartialEq, Eq)]
struct Stat {
    state: String,
    ppid: u32,
    ticks: u64,
    nice: i32,
    threads: u32,
    start_ticks: u64,
}

fn parse_stat(text: &str) -> Option<Stat> {
    let rest = &text[text.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    let n = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok());
    Some(Stat {
        state: (*f.first()?).to_owned(),
        ppid: u32::try_from(n(1)?).ok()?,
        ticks: n(11)? + n(12)?,
        nice: f.get(16)?.parse().ok()?,
        threads: u32::try_from(n(17)?).ok()?,
        start_ticks: n(19)?,
    })
}

/// `(name, uid, resident bytes)` from `/proc/<pid>/status`.
fn parse_status(text: &str) -> (String, u32, u64) {
    let field = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
            .map(str::trim)
    };
    (
        field("Name").unwrap_or_default().to_owned(),
        field("Uid")
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        field("VmRSS")
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024,
    )
}

/// The command line of `/proc/<pid>/cmdline` (arguments end with a NUL byte), clipped to `max` characters.
fn command_line(raw: &[u8], max: usize) -> String {
    let text: String = String::from_utf8_lossy(raw)
        .split('\0')
        .filter(|a| !a.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    text.chars().take(max).collect()
}

fn users() -> HashMap<u32, String> {
    read("/etc/passwd")
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            Some((f.get(2)?.parse().ok()?, (*f.first()?).to_owned()))
        })
        .collect()
}

fn pids() -> Vec<u32> {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .collect()
}

fn ticks_of(pid: u32) -> Option<u64> {
    parse_stat(&read(&format!("/proc/{pid}/stat"))).map(|s| s.ticks)
}

#[allow(clippy::cast_precision_loss)]
fn take_processes() -> Vec<HostProcess> {
    let first: HashMap<u32, u64> = pids()
        .into_iter()
        .filter_map(|p| Some((p, ticks_of(p)?)))
        .collect();
    let started = Instant::now();
    std::thread::sleep(SAMPLE);
    let elapsed = started.elapsed().as_secs_f64().max(0.001);
    let names = users();
    let total_mem = meminfo(&read("/proc/meminfo"), "MemTotal").max(1);
    let mut items: Vec<HostProcess> = pids()
        .into_iter()
        .filter_map(|pid| {
            let stat = parse_stat(&read(&format!("/proc/{pid}/stat")))?;
            let (name, uid, rss) = parse_status(&read(&format!("/proc/{pid}/status")));
            let raw = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            let mut command = command_line(&raw, 300);
            if command.is_empty() {
                command = format!("[{name}]");
            }
            let delta = stat
                .ticks
                .saturating_sub(first.get(&pid).copied().unwrap_or(stat.ticks));
            Some(HostProcess {
                pid,
                ppid: stat.ppid,
                owner: names.get(&uid).cloned().unwrap_or_else(|| uid.to_string()),
                uid,
                priority: stat.nice,
                state: stat.state,
                cpu_percent_x100: x100((delta as f64 / TICKS) * 100.0 / elapsed),
                memory_percent_x100: x100(rss as f64 * 100.0 / total_mem as f64),
                memory_bytes: rss,
                command,
            })
        })
        .collect();
    items.sort_by(|a, b| {
        b.cpu_percent_x100
            .cmp(&a.cpu_percent_x100)
            .then(b.memory_bytes.cmp(&a.memory_bytes))
    });
    items.truncate(MAX_PROCESSES);
    items
}

/// `list_host_processes`.
///
/// # Errors
/// [`AgentError`]: only when the reading is cancelled.
pub async fn list_host_processes(op: Operation) -> Result<Output, AgentError> {
    let Operation::ListHostProcesses = op else {
        return Err(super::wrong_operation());
    };
    let items = tokio::task::spawn_blocking(take_processes)
        .await
        .map_err(|_| failed("the reading was cancelled").retryable())?;
    Ok(Output::HostProcesses { items })
}

fn inspect(pid: u32) -> Option<HostProcessDetail> {
    let stat = parse_stat(&read(&format!("/proc/{pid}/stat")))?;
    let (name, uid, rss) = parse_status(&read(&format!("/proc/{pid}/status")));
    let uptime: f64 = read("/proc/uptime")
        .split_whitespace()
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let age_seconds = (uptime - stat.start_ticks as f64 / TICKS).max(0.0) as u64;
    Some(HostProcessDetail {
        pid,
        ppid: stat.ppid,
        owner: users()
            .get(&uid)
            .cloned()
            .unwrap_or_else(|| uid.to_string()),
        name,
        state: stat.state,
        threads: stat.threads,
        memory_bytes: rss,
        age_seconds,
        priority: stat.nice,
        command_line: command_line(&raw, 4000),
        cgroup: read(&format!("/proc/{pid}/cgroup")).trim().to_owned(),
    })
}

/// `inspect_host_process`.
///
/// # Errors
/// [`AgentError`]: no such process (not retryable).
pub async fn inspect_host_process(op: Operation) -> Result<Output, AgentError> {
    let Operation::InspectHostProcess { pid } = op else {
        return Err(super::wrong_operation());
    };
    let detail = tokio::task::spawn_blocking(move || inspect(pid))
        .await
        .map_err(|_| failed("the reading was cancelled").retryable())?
        .ok_or_else(|| failed("there is no such process"))?;
    Ok(Output::HostProcessDetail { detail })
}

/// Whether this process may never be signalled from the panel: the first process, kernel threads, this
/// agent and its parents, and the panel's own programs.
fn protected(pid: u32) -> bool {
    if pid < 2 {
        return true;
    }
    let Some(stat) = parse_stat(&read(&format!("/proc/{pid}/stat"))) else {
        return false;
    };
    if stat.ppid == 2 {
        return true; // a kernel thread
    }
    let (name, _, _) = parse_status(&read(&format!("/proc/{pid}/status")));
    if name.starts_with("builtpanel-") {
        return true;
    }
    let mut at = std::process::id();
    for _ in 0..32 {
        if at == pid {
            return true;
        }
        match parse_stat(&read(&format!("/proc/{at}/stat"))) {
            Some(s) if s.ppid > 1 => at = s.ppid,
            _ => break,
        }
    }
    false
}

fn alive(pid: u32) -> bool {
    parse_stat(&read(&format!("/proc/{pid}/stat"))).is_some_and(|s| s.state != "Z")
}

/// `signal_host_process`: SIGTERM, and SIGKILL after ten seconds if it is still there; or SIGKILL at once.
///
/// # Errors
/// [`AgentError`]: a process that may not be signalled (not retryable), or the helper failing.
pub async fn signal_host_process(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SignalHostProcess { pid, signal } = op else {
        return Err(super::wrong_operation());
    };
    if tokio::task::spawn_blocking(move || protected(pid))
        .await
        .unwrap_or(true)
    {
        return Err(failed("this process may not be signalled from the panel"));
    }
    if !alive(pid) {
        return Ok(Output::ProcessSignalled {
            pid,
            outcome: SignalOutcome::NotFound,
        });
    }
    let send = |kill: bool| {
        let context = context.clone();
        async move {
            super::server::through_helper(&context, Action::SignalHost { pid, kill }, "signal_host")
                .await
        }
    };
    if signal == HostSignal::Kill {
        send(true).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        return Ok(Output::ProcessSignalled {
            pid,
            outcome: if alive(pid) {
                SignalOutcome::StillRunning
            } else {
                SignalOutcome::Killed
            },
        });
    }
    send(false).await?;
    let deadline = Instant::now() + GRACE;
    while Instant::now() < deadline {
        if !alive(pid) {
            return Ok(Output::ProcessSignalled {
                pid,
                outcome: SignalOutcome::Exited,
            });
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    send(true).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(Output::ProcessSignalled {
        pid,
        outcome: if alive(pid) {
            SignalOutcome::StillRunning
        } else {
            SignalOutcome::Killed
        },
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_command_name_may_hold_spaces_and_parentheses() {
        let line = "1234 (my (odd) name) S 99 1 1 0 -1 4194560 100 0 0 0 150 50 0 0 20 5 3 0 777 1000 250 18446744073709551615";
        let s = parse_stat(line).unwrap();
        assert_eq!(
            s,
            Stat {
                state: "S".into(),
                ppid: 99,
                ticks: 200,
                nice: 5,
                threads: 3,
                start_ticks: 777,
            }
        );
        assert!(parse_stat("garbage").is_none());
    }

    #[test]
    fn status_and_command_line_are_read() {
        let status = "Name:\tmariadbd\nUid:\t100998\t100998\t100998\t100998\nVmRSS:\t  2048 kB\n";
        assert_eq!(
            parse_status(status),
            ("mariadbd".into(), 100_998, 2_097_152)
        );
        assert_eq!(
            command_line(b"/usr/bin/x\0--flag\0value\0", 300),
            "/usr/bin/x --flag value"
        );
        assert_eq!(command_line(b"abcdef\0", 3), "abc");
        assert_eq!(command_line(b"", 10), "");
    }

    #[test]
    fn processor_use_comes_from_two_readings() {
        let a = cpu_ticks("cpu  100 0 100 800 0 0 0 0\ncpu0 50 0 50 400 0 0 0 0\n");
        let b = cpu_ticks("cpu  150 0 150 900 0 0 0 0\ncpu0 100 0 50 450 0 0 0 0\n");
        assert_eq!(a[0], (200, 1000));
        assert!((percent(a[0], b[0]) - 50.0).abs() < 1e-9);
        assert!((percent(a[1], b[1]) - 50.0).abs() < 1e-9);
        assert!(percent(a[0], a[0]).abs() < 1e-9, "no time passed");
    }

    #[test]
    fn counters_are_read_from_the_kernel_tables() {
        let disks = disk_io(
            "   8       0 vda 15441400 100 2000 6571170 1020260657 5 4000 872426104 0 0 0\n   7       0 loop0 1 0 0 0 0 0 0 0 0 0 0\n",
        );
        assert_eq!(disks.len(), 1);
        assert_eq!(
            (
                disks[0].read_count,
                disks[0].write_count,
                disks[0].read_bytes,
                disks[0].write_bytes
            ),
            (15_441_400, 1_020_260_657, 2000 * 512, 4000 * 512)
        );
        assert_eq!(
            (disks[0].read_ms, disks[0].write_ms),
            (6_571_170, 872_426_104)
        );
        let net = net_io(
            "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n  eth0: 68020 212 1 0 0 0 0 0 32430 45 2 0 0 0 0 0\n",
        );
        assert_eq!(net.len(), 1);
        assert_eq!(
            (
                net[0].name.as_str(),
                net[0].bytes_received,
                net[0].bytes_sent,
                net[0].errors_in,
                net[0].errors_out
            ),
            ("eth0", 68_020, 32_430, 1, 2)
        );
        assert_eq!(load("0.85 1.24 1.30 1/200 999"), [85, 124, 130]);
        assert_eq!(
            root_mount("proc /proc proc rw 0 0\n/dev/vda1 / ext4 rw 0 0\n"),
            ("/dev/vda1".to_owned(), "ext4".to_owned())
        );
    }

    #[test]
    fn this_machine_can_be_read() {
        // Whatever the machine, the readings come back in the right shape.
        if !std::path::Path::new("/proc/self/stat").exists() {
            return;
        }
        let items = take_processes();
        assert!(items.iter().any(|p| p.pid == std::process::id()));
        assert!(protected(1) && protected(std::process::id()));
        assert!(inspect(std::process::id()).is_some());
        let stats = take_stats();
        assert!(stats.memory_total_bytes > 0 && !stats.cpu_cores_percent_x100.is_empty());
    }
}
