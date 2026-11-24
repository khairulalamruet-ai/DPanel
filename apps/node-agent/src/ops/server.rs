//! The server tools (T10.15): what the node says about itself, and the four things an
//! administrator may do to it (time zone, swap, page cache, reboot), each asked of the
//! privileged helper as its own typed action.

use std::sync::Arc;

use builtpanel_node_protocol::{AgentError, Operation, Output, ServerInfo};
use builtpanel_priv_helper::protocol::Action;

use super::{
    sqlimport::{failed, invalid},
    tenant::{TenantContext, ask_helper},
};

/// The SSH server settings that matter for safety, lower-case names.
const SSH_KEYS: [&str; 10] = [
    "port",
    "permitrootlogin",
    "passwordauthentication",
    "pubkeyauthentication",
    "kbdinteractiveauthentication",
    "permitemptypasswords",
    "x11forwarding",
    "maxauthtries",
    "allowusers",
    "allowgroups",
];

/// The effective values of [`SSH_KEYS`] in an `sshd_config` text: the first value of a setting
/// wins, as in sshd; `Include` lines are followed through `include` (given the pattern, it
/// returns the texts of the files).
fn parse_ssh(
    text: &str,
    include: &dyn Fn(&str) -> Vec<String>,
    found: &mut Vec<(String, String)>,
    depth: u8,
) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once(char::is_whitespace)
            .map_or((line, ""), |(k, v)| (k, v.trim()));
        let key = key.to_ascii_lowercase();
        // A `Match` block's settings are for some users or addresses only.
        if key == "match" {
            return;
        }
        if key == "include" && depth < 3 {
            for file in include(value) {
                parse_ssh(&file, include, found, depth + 1);
            }
            continue;
        }
        if SSH_KEYS.contains(&key.as_str()) && !found.iter().any(|(k, _)| *k == key) {
            found.push((key, value.chars().take(200).collect()));
        }
    }
}

fn read_ssh() -> Vec<(String, String)> {
    let include = |pattern: &str| -> Vec<String> {
        // Only the usual drop-in folder is followed.
        if pattern.trim() != "/etc/ssh/sshd_config.d/*.conf" {
            return Vec::new();
        }
        let mut names: Vec<_> = std::fs::read_dir("/etc/ssh/sshd_config.d")
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "conf"))
            .collect();
        names.sort();
        names
            .iter()
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .collect()
    };
    let mut found = Vec::new();
    if let Ok(text) = std::fs::read_to_string("/etc/ssh/sshd_config") {
        parse_ssh(&text, &include, &mut found, 0);
    }
    found
}

fn word(text: &str, key: &str) -> String {
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
        .map(|v| v.trim().trim_matches('"').to_owned())
        .unwrap_or_default()
}

fn kib(meminfo: &str, key: &str) -> u64 {
    meminfo
        .lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        * 1024
}

fn timezone() -> String {
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| {
            p.to_str()
                .and_then(|s| s.split_once("zoneinfo/"))
                .map(|(_, zone)| zone.to_owned())
        })
        .or_else(|| {
            std::fs::read_to_string("/etc/timezone")
                .ok()
                .map(|t| t.trim().to_owned())
        })
        .unwrap_or_default()
}

/// `report_server_info`.
///
/// # Errors
/// [`AgentError`]: none in practice.
pub async fn report_server_info(op: Operation) -> Result<Output, AgentError> {
    let Operation::ReportServerInfo = op else {
        return Err(super::wrong_operation());
    };
    let info = tokio::task::spawn_blocking(|| {
        let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
        let mem = read("/proc/meminfo");
        ServerInfo {
            hostname: read("/etc/hostname").trim().to_owned(),
            os: word(&read("/etc/os-release"), "PRETTY_NAME"),
            kernel: read("/proc/sys/kernel/osrelease").trim().to_owned(),
            timezone: timezone(),
            uptime_seconds: read("/proc/uptime")
                .split('.')
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            memory_total_bytes: kib(&mem, "MemTotal"),
            memory_available_bytes: kib(&mem, "MemAvailable"),
            swap_total_bytes: kib(&mem, "SwapTotal"),
            swap_used_bytes: kib(&mem, "SwapTotal").saturating_sub(kib(&mem, "SwapFree")),
            ssh: read_ssh(),
        }
    })
    .await
    .map_err(|_| failed("the reading was cancelled").retryable())?;
    Ok(Output::ServerInfo { info })
}

async fn through_helper(
    context: &TenantContext,
    action: Action,
    name: &str,
) -> Result<Output, AgentError> {
    let socket = context.helper_socket.clone();
    tokio::task::spawn_blocking(move || ask_helper(&socket, action))
        .await
        .map_err(|_| failed("the action was cancelled"))??;
    Ok(Output::ServerActionDone {
        action: name.to_owned(),
    })
}

/// `set_timezone`.
///
/// # Errors
/// [`AgentError`]: a zone that is not one of the machine's, or the helper failing.
pub async fn set_timezone(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SetTimezone { zone } = op else {
        return Err(super::wrong_operation());
    };
    if zone.is_empty() || zone.len() > 64 {
        return Err(invalid("the time zone is not usable"));
    }
    through_helper(&context, Action::SetTimezone { zone }, "set_timezone").await
}

/// `set_swap`.
///
/// # Errors
/// [`AgentError`]: a size out of range, or the helper failing.
pub async fn set_swap(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::SetSwap { megabytes } = op else {
        return Err(super::wrong_operation());
    };
    if megabytes > builtpanel_priv_helper::servertools::MAX_SWAP_MB {
        return Err(invalid("the swap size is out of range"));
    }
    through_helper(&context, Action::SetSwap { megabytes }, "set_swap").await
}

/// `set_firewall_rules`: the policy is checked here, and again by the helper.
///
/// # Errors
/// [`AgentError`]: a policy that is not valid (not retryable), the helper or the packet filter
/// refusing (the old rules stay in force).
pub async fn set_firewall_rules(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SetFirewallRules { policy } = op else {
        return Err(super::wrong_operation());
    };
    let policy = policy.checked().map_err(|m| invalid(&m))?;
    let rules = builtpanel_priv_helper::nodefw::rule_count(&policy);
    through_helper(
        &context,
        Action::SetNodeFirewall { policy },
        "set_firewall_rules",
    )
    .await?;
    Ok(Output::FirewallSet { rules })
}

/// `drop_caches`.
///
/// # Errors
/// [`AgentError`]: the helper failing.
pub async fn drop_caches(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::DropCaches = op else {
        return Err(super::wrong_operation());
    };
    through_helper(&context, Action::DropCaches, "drop_caches").await
}

/// `reboot_node`: the answer is given first, the machine goes down a few seconds later.
///
/// # Errors
/// [`AgentError`]: the helper failing.
pub async fn reboot_node(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::RebootNode = op else {
        return Err(super::wrong_operation());
    };
    // The operation is answered before the reboot starts, or it would be sent again.
    let socket = context.helper_socket.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let _ = tokio::task::spawn_blocking(move || ask_helper(&socket, Action::Reboot)).await;
    });
    Ok(Output::ServerActionDone {
        action: "reboot_node".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_value_wins_includes_are_followed_and_match_blocks_are_left() {
        let main = "Include /etc/ssh/sshd_config.d/*.conf\n# comment\nPort 2200\nPermitRootLogin yes\nPasswordAuthentication yes\nUseDNS no\nMatch User bob\n  PasswordAuthentication no\n  PermitRootLogin no\n";
        let dropin = "PermitRootLogin prohibit-password\nX11Forwarding no\n".to_owned();
        let mut found = Vec::new();
        parse_ssh(main, &|_| vec![dropin.clone()], &mut found, 0);
        let get = |k: &str| found.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("permitrootlogin"), Some("prohibit-password"));
        assert_eq!(get("passwordauthentication"), Some("yes"));
        assert_eq!(get("port"), Some("2200"));
        assert_eq!(get("x11forwarding"), Some("no"));
        assert_eq!(get("usedns"), None);
    }

    #[test]
    fn os_release_and_meminfo_are_read() {
        assert_eq!(
            word(
                "NAME=\"X\"\nPRETTY_NAME=\"Debian GNU/Linux 12\"\n",
                "PRETTY_NAME"
            ),
            "Debian GNU/Linux 12"
        );
        assert_eq!(kib("SwapTotal:  10 kB\n", "SwapTotal"), 10 * 1024);
    }
}
