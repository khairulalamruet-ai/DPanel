//! `collect_metrics` (T10.08): how busy the node and each of its containers are.
//!
//! The container figures come from Docker's statistics (which read the cgroups: CPU time,
//! memory, processes, network and block I/O), a few containers at a time; the node's from
//! `/proc` and the file system of the tenants' files. All are counters or levels: the
//! control plane makes rates from two readings. Nothing here starts a process.

use std::sync::Arc;

use builtpanel_node_protocol::{
    AgentError, ContainerMetrics, MAX_METRIC_CONTAINERS, NodeMetrics, Operation, Output,
};

use super::{
    sqlimport::failed,
    tenant::{TenantContext, docker_error},
};

fn number(text: &str) -> u64 {
    text.trim().parse().unwrap_or(0)
}

/// `(busy, total)` jiffies of all CPUs from the first line of `/proc/stat`, and the CPU count.
fn parse_cpu(stat: &str) -> (u64, u64, u32) {
    let mut busy_total = (0, 0);
    let mut cpus = 0u32;
    for line in stat.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("cpu") => {
                let v: Vec<u64> = parts.map(number).collect();
                // user nice system idle iowait irq softirq steal
                let idle = v.get(3).copied().unwrap_or(0) + v.get(4).copied().unwrap_or(0);
                let total: u64 = v.iter().take(8).sum();
                busy_total = (total.saturating_sub(idle), total);
            }
            Some(name) if name.starts_with("cpu") => cpus += 1,
            _ => {}
        }
    }
    (busy_total.0, busy_total.1, cpus.max(1))
}

fn kib(meminfo: &str, key: &str) -> u64 {
    meminfo
        .lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .and_then(|v| v.split_whitespace().next())
        .map_or(0, number)
        * 1024
}

fn parse_load(text: &str) -> [u32; 3] {
    let mut it = text
        .split_whitespace()
        .map(|v| v.parse::<f64>().unwrap_or(0.0));
    let mut next = || {
        let x = it.next().unwrap_or(0.0);
        // Rounded to hundredths; the cast cannot lose more than that.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let hundredths = (x.clamp(0.0, 1_000_000.0) * 100.0).round() as u32;
        hundredths
    };
    [next(), next(), next()]
}

fn node_metrics(disk_path: &std::path::Path) -> NodeMetrics {
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    let (busy, total, cpus) = parse_cpu(&read("/proc/stat"));
    let mem = read("/proc/meminfo");
    let (disk_total, disk_used) = rustix::fs::statvfs(disk_path).map_or((0, 0), |s| {
        let size = s.f_blocks.saturating_mul(s.f_frsize);
        (
            size,
            size.saturating_sub(s.f_bfree.saturating_mul(s.f_frsize)),
        )
    });
    NodeMetrics {
        cpus,
        cpu_busy_jiffies: busy,
        cpu_total_jiffies: total,
        memory_total_bytes: kib(&mem, "MemTotal"),
        memory_available_bytes: kib(&mem, "MemAvailable"),
        swap_total_bytes: kib(&mem, "SwapTotal"),
        swap_used_bytes: kib(&mem, "SwapTotal").saturating_sub(kib(&mem, "SwapFree")),
        load_x100: parse_load(&read("/proc/loadavg")),
        uptime_seconds: read("/proc/uptime").split('.').next().map_or(0, number),
        disk_total_bytes: disk_total,
        disk_used_bytes: disk_used,
    }
}

/// `collect_metrics`.
///
/// # Errors
/// [`AgentError`] when Docker cannot be asked (retryable).
pub async fn collect_metrics(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::CollectMetrics = op else {
        return Err(super::wrong_operation());
    };
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let disk_path = context.tenants_root.clone();
    let node = tokio::task::spawn_blocking(move || node_metrics(&disk_path))
        .await
        .map_err(|_| failed("the reading was cancelled").retryable())?;
    let mut containers: Vec<ContainerMetrics> = context
        .docker
        .usage_of_running()
        .await
        .map_err(|e| docker_error(&e))?
        .into_iter()
        .map(|(name, u)| ContainerMetrics {
            name,
            memory_bytes: u.memory_bytes,
            memory_limit_bytes: u.memory_limit_bytes,
            cpu_nanos: u.cpu_total_nanos,
            pids: u.pids,
            net_rx_bytes: u.net_rx_bytes,
            net_tx_bytes: u.net_tx_bytes,
            io_read_bytes: u.io_read_bytes,
            io_write_bytes: u.io_write_bytes,
        })
        .collect();
    containers.truncate(MAX_METRIC_CONTAINERS);
    Ok(Output::Metrics {
        at,
        node,
        containers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_files_are_read_into_numbers() {
        let stat = "cpu  100 5 50 800 20 3 2 0 0 0\ncpu0 50 2 25 400 10 1 1 0 0 0\ncpu1 50 3 25 400 10 2 1 0 0 0\nintr 1\n";
        assert_eq!(parse_cpu(stat), (100 + 5 + 50 + 3 + 2, 980, 2));
        let mem = "MemTotal:        2048 kB\nMemAvailable:    1024 kB\nSwapTotal:       512 kB\nSwapFree:        256 kB\n";
        assert_eq!(kib(mem, "MemTotal"), 2048 * 1024);
        assert_eq!(kib(mem, "SwapFree"), 256 * 1024);
        assert_eq!(kib(mem, "Nope"), 0);
        assert_eq!(parse_load("0.52 1.50 12.34 1/100 999\n"), [52, 150, 1234]);
        assert_eq!(parse_load(""), [0, 0, 0]);
    }
}
