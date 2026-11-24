//! `builtpanel-root-auth`: the root login helper.
//!
//! The control plane never runs as root, so it cannot read `/etc/shadow`. When someone logs in
//! to the panel as `root` (like WHM), the control plane asks this helper whether the password is
//! the server's root password. That is all it does: it reads one line of `/etc/shadow`, checks
//! one password against it and answers one byte. It runs no programs, writes no files except its
//! socket, and talks only to the panel's own system user (checked with `SO_PEERCRED`).
//!
//! Protocol (one request per connection): two bytes, the password's length (big endian, at most
//! [`MAX_PASSWORD_BYTES`]), then the password. Answer, one byte: `Y` it is root's password,
//! `N` it is not, `L` root has no usable password (locked or empty), `U` root's hash uses a
//! scheme this helper does not know, `E` it could not check (e.g. cannot read the file).
//!
//! Brute force: requests are handled one at a time and a wrong password is answered only after
//! [`Config::wrong_delay`], so no more than about one guess a second reaches the hash whatever
//! the panel's own lockout does.

use std::{
    io::{self, Read as _, Write as _},
    os::unix::{
        fs::{MetadataExt as _, PermissionsExt as _},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use sha_crypt::ShaCrypt;
use yescrypt::{PasswordVerifier as _, Yescrypt};
use zeroize::Zeroize as _;

/// Longest password accepted (bytes), the same limit as the panel's login.
pub const MAX_PASSWORD_BYTES: usize = 1024;
/// Where the helper listens by default.
pub const DEFAULT_SOCKET: &str = "/run/builtpanel-root-auth/root-auth.sock";

/// The result of checking a password against the shadow entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// The password matches.
    Match,
    /// It does not.
    Mismatch,
    /// The account has no usable password (locked, disabled or empty).
    NoPassword,
    /// The hash uses a scheme that is not supported.
    Unsupported,
    /// The check could not be made.
    Error,
}

impl Answer {
    /// The byte sent on the socket.
    #[must_use]
    pub fn byte(self) -> u8 {
        match self {
            Self::Match => b'Y',
            Self::Mismatch => b'N',
            Self::NoPassword => b'L',
            Self::Unsupported => b'U',
            Self::Error => b'E',
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Match => "correct password",
            Self::Mismatch => "wrong password",
            Self::NoPassword => "the account has no usable password",
            Self::Unsupported => "unsupported hash scheme",
            Self::Error => "could not check",
        }
    }
}

/// The hash field of `user`'s line in a shadow file's text, if there is a line for it.
#[must_use]
pub fn shadow_hash<'a>(shadow: &'a str, user: &str) -> Option<&'a str> {
    shadow.lines().find_map(|line| {
        let mut fields = line.split(':');
        (fields.next() == Some(user)).then(|| fields.next().unwrap_or(""))
    })
}

/// Checks `password` against a hash field of `/etc/shadow`.
#[must_use]
pub fn verify(password: &[u8], hash: &str) -> Answer {
    // Empty: a login without a password, which the panel never allows. `!`/`*`: locked or no
    // password ("!" in front of a real hash is `passwd -l`).
    if hash.is_empty() || hash.starts_with('!') || hash.starts_with('*') {
        return Answer::NoPassword;
    }
    let ok = if hash.starts_with("$y$") {
        Yescrypt::default().verify_password(password, hash)
    } else if hash.starts_with("$6$") || hash.starts_with("$5$") {
        ShaCrypt::default().verify_password(password, hash)
    } else {
        return Answer::Unsupported;
    };
    match ok {
        Ok(()) => Answer::Match,
        Err(yescrypt::password_hash::Error::PasswordInvalid) => Answer::Mismatch,
        Err(_) => Answer::Unsupported,
    }
}

/// Where a message goes (stderr in the program, memory in tests).
pub trait Log {
    /// Writes one line.
    fn line(&self, text: &str);
}

/// Writes to stderr (the journal under systemd).
#[derive(Debug, Clone, Copy)]
pub struct StderrLog;

impl Log for StderrLog {
    #[allow(clippy::print_stderr)]
    fn line(&self, text: &str) {
        eprintln!("{text}");
    }
}

/// What the helper serves.
#[derive(Debug, Clone)]
pub struct Config {
    /// The socket path.
    pub socket: PathBuf,
    /// The shadow file (`/etc/shadow`).
    pub shadow: PathBuf,
    /// The system account whose password is checked (`root`).
    pub user: String,
    /// The only user id (besides root) that may ask: the panel's user.
    pub client_uid: u32,
    /// The group that owns the socket (the panel's group).
    pub client_gid: u32,
    /// How long to wait before answering a wrong password.
    pub wrong_delay: Duration,
    /// How long a client may take to send its request.
    pub read_timeout: Duration,
}

/// Creates the socket: owned by root and the panel's group, mode 0660.
///
/// # Errors
/// I/O errors; a non-socket file at the path is left alone and reported.
pub fn bind(config: &Config) -> io::Result<UnixListener> {
    use std::os::unix::fs::FileTypeExt as _;
    match std::fs::symlink_metadata(&config.socket) {
        Ok(meta) if meta.file_type().is_socket() => std::fs::remove_file(&config.socket)?,
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} exists and is not a socket", config.socket.display()),
            ));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    let listener = UnixListener::bind(&config.socket)?;
    // Until these calls are done the peer check still keeps everyone else out.
    std::fs::set_permissions(&config.socket, std::fs::Permissions::from_mode(0o600))?;
    if std::fs::metadata(&config.socket)?.gid() != config.client_gid {
        std::os::unix::fs::chown(&config.socket, None, Some(config.client_gid))?;
    }
    std::fs::set_permissions(&config.socket, std::fs::Permissions::from_mode(0o660))?;
    Ok(listener)
}

/// Serves until `stop` is set, one connection at a time.
pub fn serve(listener: &UnixListener, config: &Config, log: &dyn Log, stop: &AtomicBool) {
    for stream in listener.incoming() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match stream {
            Ok(stream) => handle(stream, config, log),
            Err(err) => {
                log.line(&format!("accept failed: {err}"));
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn handle(mut stream: UnixStream, config: &Config, log: &dyn Log) {
    let peer = match rustix::net::sockopt::socket_peercred(&stream) {
        Ok(cred) => cred,
        Err(err) => {
            log.line(&format!(
                "refused a connection: no peer credentials ({err})"
            ));
            return;
        }
    };
    let uid = peer.uid.as_raw();
    if uid != 0 && uid != config.client_uid {
        log.line(&format!(
            "refused a connection from uid {uid} (pid {}): only the panel may ask",
            peer.pid.as_raw_nonzero()
        ));
        return;
    }
    let _ = stream.set_read_timeout(Some(config.read_timeout));
    let _ = stream.set_write_timeout(Some(config.read_timeout));
    let mut password = match read_request(&mut stream) {
        Ok(password) => password,
        Err(err) => {
            log.line(&format!(
                "bad request from pid {}: {err}",
                peer.pid.as_raw_nonzero()
            ));
            return;
        }
    };
    let answer = match std::fs::read_to_string(&config.shadow) {
        Ok(mut text) => {
            let answer = match shadow_hash(&text, &config.user) {
                Some(hash) => verify(&password, hash),
                None => Answer::NoPassword,
            };
            text.zeroize();
            answer
        }
        Err(err) => {
            log.line(&format!("cannot read {}: {err}", config.shadow.display()));
            Answer::Error
        }
    };
    password.zeroize();
    log.line(&format!(
        "{} login check for pid {}: {}",
        config.user,
        peer.pid.as_raw_nonzero(),
        answer.word()
    ));
    if answer != Answer::Match {
        std::thread::sleep(config.wrong_delay);
    }
    let _ = stream.write_all(&[answer.byte()]);
}

fn read_request(stream: &mut UnixStream) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 2];
    stream.read_exact(&mut len)?;
    let len = usize::from(u16::from_be_bytes(len));
    if len > MAX_PASSWORD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the password is too long",
        ));
    }
    let mut password = vec![0u8; len];
    stream.read_exact(&mut password)?;
    Ok(password)
}

/// The user and group ids of a system account, from the text of `/etc/passwd`.
#[must_use]
pub fn lookup_user(passwd: &str, name: &str) -> Option<(u32, u32)> {
    passwd.lines().find_map(|line| {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() >= 4 && f[0] == name {
            Some((f[2].parse().ok()?, f[3].parse().ok()?))
        } else {
            None
        }
    })
}

/// [`lookup_user`] on `/etc/passwd`.
#[must_use]
pub fn lookup_system_user(name: &str) -> Option<(u32, u32)> {
    lookup_user(
        &std::fs::read_to_string(Path::new("/etc/passwd")).ok()?,
        name,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    // Made on Ubuntu 24.04 for the password "correct horse": libxcrypt's crypt() with a
    // yescrypt setting (what `passwd` stores), `openssl passwd -6` and `openssl passwd -5`.
    const YESCRYPT: &str =
        "$y$j9T$3JMzYbdrRHDw9kE5CBqhf/$phLjPDdpRsXB0PPBf08wTBs3A5xIs73ino3WvvAQ76.";
    const SHA512: &str = "$6$saltsaltsalt$Cy2drr8kDRji6smvDcT28wkqtq0R0VzVL5CkrjPQCITc5d/31j94knt9rGTcVSyjLXfjsiIsBh5ee8qR/3QDx1";
    const SHA256: &str = "$5$saltsaltsalt$7axwV9kT3PvvGXgJl1FcfUCHLdLFhWk/dIFhzsbucLB";

    #[test]
    fn the_hash_field_of_the_right_line_is_found() {
        let shadow =
            "daemon:*:19000:0:99999:7:::\nroot:$y$abc:19000:0:99999:7:::\nrooty:x:1::::::\n";
        assert_eq!(shadow_hash(shadow, "root"), Some("$y$abc"));
        assert_eq!(shadow_hash(shadow, "daemon"), Some("*"));
        assert_eq!(shadow_hash(shadow, "nobody"), None);
        assert_eq!(shadow_hash("root\n", "root"), Some(""));
    }

    #[test]
    fn locked_and_empty_passwords_never_match() {
        for hash in ["", "!", "*", "!*", "!$y$j9T$abc$def", "*LK*"] {
            assert_eq!(verify(b"", hash), Answer::NoPassword, "{hash}");
            assert_eq!(verify(b"anything", hash), Answer::NoPassword, "{hash}");
        }
    }

    #[test]
    fn unknown_schemes_are_reported() {
        assert_eq!(verify(b"x", "$1$abc$def"), Answer::Unsupported);
        assert_eq!(verify(b"x", "$2b$10$abc"), Answer::Unsupported);
        assert_eq!(verify(b"x", "abcdefghijklm"), Answer::Unsupported);
    }

    #[test]
    fn real_hashes_are_checked() {
        for hash in [YESCRYPT, SHA512, SHA256] {
            assert_eq!(verify(b"correct horse", hash), Answer::Match, "{hash}");
            assert_eq!(verify(b"correct horsE", hash), Answer::Mismatch, "{hash}");
            assert_eq!(verify(b"", hash), Answer::Mismatch, "{hash}");
        }
    }

    #[test]
    fn passwd_lines_give_ids() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\nbuiltpanel:x:998:997::/var/lib/builtpanel:/usr/sbin/nologin\n";
        assert_eq!(lookup_user(passwd, "builtpanel"), Some((998, 997)));
        assert_eq!(lookup_user(passwd, "dh"), None);
    }

    #[derive(Default)]
    struct Memory(Mutex<Vec<String>>);

    impl Log for Memory {
        fn line(&self, text: &str) {
            self.0.lock().unwrap().push(text.to_owned());
        }
    }

    fn ask(path: &Path, password: &[u8]) -> io::Result<u8> {
        let mut s = UnixStream::connect(path)?;
        let len = u16::try_from(password.len()).unwrap();
        s.write_all(&len.to_be_bytes())?;
        s.write_all(password)?;
        let mut answer = [0u8; 1];
        s.read_exact(&mut answer)?;
        Ok(answer[0])
    }

    #[test]
    fn the_socket_answers_the_panel_and_never_logs_the_password() {
        let dir = std::env::temp_dir().join(format!("bp-root-auth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shadow = dir.join("shadow");
        std::fs::write(&shadow, format!("root:{YESCRYPT}:19000:0:99999:7:::\n")).unwrap();
        let me = std::fs::metadata(&dir).unwrap();
        let config = Config {
            socket: dir.join("s"),
            shadow,
            user: "root".into(),
            client_uid: me.uid(),
            client_gid: me.gid(),
            wrong_delay: Duration::from_millis(10),
            read_timeout: Duration::from_secs(2),
        };
        let listener = bind(&config).unwrap();
        let mode = std::fs::metadata(&config.socket)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o660);
        let log = Arc::new(Memory::default());
        let stop = Arc::new(AtomicBool::new(false));
        let server = {
            let (config, log, stop) = (config.clone(), Arc::clone(&log), Arc::clone(&stop));
            std::thread::spawn(move || serve(&listener, &config, log.as_ref(), &stop))
        };
        assert_eq!(ask(&config.socket, b"correct horse").unwrap(), b'Y');
        assert_eq!(ask(&config.socket, b"wrong horse").unwrap(), b'N');
        // Too long: the connection is closed without an answer.
        let mut s = UnixStream::connect(&config.socket).unwrap();
        s.write_all(&u16::MAX.to_be_bytes()).unwrap();
        let mut rest = Vec::new();
        assert_eq!(s.read_to_end(&mut rest).unwrap(), 0);
        stop.store(true, Ordering::Relaxed);
        let _ = UnixStream::connect(&config.socket);
        server.join().unwrap();
        let lines = log.0.lock().unwrap().join("\n");
        assert!(!lines.contains("horse"), "{lines}");
        assert!(lines.contains("correct password") && lines.contains("wrong password"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
