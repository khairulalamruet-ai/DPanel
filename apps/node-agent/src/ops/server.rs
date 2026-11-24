//! The server tools (T10.15): what the node says about itself, and the four things an
//! administrator may do to it (time zone, swap, page cache, reboot), each asked of the
//! privileged helper as its own typed action.

use std::sync::Arc;

use builtpanel_node_protocol::{
    AgentError, Operation, Output, ServerInfo, SshAccess, SshAuthorizedKey,
};
use builtpanel_priv_helper::protocol::Action;

use super::{
    sqlimport::{failed, invalid},
    tenant::{TenantContext, ask_helper, ask_helper_data},
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

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 without padding (the form of an SSH fingerprint).
fn base64_nopad(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(char::from(B64[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

/// Standard base64 (padding allowed), or `None` for anything else.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = u32::try_from(B64.iter().position(|b| *b == c)?).ok()?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

/// `SHA256:...` of the key text of a public-key line, as `ssh-keygen -l` prints it.
fn fingerprint(blob: &str) -> Option<String> {
    use sha2::{Digest, Sha256};
    let raw = base64_decode(blob)?;
    Some(format!("SHA256:{}", base64_nopad(&Sha256::digest(raw))))
}

/// The type, key text and comment of a key line (options in front of the type are skipped).
fn key_parts(line: &str) -> Option<(String, String, String)> {
    const TYPES: [&str; 7] = [
        "ssh-ed25519",
        "ssh-rsa",
        "ecdsa-sha2-nistp256",
        "ecdsa-sha2-nistp384",
        "ecdsa-sha2-nistp521",
        "sk-ssh-ed25519@openssh.com",
        "sk-ecdsa-sha2-nistp256@openssh.com",
    ];
    let words: Vec<&str> = line.split_whitespace().collect();
    let i = words.iter().position(|w| TYPES.contains(w))?;
    let blob = words.get(i + 1)?;
    let comment = words.get(i + 2..).unwrap_or_default().join(" ");
    Some((words[i].to_owned(), (*blob).to_owned(), comment))
}

/// [`through_helper`] for the SSH changes: a failed one (refused, or rolled back) is not tried again
/// by itself, so the administrator sees the reason and decides.
async fn ssh_change(
    context: &TenantContext,
    action: Action,
    name: &str,
) -> Result<Output, AgentError> {
    through_helper(context, action, name)
        .await
        .map_err(|mut e| {
            e.retryable = false;
            e
        })
}

async fn helper_data(
    context: &TenantContext,
    action: Action,
) -> Result<Option<serde_json::Value>, AgentError> {
    let socket = context.helper_socket.clone();
    tokio::task::spawn_blocking(move || ask_helper_data(&socket, action))
        .await
        .map_err(|_| failed("the action was cancelled"))?
}

/// The keys the helper lists, as (fingerprint, key text, type, comment).
fn listed_keys(data: &serde_json::Value) -> Vec<(String, String, String, String)> {
    data["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| key_parts(l.as_str()?))
        .filter_map(|(kind, blob, comment)| Some((fingerprint(&blob)?, blob, kind, comment)))
        .collect()
}

/// `read_ssh_access`.
///
/// # Errors
/// [`AgentError`]: the helper failing.
pub async fn read_ssh_access(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::ReadSshAccess = op else {
        return Err(super::wrong_operation());
    };
    let data = helper_data(&context, Action::ReadSsh)
        .await?
        .ok_or_else(|| failed("the helper gave no answer"))?;
    let keys = listed_keys(&data)
        .into_iter()
        .map(|(fingerprint, _, kind, comment)| SshAuthorizedKey {
            kind,
            fingerprint,
            comment,
        })
        .collect();
    let config = tokio::task::spawn_blocking(|| {
        std::fs::read_to_string("/etc/ssh/sshd_config")
            .unwrap_or_default()
            .chars()
            .take(65_536)
            .collect::<String>()
    })
    .await
    .map_err(|_| failed("the reading was cancelled").retryable())?;
    let ports = data["ports"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| u16::try_from(p.as_u64()?).ok())
        .collect();
    Ok(Output::SshAccess {
        access: SshAccess {
            ports,
            root_login: data["root_login"].as_str().unwrap_or_default().to_owned(),
            password_authentication: data["password_authentication"].as_bool().unwrap_or(true),
            pubkey_authentication: data["pubkey_authentication"].as_bool().unwrap_or(true),
            keys,
            config,
        },
    })
}

/// `set_ssh_settings`: the helper checks the values again, and the result.
///
/// # Errors
/// [`AgentError`]: nothing to change, a value out of range, a lock-out, or SSH not coming back
/// (the old settings stay).
pub async fn set_ssh_settings(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::SetSshSettings {
        port,
        root_login,
        password_authentication,
        pubkey_authentication,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    if port.is_none()
        && root_login.is_none()
        && password_authentication.is_none()
        && pubkey_authentication.is_none()
    {
        return Err(invalid("nothing to change"));
    }
    ssh_change(
        &context,
        Action::SetSshSettings {
            port,
            root_login,
            password: password_authentication,
            pubkey: pubkey_authentication,
        },
        "set_ssh_settings",
    )
    .await
}

/// `add_ssh_key`.
///
/// # Errors
/// [`AgentError`]: a key that is not acceptable or is there already.
pub async fn add_ssh_key(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::AddSshKey { key } = op else {
        return Err(super::wrong_operation());
    };
    if key.is_empty() || key.len() > 8192 {
        return Err(invalid("the key is not usable"));
    }
    ssh_change(&context, Action::AddSshKey { key }, "add_ssh_key").await
}

/// `remove_ssh_key`: the fingerprint is looked up in the keys the helper lists now.
///
/// # Errors
/// [`AgentError`]: no key with that fingerprint, or the removal would lock everyone out.
pub async fn remove_ssh_key(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::RemoveSshKey { fingerprint } = op else {
        return Err(super::wrong_operation());
    };
    let data = helper_data(&context, Action::ReadSsh)
        .await?
        .ok_or_else(|| failed("the helper gave no answer"))?;
    let blob = listed_keys(&data)
        .into_iter()
        .find(|(f, ..)| *f == fingerprint)
        .map(|(_, blob, ..)| blob)
        .ok_or_else(|| invalid("there is no key with this fingerprint"))?;
    ssh_change(&context, Action::RemoveSshKey { blob }, "remove_ssh_key").await
}

/// `write_sshd_config`.
///
/// # Errors
/// [`AgentError`]: a text the SSH server does not accept, a lock-out, or SSH not coming back (the
/// old file stays).
pub async fn write_sshd_config(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::WriteSshdConfig { text } = op else {
        return Err(super::wrong_operation());
    };
    if text.trim().is_empty() || text.len() > 65_536 {
        return Err(invalid("the configuration is empty or too long"));
    }
    ssh_change(
        &context,
        Action::WriteSshdConfig { text },
        "write_sshd_config",
    )
    .await
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

/// `drop_swap`.
///
/// # Errors
/// [`AgentError`]: not enough free memory for what is in swap, or the helper failing.
pub async fn drop_swap(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::DropSwap = op else {
        return Err(super::wrong_operation());
    };
    through_helper(&context, Action::DropSwap, "drop_swap").await
}

/// `reboot_node`: the answer is given first, the machine goes down a few seconds later.
///
/// # Errors
/// [`AgentError`]: the helper failing.
pub async fn reboot_node(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    let Operation::RebootNode { hard } = op else {
        return Err(super::wrong_operation());
    };
    // The operation is answered before the reboot starts, or it would be sent again.
    let socket = context.helper_socket.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let _ =
            tokio::task::spawn_blocking(move || ask_helper(&socket, Action::Reboot { hard })).await;
    });
    Ok(Output::ServerActionDone {
        action: "reboot_node".into(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
    fn a_fingerprint_is_the_one_ssh_keygen_prints() {
        // `ssh-keygen -l -f` of this key prints SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU.
        let blob = "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
        assert_eq!(
            fingerprint(blob).as_deref(),
            Some("SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU")
        );
        assert_eq!(base64_nopad(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_nopad(b"fo"), "Zm8");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert!(base64_decode("a b").is_none());
        let (kind, _, comment) =
            key_parts(&format!("no-pty ssh-ed25519 {blob} me@laptop")).unwrap();
        assert_eq!(
            (kind.as_str(), comment.as_str()),
            ("ssh-ed25519", "me@laptop")
        );
        assert!(key_parts("# only a comment").is_none());
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
