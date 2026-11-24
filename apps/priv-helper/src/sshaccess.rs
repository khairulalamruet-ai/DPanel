//! SSH access (System menu): the SSH server's few safety settings, the keys that may log in as
//! root, and the whole `sshd_config` text.
//!
//! The changes are the kind that can lock an administrator out, so each one is made like this:
//!
//! 1. what is there is kept (the old drop-in, the old `sshd_config`, the old key file);
//! 2. the new text is written beside it and checked with `sshd -t`, and `sshd -T` must report the
//!    values that were asked for (a setting in the main file that wins over the drop-in is caught
//!    here);
//! 3. a change that would leave nobody able to log in (no password, and no key or no key login) is
//!    refused before anything is written;
//! 4. SSH is reloaded and must really listen on its port, or the old text is put back.
//!
//! Every command is fixed; the only caller-written values are a port number, a choice from a short
//! list, a public-key line (checked character by character, written through standard input) and the
//! configuration text (written through standard input, never part of a command line).

use std::{
    net::{Ipv4Addr, SocketAddr, TcpStream},
    path::Path,
    time::Duration,
};

use crate::{
    protocol::ErrorCode,
    servertools::{Env, run, run_io},
};

type Failure = (ErrorCode, &'static str);

/// The longest `sshd_config` text that is accepted, in bytes.
pub const MAX_CONFIG_BYTES: usize = 65_536;
/// Most keys the key file may hold.
pub const MAX_KEYS: usize = 200;
const MAX_KEY_LINE: usize = 8192;

/// The public-key types that are accepted.
const KEY_TYPES: [&str; 7] = [
    "ssh-ed25519",
    "ssh-rsa",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

/// Ports other services of the panel use: SSH must not be moved onto them.
const RESERVED_PORTS: [u16; 10] = [80, 443, 2087, 3306, 5432, 6379, 8443, 8444, 13306, 15432];

/// The root login choices of `PermitRootLogin`.
pub const ROOT_LOGIN: [&str; 4] = ["yes", "no", "prohibit-password", "forced-commands-only"];

/// What the SSH server will do, as `sshd -T` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effective {
    /// Every port it listens on.
    pub ports: Vec<u16>,
    /// `PermitRootLogin`, with `without-password` written as `prohibit-password`.
    pub root_login: String,
    /// `PasswordAuthentication`.
    pub password: bool,
    /// `PubkeyAuthentication`.
    pub pubkey: bool,
}

/// Reads the output of `sshd -T`.
#[must_use]
pub fn parse_effective(text: &str) -> Effective {
    let mut e = Effective {
        ports: Vec::new(),
        root_login: String::new(),
        password: true,
        pubkey: true,
    };
    let mut seen_root = false;
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once(char::is_whitespace) else {
            continue;
        };
        let value = value.trim().to_ascii_lowercase();
        match key.to_ascii_lowercase().as_str() {
            "port" => {
                if let Ok(p) = value.parse::<u16>()
                    && !e.ports.contains(&p)
                {
                    e.ports.push(p);
                }
            }
            "permitrootlogin" if !seen_root => {
                seen_root = true;
                e.root_login = if value == "without-password" {
                    "prohibit-password".to_owned()
                } else {
                    value
                };
            }
            "passwordauthentication" => e.password = value == "yes",
            "pubkeyauthentication" => e.pubkey = value == "yes",
            _ => {}
        }
    }
    e
}

/// Whether the state leaves nobody able to log in: no password, and no key or no key login.
#[must_use]
pub fn locks_out(password: bool, pubkey: bool, keys: usize) -> bool {
    !password && (!pubkey || keys == 0)
}

fn path_text(path: &Path) -> Result<&str, Failure> {
    path.to_str()
        .filter(|p| p.starts_with('/') && !p.contains(['\0', '\n', ' ', '"', '\'', '$', '`']))
        .ok_or((
            ErrorCode::Failed,
            "a path of the SSH settings is not usable",
        ))
}

/// A file's text; empty when the file is not there.
fn read_file(env: &Env, path: &str) -> Option<String> {
    let bytes = run_io(
        env,
        "/usr/bin/sh",
        &["-c", "[ ! -f \"$1\" ] || cat \"$1\"", "sh", path],
        b"",
    )?;
    String::from_utf8(bytes).ok()
}

/// Writes a file in one step (beside it, then renamed over it), with the given mode.
fn write_atomic(env: &Env, path: &str, mode: &str, content: &str) -> bool {
    run_io(
        env,
        "/usr/bin/sh",
        &[
            "-c",
            "cat > \"$1.new\" && chmod \"$2\" \"$1.new\" && mv -f \"$1.new\" \"$1\"",
            "sh",
            path,
            mode,
        ],
        content.as_bytes(),
    )
    .is_some()
}

/// What `sshd` will do with the configuration in `config`.
fn effective_of(env: &Env, config: &str) -> Option<Effective> {
    let out = run_io(env, &env.sshd.to_string_lossy(), &["-T", "-f", config], b"")?;
    Some(parse_effective(&String::from_utf8_lossy(&out)))
}

/// The ports SSH listens on now (empty when they cannot be read).
#[must_use]
pub fn listening_ports(env: &Env) -> Vec<u16> {
    path_text(&env.sshd_config)
        .ok()
        .and_then(|config| effective_of(env, config))
        .map(|e| e.ports)
        .unwrap_or_default()
}

fn reload(env: &Env) -> bool {
    env.direct
        || run(env, "/usr/bin/systemctl", &["reload", "ssh"])
        || run(env, "/usr/bin/systemctl", &["reload", "sshd"])
}

/// Whether something answers on `port` of this machine (a few tries, as a reload takes a moment).
fn listening(env: &Env, port: u16) -> bool {
    if env.direct {
        return true;
    }
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    for _ in 0..8 {
        if TcpStream::connect_timeout(&address, Duration::from_secs(1)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

// --- keys -----------------------------------------------------------------------------------------

fn valid_blob(blob: &str) -> bool {
    (16..=4096).contains(&blob.len())
        && blob.starts_with("AAAA")
        && blob
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
}

/// The key type and key of a line of the key file, or `None` for a comment, a blank or a line
/// that is not a key. Options in front of the type are skipped.
fn parse_key(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.starts_with('#') {
        return None;
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    let i = words.iter().position(|w| KEY_TYPES.contains(w))?;
    let blob = words.get(i + 1)?;
    valid_blob(blob).then(|| (words[i], *blob))
}

/// A new key line, checked and written the plain way: `type key [comment]`, no options.
fn checked_new_key(line: &str) -> Result<String, Failure> {
    let bad = |m| (ErrorCode::InvalidArguments, m);
    let line = line.trim();
    if line.is_empty() || line.len() > MAX_KEY_LINE || line.contains(['\n', '\r', '\0']) {
        return Err(bad("that is not one public key line"));
    }
    let mut words = line.split_whitespace();
    let kind = words.next().unwrap_or_default();
    let blob = words.next().unwrap_or_default();
    if !KEY_TYPES.contains(&kind) {
        return Err(bad("this key type is not accepted"));
    }
    if !valid_blob(blob) {
        return Err(bad("the key is not valid"));
    }
    let comment: Vec<&str> = words.collect();
    let comment = comment.join(" ");
    if comment.len() > 256 || !comment.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        return Err(bad("the key's comment is not usable"));
    }
    Ok(if comment.is_empty() {
        format!("{kind} {blob}")
    } else {
        format!("{kind} {blob} {comment}")
    })
}

fn load_keys(env: &Env) -> Result<String, Failure> {
    let path = path_text(&env.root_keys)?;
    read_file(env, path).ok_or((ErrorCode::Failed, "the key file could not be read"))
}

/// The key lines of root's key file (at most [`MAX_KEYS`]).
///
/// # Errors
/// The file could not be read.
pub fn read_keys(env: &Env) -> Result<Vec<String>, Failure> {
    Ok(load_keys(env)?
        .lines()
        .filter(|l| parse_key(l).is_some())
        .take(MAX_KEYS)
        .map(|l| l.trim().to_owned())
        .collect())
}

/// What SSH does now and the keys of root, as the answer to the read action.
///
/// # Errors
/// The settings or the key file could not be read.
pub fn read_all(env: &Env) -> Result<serde_json::Value, Failure> {
    let config = path_text(&env.sshd_config)?;
    let now = effective_of(env, config).ok_or((
        ErrorCode::Failed,
        "the SSH server's settings could not be read",
    ))?;
    Ok(serde_json::json!({
        "keys": read_keys(env)?,
        "ports": now.ports,
        "root_login": now.root_login,
        "password_authentication": now.password,
        "pubkey_authentication": now.pubkey,
    }))
}

/// Authorises one more key for root.
///
/// # Errors
/// A key that is not acceptable or is there already, too many keys, or the file not written.
pub fn add_key(env: &Env, key: &str) -> Result<(), Failure> {
    let line = checked_new_key(key)?;
    let (_, blob) =
        parse_key(&line).ok_or((ErrorCode::InvalidArguments, "the key is not valid"))?;
    let mut text = load_keys(env)?;
    let mut count = 0;
    for l in text.lines() {
        if let Some((_, b)) = parse_key(l) {
            if b == blob {
                return Err((
                    ErrorCode::InvalidArguments,
                    "that key is already authorised",
                ));
            }
            count += 1;
        }
    }
    if count >= MAX_KEYS {
        return Err((
            ErrorCode::InvalidArguments,
            "there are too many keys already",
        ));
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&line);
    text.push('\n');
    let path = path_text(&env.root_keys)?;
    let dir = env
        .root_keys
        .parent()
        .and_then(Path::to_str)
        .unwrap_or("/root/.ssh");
    let _ = run(env, "/usr/bin/install", &["-d", "-m", "0700", dir]);
    if write_atomic(env, path, "600", &text) {
        Ok(())
    } else {
        Err((ErrorCode::Failed, "the key could not be saved"))
    }
}

/// Takes one key away (named by its key text). Refused when it would leave nobody able to log in.
///
/// # Errors
/// No such key, a removal that would lock everyone out, or the file not written.
pub fn remove_key(env: &Env, blob: &str) -> Result<(), Failure> {
    if !valid_blob(blob) {
        return Err((ErrorCode::InvalidArguments, "the key is not valid"));
    }
    let text = load_keys(env)?;
    let mut kept = String::new();
    let (mut removed, mut remaining) = (false, 0);
    for l in text.lines() {
        match parse_key(l) {
            Some((_, b)) if b == blob => removed = true,
            other => {
                if other.is_some() {
                    remaining += 1;
                }
                kept.push_str(l);
                kept.push('\n');
            }
        }
    }
    if !removed {
        return Err((ErrorCode::InvalidArguments, "there is no such key"));
    }
    let config = path_text(&env.sshd_config)?;
    if effective_of(env, config).is_some_and(|now| locks_out(now.password, now.pubkey, remaining)) {
        return Err((
            ErrorCode::InvalidArguments,
            "this is the last key and password login is off: removing it would lock everyone out",
        ));
    }
    let path = path_text(&env.root_keys)?;
    if write_atomic(env, path, "600", &kept) {
        Ok(())
    } else {
        Err((ErrorCode::Failed, "the key could not be removed"))
    }
}

// --- the few settings -------------------------------------------------------------------------------

/// What the administrator asked to change; what is `None` stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// The port SSH listens on (22, or 1024 to 65535 and not one of the panel's).
    pub port: Option<u16>,
    /// One of [`ROOT_LOGIN`].
    pub root_login: Option<String>,
    /// Password login.
    pub password: Option<bool>,
    /// Key login.
    pub pubkey: Option<bool>,
}

/// Whether `port` may be used for SSH.
#[must_use]
pub fn port_allowed(port: u16) -> bool {
    (port == 22 || port >= 1024) && !RESERVED_PORTS.contains(&port)
}

fn render(ports: &[u16], root_login: &str, password: bool, pubkey: bool) -> String {
    let yes = |b: bool| if b { "yes" } else { "no" };
    let mut text = String::from(
        "# Written by Built Panel (System > SSH access). Change the settings there: this file is rewritten.\n",
    );
    for p in ports {
        text.push_str(&format!("Port {p}\n"));
    }
    text.push_str(&format!("PermitRootLogin {root_login}\n"));
    text.push_str(&format!("PasswordAuthentication {}\n", yes(password)));
    text.push_str(&format!("PubkeyAuthentication {}\n", yes(pubkey)));
    text
}

/// Changes the SSH settings, checks them, reloads SSH, and puts the old ones back when anything
/// does not hold.
///
/// # Errors
/// Nothing asked, a value out of range, a combination that locks everyone out, a setting that
/// another line of `sshd_config` overrides, or SSH not coming back.
pub fn set_settings(env: &Env, ask: &Settings) -> Result<(), Failure> {
    let bad = |m| (ErrorCode::InvalidArguments, m);
    if *ask == Settings::default() {
        return Err(bad("nothing to change"));
    }
    if ask.port.is_some_and(|p| !port_allowed(p)) {
        return Err(bad("this port cannot be used for SSH"));
    }
    if ask
        .root_login
        .as_deref()
        .is_some_and(|v| !ROOT_LOGIN.contains(&v))
    {
        return Err(bad("this root login choice is not known"));
    }
    let config = path_text(&env.sshd_config)?;
    let dropin = path_text(&env.ssh_dropin)?;
    let now = effective_of(env, config).ok_or((
        ErrorCode::Failed,
        "the SSH server's settings could not be read",
    ))?;
    let ports = ask.port.map_or_else(|| now.ports.clone(), |p| vec![p]);
    let root_login = ask
        .root_login
        .clone()
        .unwrap_or_else(|| now.root_login.clone());
    let root_login = if ROOT_LOGIN.contains(&root_login.as_str()) {
        root_login
    } else {
        "prohibit-password".to_owned()
    };
    let password = ask.password.unwrap_or(now.password);
    let pubkey = ask.pubkey.unwrap_or(now.pubkey);
    if ports.is_empty() {
        return Err((ErrorCode::Failed, "the SSH server's port could not be read"));
    }
    if locks_out(password, pubkey, read_keys(env)?.len()) {
        return Err(bad(
            "this would lock everyone out: password login is off and there is no key that can log in",
        ));
    }
    let previous =
        read_file(env, dropin).ok_or((ErrorCode::Failed, "the old settings could not be read"))?;
    let dir = env
        .ssh_dropin
        .parent()
        .and_then(Path::to_str)
        .unwrap_or("/etc/ssh");
    let _ = run(env, "/usr/bin/install", &["-d", "-m", "0755", dir]);
    if !write_atomic(
        env,
        dropin,
        "644",
        &render(&ports, &root_login, password, pubkey),
    ) {
        return Err((ErrorCode::Failed, "the settings could not be written"));
    }
    let wanted = Effective {
        ports: ports.clone(),
        root_login,
        password,
        pubkey,
    };
    let outcome = check_applied(env, config, &wanted);
    if outcome.is_err() {
        if previous.is_empty() {
            let _ = run(env, "/usr/bin/rm", &["-f", dropin]);
        } else {
            let _ = write_atomic(env, dropin, "644", &previous);
        }
        let _ = reload(env);
    }
    outcome
}

fn check_applied(env: &Env, config: &str, wanted: &Effective) -> Result<(), Failure> {
    if !run(env, &env.sshd.to_string_lossy(), &["-t", "-f", config]) {
        return Err((ErrorCode::Failed, "the SSH server refuses these settings"));
    }
    let got = effective_of(env, config).ok_or((
        ErrorCode::Failed,
        "the SSH server's settings could not be read",
    ))?;
    let (mut a, mut b) = (got.ports.clone(), wanted.ports.clone());
    a.sort_unstable();
    b.sort_unstable();
    if a != b
        || got.root_login != wanted.root_login
        || got.password != wanted.password
        || got.pubkey != wanted.pubkey
    {
        return Err((
            ErrorCode::Failed,
            "a line of sshd_config overrides this setting: change it in the advanced editor",
        ));
    }
    // A node firewall must let the new port in before SSH moves to it.
    if !env.direct {
        for port in &wanted.ports {
            crate::nodefw::open_port_now(*port);
        }
    }
    if !reload(env) {
        return Err((ErrorCode::Failed, "the SSH server could not be reloaded"));
    }
    if !listening(env, wanted.ports[0]) {
        return Err((
            ErrorCode::Failed,
            "the SSH server does not answer on its port",
        ));
    }
    Ok(())
}

// --- the whole file ---------------------------------------------------------------------------------

/// Replaces `sshd_config` with `text`, after `sshd -t` accepted it, and puts the old file back when
/// SSH does not come up.
///
/// # Errors
/// A text that is empty, too long or not accepted by `sshd -t`, a lock-out, or SSH not coming back.
pub fn write_config(env: &Env, text: &str) -> Result<(), Failure> {
    let bad = |m| (ErrorCode::InvalidArguments, m);
    if text.trim().is_empty() || text.len() > MAX_CONFIG_BYTES || text.contains('\0') {
        return Err(bad("the configuration is empty or too long"));
    }
    let config = path_text(&env.sshd_config)?;
    let candidate = format!("{config}.builtpanel-new");
    let previous = format!("{config}.builtpanel-prev");
    let mut text = text.replace("\r\n", "\n");
    if !text.ends_with('\n') {
        text.push('\n');
    }
    if !write_atomic(env, &candidate, "644", &text) {
        return Err((ErrorCode::Failed, "the configuration could not be written"));
    }
    let discard = |why: Failure| {
        let _ = run(env, "/usr/bin/rm", &["-f", &candidate]);
        why
    };
    if !run(env, &env.sshd.to_string_lossy(), &["-t", "-f", &candidate]) {
        return Err(discard((
            ErrorCode::InvalidArguments,
            "the SSH server does not accept this configuration",
        )));
    }
    let wanted = effective_of(env, &candidate).ok_or_else(|| {
        discard((
            ErrorCode::Failed,
            "the SSH server's settings could not be read",
        ))
    })?;
    if locks_out(wanted.password, wanted.pubkey, read_keys(env)?.len()) {
        return Err(discard(bad(
            "this would lock everyone out: password login is off and there is no key that can log in",
        )));
    }
    if wanted.ports.is_empty() {
        return Err(discard(bad("the configuration leaves SSH without a port")));
    }
    if !run(env, "/usr/bin/cp", &["-p", config, &previous]) {
        return Err(discard((
            ErrorCode::Failed,
            "the old configuration could not be kept",
        )));
    }
    if !run(env, "/usr/bin/mv", &["-f", &candidate, config]) {
        return Err(discard((
            ErrorCode::Failed,
            "the configuration could not be put in place",
        )));
    }
    if reload(env) && listening(env, wanted.ports[0]) {
        return Ok(());
    }
    let _ = run(env, "/usr/bin/mv", &["-f", &previous, config]);
    let _ = reload(env);
    Err((
        ErrorCode::Failed,
        "the SSH server did not come back with it: the old configuration is back",
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl me@laptop";
    const KEY2: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGq1xWxkq3Qq7f0oPzv3YqkLwY5l1u2w9Zk8m4n6p7QR other";

    #[test]
    fn keys_are_read_and_only_plain_ones_are_taken() {
        assert_eq!(parse_key(KEY).unwrap().0, "ssh-ed25519");
        assert!(parse_key("# ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMq").is_none());
        assert!(parse_key("").is_none());
        // Options in front of a key are read (so such a key can be listed and removed) ...
        assert!(parse_key(&format!("no-pty {KEY}")).is_some());
        // ... but a new key is taken only in the plain form.
        assert_eq!(checked_new_key(&format!("  {KEY}  ")).unwrap(), KEY);
        for bad in [
            "",
            "command=\"x\" ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMq",
            "ssh-dss AAAAB3NzaC1kc3MAAACB",
            "ssh-ed25519 notbase64!",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMq\nssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMq",
        ] {
            assert!(checked_new_key(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn sshd_output_is_read_and_a_lock_out_is_named() {
        let e = parse_effective(
            "port 22\nport 2222\npermitrootlogin without-password\npasswordauthentication no\npubkeyauthentication yes\n",
        );
        assert_eq!(e.ports, vec![22, 2222]);
        assert_eq!(e.root_login, "prohibit-password");
        assert!(!e.password && e.pubkey);
        assert!(locks_out(false, true, 0));
        assert!(locks_out(false, false, 3));
        assert!(!locks_out(false, true, 1));
        assert!(!locks_out(true, false, 0));
    }

    #[test]
    fn ports_of_the_panel_cannot_be_taken() {
        assert!(port_allowed(22) && port_allowed(2222) && port_allowed(40000));
        assert!(!port_allowed(0) && !port_allowed(80) && !port_allowed(443) && !port_allowed(8444));
        assert!(!port_allowed(23) && !port_allowed(1023) && !port_allowed(13306));
    }

    /// A folder with a stand-in for `sshd` (`-t` fails on a config that says `Broken`; `-T` prints
    /// the settings of the drop-in over the defaults) and the files the module works on.
    struct Rig {
        dir: std::path::PathBuf,
        env: Env,
    }

    impl Rig {
        fn new(name: &str, keys: &str, dropin: Option<&str>) -> Self {
            let dir = std::env::temp_dir().join(format!("bp-ssh-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("sshd_config.d")).unwrap();
            let sshd = dir.join("sshd");
            let script = r#"#!/bin/sh
mode="$1"; shift; [ "$1" = -f ] && file="$2"
case "$mode" in
  -t) ! grep -q Broken "$file" ;;
  -T) d="$(dirname "$file")/sshd_config.d/00-builtpanel.conf"
      if grep -qi '^PasswordAuthentication' "$d" 2>/dev/null || grep -qi '^PasswordAuthentication' "$file"; then :; else echo passwordauthentication yes; fi
      grep -hiE '^(port|permitrootlogin|passwordauthentication|pubkeyauthentication) ' "$d" "$file" 2>/dev/null | tr 'A-Z' 'a-z'
      grep -qi '^port' "$d" "$file" 2>/dev/null || echo port 22
      grep -qi '^pubkeyauthentication' "$d" "$file" 2>/dev/null || echo pubkeyauthentication yes ;;
esac
"#;
            std::fs::write(&sshd, script).unwrap();
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&sshd, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            std::fs::write(dir.join("sshd_config"), "Include sshd_config.d/*.conf\n").unwrap();
            std::fs::write(dir.join("authorized_keys"), keys).unwrap();
            let dropin_path = dir.join("sshd_config.d/00-builtpanel.conf");
            if let Some(d) = dropin {
                std::fs::write(&dropin_path, d).unwrap();
            }
            let env = Env {
                direct: true,
                sshd,
                sshd_config: dir.join("sshd_config"),
                ssh_dropin: dropin_path,
                root_keys: dir.join("authorized_keys"),
                ..Env::default()
            };
            Self { dir, env }
        }

        fn dropin(&self) -> String {
            std::fs::read_to_string(&self.env.ssh_dropin).unwrap_or_default()
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn keys_can_be_added_listed_and_removed_but_not_twice_or_the_last_without_a_password() {
        let rig = Rig::new("keys", "# my keys\n", None);
        add_key(&rig.env, KEY).unwrap();
        add_key(&rig.env, KEY2).unwrap();
        assert_eq!(
            add_key(&rig.env, KEY).unwrap_err().0,
            ErrorCode::InvalidArguments
        );
        assert_eq!(read_keys(&rig.env).unwrap().len(), 2);
        let text = std::fs::read_to_string(&rig.env.root_keys).unwrap();
        assert!(text.starts_with("# my keys\n"), "other lines stay: {text}");
        let (_, blob) = parse_key(KEY).unwrap();
        remove_key(&rig.env, blob).unwrap();
        assert_eq!(read_keys(&rig.env).unwrap().len(), 1);
        assert_eq!(
            remove_key(&rig.env, blob).unwrap_err().0,
            ErrorCode::InvalidArguments
        );
        // With password login off, the last key stays.
        std::fs::write(
            rig.dir.join("sshd_config.d/00-builtpanel.conf"),
            "PasswordAuthentication no\n",
        )
        .unwrap();
        let (_, blob2) = parse_key(KEY2).unwrap();
        let e = remove_key(&rig.env, blob2).unwrap_err();
        assert!(e.1.contains("lock everyone out"), "{}", e.1);
        assert_eq!(read_keys(&rig.env).unwrap().len(), 1);
    }

    #[test]
    fn settings_are_written_checked_and_a_lock_out_is_refused() {
        let rig = Rig::new("settings", &format!("{KEY}\n"), None);
        set_settings(
            &rig.env,
            &Settings {
                port: Some(2222),
                password: Some(false),
                root_login: Some("prohibit-password".into()),
                ..Settings::default()
            },
        )
        .unwrap();
        let text = rig.dropin();
        for line in [
            "Port 2222",
            "PermitRootLogin prohibit-password",
            "PasswordAuthentication no",
            "PubkeyAuthentication yes",
        ] {
            assert!(text.contains(line), "{line} in {text}");
        }
        // Key login off with password login off: refused, and the file stays as it was.
        let before = rig.dropin();
        let e = set_settings(
            &rig.env,
            &Settings {
                pubkey: Some(false),
                ..Settings::default()
            },
        )
        .unwrap_err();
        assert!(e.1.contains("lock everyone out"), "{}", e.1);
        assert_eq!(rig.dropin(), before);
        for bad in [
            Settings::default(),
            Settings {
                port: Some(80),
                ..Settings::default()
            },
            Settings {
                root_login: Some("maybe".into()),
                ..Settings::default()
            },
        ] {
            assert_eq!(
                set_settings(&rig.env, &bad).unwrap_err().0,
                ErrorCode::InvalidArguments
            );
        }
    }

    #[test]
    fn a_setting_that_sshd_does_not_report_back_is_taken_out_again() {
        // The main file sets the password login itself, and the stand-in reports the main file's
        // value ahead of the drop-in's only when the drop-in has none: here the drop-in has it, so
        // make the stand-in disagree by a drop-in that never takes effect (no Include line).
        let rig = Rig::new("override", &format!("{KEY}\n"), Some("Port 22\n"));
        std::fs::write(rig.dir.join("sshd_config"), "Port 22\nBroken\n").unwrap();
        let e = set_settings(
            &rig.env,
            &Settings {
                password: Some(false),
                ..Settings::default()
            },
        );
        // `-t` fails on the broken main file, so the old drop-in is back.
        assert!(e.is_err());
        assert_eq!(rig.dropin(), "Port 22\n");
    }

    #[test]
    fn the_whole_file_is_checked_and_kept_until_it_works() {
        let rig = Rig::new("config", &format!("{KEY}\n"), None);
        let old = std::fs::read_to_string(&rig.env.sshd_config).unwrap();
        // Rejected by `sshd -t`: nothing changes, nothing is left behind.
        let e = write_config(&rig.env, "Port 22\nBroken\n").unwrap_err();
        assert_eq!(e.0, ErrorCode::InvalidArguments);
        assert_eq!(std::fs::read_to_string(&rig.env.sshd_config).unwrap(), old);
        assert!(!rig.dir.join("sshd_config.builtpanel-new").exists());
        // A lock-out is refused.
        let e = write_config(
            &rig.env,
            "PasswordAuthentication no\nPubkeyAuthentication no\n",
        )
        .unwrap_err();
        assert!(e.1.contains("lock everyone out"), "{}", e.1);
        // A good text goes in, and the old one is kept beside it.
        write_config(&rig.env, "Port 2200\r\nPermitRootLogin no").unwrap();
        assert_eq!(
            std::fs::read_to_string(&rig.env.sshd_config).unwrap(),
            "Port 2200\nPermitRootLogin no\n"
        );
        assert_eq!(
            std::fs::read_to_string(rig.dir.join("sshd_config.builtpanel-prev")).unwrap(),
            old
        );
        for bad in ["", "   \n", &"x".repeat(MAX_CONFIG_BYTES + 1)] {
            assert_eq!(
                write_config(&rig.env, bad).unwrap_err().0,
                ErrorCode::InvalidArguments
            );
        }
    }
}
