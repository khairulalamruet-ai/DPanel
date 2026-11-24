//! The panel's own backup (T09.08): the control plane's database (with the audit log), its
//! configuration, the files its configuration points at (but **never the master key**) and
//! the node CA, in one encrypted archive.
//!
//! # The key
//!
//! A tenant backup's key is sealed with the master key and kept in the database, which a
//! lost panel no longer has. The system backup is therefore encrypted with a key **derived
//! from the master key** ([`derive_key`]): whoever keeps the master key (an operator's
//! offline copy, as always) can open every system backup, and nobody else can. The master
//! key itself is not in the archive.
//!
//! # The archive
//!
//! A tar, compressed with zstd, cut into authenticated chunks (`builtpanel_backup`):
//!
//! | Entry | What |
//! |---|---|
//! | `info.json` | when, which version, which files |
//! | `database.dump` | `pg_dump --format=custom` of the control database |
//! | `config/control.toml` | the control plane's configuration file |
//! | `secrets/<name>` | the files the configuration names (database URL, Redis URL, DNS key) |
//! | `pki/node_ca.crt`, `pki/node_ca.key` | the node CA |
//! | `license/installation.id`, `license/installation.key` | the installation's licensing identity |
//!
//! [`unpack`] writes the entries back to a folder (mode 0600), refusing any name that is not
//! one of these.

use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use builtpanel_backup::{KEY_LEN, StreamReader, StreamWriter};
use sha2::{Digest, Sha256};

/// Why the system backup failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Reading, writing or packing.
    #[error("{0}")]
    Io(String),
    /// `pg_dump` is missing or failed.
    #[error("pg_dump: {0}")]
    Dump(String),
    /// The database URL cannot be used.
    #[error("the database URL is not usable")]
    Url,
    /// The archive holds something that is not part of the layout.
    #[error("the archive has an entry that does not belong: {0}")]
    BadEntry(String),
}

fn io(what: &str, e: &std::io::Error) -> Error {
    Error::Io(format!("{what}: {}", e.kind()))
}

/// The key system backups are encrypted with: SHA-256 of a fixed label and the master key's
/// text (as it is in its file, whitespace around it ignored).
#[must_use]
pub fn derive_key(master_key_text: &str) -> [u8; KEY_LEN] {
    let mut h = Sha256::new();
    h.update(b"builtpanel/system-backup/v1\0");
    h.update(master_key_text.trim().as_bytes());
    h.finalize().into()
}

/// How to reach a PostgreSQL server, as `pg_dump` wants it.
#[derive(Clone, PartialEq, Eq)]
pub struct Connection {
    /// Host.
    pub host: String,
    /// Port.
    pub port: String,
    /// User.
    pub user: String,
    /// Password (given to `pg_dump` through its environment, never its arguments).
    pub password: String,
    /// Database.
    pub database: String,
    /// `sslmode` of the URL, if given.
    pub sslmode: Option<String>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("host", &self.host)
            .field("database", &self.database)
            .finish_non_exhaustive()
    }
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

impl Connection {
    /// Reads `postgres://user:password@host:port/database?sslmode=...`.
    ///
    /// # Errors
    /// [`Error::Url`].
    pub fn parse(url: &str) -> Result<Self, Error> {
        let rest = url
            .strip_prefix("postgres://")
            .or_else(|| url.strip_prefix("postgresql://"))
            .ok_or(Error::Url)?;
        let (authority, tail) = rest.split_once('/').ok_or(Error::Url)?;
        let (login, hostport) = authority.rsplit_once('@').ok_or(Error::Url)?;
        let (user, password) = login.split_once(':').unwrap_or((login, ""));
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => (h, p),
            _ => (hostport, "5432"),
        };
        let (database, query) = tail.split_once('?').unwrap_or((tail, ""));
        let sslmode = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("sslmode="))
            .map(str::to_owned);
        let this = Self {
            host: percent_decode(host.trim_matches(['[', ']'])).ok_or(Error::Url)?,
            port: port.to_owned(),
            user: percent_decode(user).ok_or(Error::Url)?,
            password: percent_decode(password).ok_or(Error::Url)?,
            database: percent_decode(database).ok_or(Error::Url)?,
            sslmode,
        };
        if this.host.is_empty()
            || this.user.is_empty()
            || this.database.is_empty()
            || this.port.parse::<u16>().is_err()
        {
            return Err(Error::Url);
        }
        Ok(this)
    }
}

/// Dumps the database to `out` (custom format: restorable with `pg_restore`, tables in any
/// order). The password goes through the environment; the dump file is made private.
///
/// # Errors
/// [`Error::Dump`] when `pg_dump` is missing or fails.
pub fn pg_dump(connection: &Connection, out: &Path) -> Result<(), Error> {
    let mut command = Command::new("pg_dump");
    command
        .args(["--format=custom", "--no-owner", "--no-privileges", "--file"])
        .arg(out)
        .args(["--host", &connection.host, "--port", &connection.port])
        .args([
            "--username",
            &connection.user,
            "--dbname",
            &connection.database,
        ])
        .arg("--no-password")
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("PGPASSWORD", &connection.password)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(mode) = &connection.sslmode {
        command.env("PGSSLMODE", mode);
    }
    let child = command.spawn().map_err(|e| {
        Error::Dump(format!(
            "cannot start it ({}): is postgresql-client installed?",
            e.kind()
        ))
    })?;
    let output = child
        .wait_with_output()
        .map_err(|e| Error::Dump(e.kind().to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        let said = String::from_utf8_lossy(&output.stderr);
        Err(Error::Dump(
            said.lines()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(300)
                .collect(),
        ))
    }
}

/// What goes into the archive.
#[derive(Debug, Clone)]
pub struct Inputs {
    /// The dump made by [`pg_dump`].
    pub dump: PathBuf,
    /// The control plane's configuration file.
    pub config_file: Option<PathBuf>,
    /// Files the configuration names, by the name they get under `secrets/`.
    pub secrets: Vec<(String, PathBuf)>,
    /// The node CA's folder (`node_ca.crt` and `node_ca.key` are taken from it).
    pub ca_dir: Option<PathBuf>,
    /// The installation's identity folder for licensing (`installation.id` and
    /// `installation.key`), so that a restore is the same installation and does not use
    /// another activation.
    pub license_dir: Option<PathBuf>,
    /// When it is made (Unix seconds).
    pub created_at: i64,
    /// The panel's version.
    pub version: String,
}

/// What was packed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Packed {
    /// The entries, by name.
    pub entries: Vec<String>,
    /// Bytes of contents (before compression).
    pub bytes: u64,
}

fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !name.starts_with('.')
}

/// Packs everything in `inputs` into `out`, encrypted with `key`.
///
/// # Errors
/// [`Error::Io`].
pub fn pack(inputs: &Inputs, key: &[u8; KEY_LEN], out: impl Write) -> Result<Packed, Error> {
    let e = |what: &str| {
        let what = what.to_owned();
        move |err: std::io::Error| io(&what, &err)
    };
    let encrypted = StreamWriter::new(out, key).map_err(e("cannot start"))?;
    let compressed = zstd::stream::Encoder::new(encrypted, 3).map_err(e("cannot compress"))?;
    let mut tar = tar::Builder::new(compressed);
    tar.mode(tar::HeaderMode::Deterministic);
    let mut packed = Packed::default();
    let add_file = |tar: &mut tar::Builder<_>,
                    packed: &mut Packed,
                    name: &str,
                    path: &Path|
     -> Result<(), Error> {
        let mut file = File::open(path).map_err(e(&format!("cannot read {name}")))?;
        let size = file
            .metadata()
            .map_err(e(&format!("cannot read {name}")))?
            .len();
        let mut header = tar::Header::new_gnu();
        header.set_size(size);
        header.set_mode(0o600);
        header.set_mtime(u64::try_from(inputs.created_at).unwrap_or(0));
        header.set_entry_type(tar::EntryType::Regular);
        // A file that grows while it is read is cut to what the header says.
        tar.append_data(&mut header, name, (&mut file).take(size))
            .map_err(e(&format!("cannot pack {name}")))?;
        packed.entries.push(name.to_owned());
        packed.bytes += size;
        Ok(())
    };
    let mut files: Vec<(String, PathBuf)> = vec![("database.dump".into(), inputs.dump.clone())];
    if let Some(config) = &inputs.config_file {
        files.push(("config/control.toml".into(), config.clone()));
    }
    for (name, path) in &inputs.secrets {
        if !plain_name(name) {
            return Err(Error::BadEntry(name.clone()));
        }
        files.push((format!("secrets/{name}"), path.clone()));
    }
    if let Some(dir) = &inputs.ca_dir {
        for name in ["node_ca.crt", "node_ca.key"] {
            let path = dir.join(name);
            if path.is_file() {
                files.push((format!("pki/{name}"), path));
            }
        }
    }
    if let Some(dir) = &inputs.license_dir {
        for name in ["installation.id", "installation.key"] {
            let path = dir.join(name);
            if path.is_file() {
                files.push((format!("license/{name}"), path));
            }
        }
    }
    let info = serde_json::json!({
        "format": 1,
        "created_at": inputs.created_at,
        "version": inputs.version,
        "entries": files.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
    });
    let info =
        serde_json::to_vec_pretty(&info).map_err(|_| Error::Io("cannot write info".into()))?;
    let mut header = tar::Header::new_gnu();
    header.set_size(info.len() as u64);
    header.set_mode(0o600);
    header.set_mtime(u64::try_from(inputs.created_at).unwrap_or(0));
    tar.append_data(&mut header, "info.json", &info[..])
        .map_err(e("cannot pack info"))?;
    packed.entries.push("info.json".into());
    for (name, path) in &files {
        add_file(&mut tar, &mut packed, name, path)?;
    }
    let compressed = tar.into_inner().map_err(e("cannot finish"))?;
    let encrypted = compressed.finish().map_err(e("cannot finish"))?;
    encrypted.finish().map_err(e("cannot finish"))?;
    Ok(packed)
}

fn allowed_entry(name: &str) -> bool {
    match name {
        "info.json"
        | "database.dump"
        | "config/control.toml"
        | "pki/node_ca.crt"
        | "pki/node_ca.key"
        | "license/installation.id"
        | "license/installation.key" => true,
        _ => name.strip_prefix("secrets/").is_some_and(plain_name),
    }
}

/// Writes the entries of an archive made by [`pack`] under `dir` (made if missing, private),
/// and returns their names.
///
/// # Errors
/// A wrong key or a damaged archive (as [`Error::Io`]), or an entry that is not part of the
/// layout ([`Error::BadEntry`]).
pub fn unpack(key: &[u8; KEY_LEN], input: impl Read, dir: &Path) -> Result<Vec<String>, Error> {
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
    let bad = |e: std::io::Error| io("cannot open the archive (wrong key, or damaged)", &e);
    let plain = zstd::stream::Decoder::new(StreamReader::new(input, key)).map_err(bad)?;
    let mut archive = tar::Archive::new(plain);
    let mut made = Vec::new();
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| io("cannot make the folder", &e))?;
    for entry in archive.entries().map_err(bad)? {
        let mut entry = entry.map_err(bad)?;
        let name = entry.path().map_err(bad)?.to_string_lossy().into_owned();
        if !allowed_entry(&name) || entry.header().entry_type() != tar::EntryType::Regular {
            return Err(Error::BadEntry(name));
        }
        let target = dir.join(&name);
        if let Some(parent) = target.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(|e| io("cannot make a folder", &e))?;
        }
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&target)
            .map_err(|e| io(&format!("cannot write {name}"), &e))?;
        std::io::copy(&mut entry, &mut out).map_err(bad)?;
        made.push(name);
    }
    Ok(made)
}

/// The part of the archive at the destination.
pub const PART: &str = "system.tar.zst.enc";

/// Everything one system backup needs.
#[derive(Debug, Clone)]
pub struct Job {
    /// The backup's id (32 hex digits): the folder at the destination.
    pub id: String,
    /// When it was asked for (Unix seconds).
    pub created_at: i64,
    /// The destination's kind and settings, and its secrets as JSON (`null` for none).
    pub kind: String,
    /// The destination's plain settings.
    pub settings: serde_json::Value,
    /// The destination's secrets.
    pub secrets: serde_json::Value,
    /// The control database.
    pub connection: Connection,
    /// The configuration file, if it is a file.
    pub config_file: Option<PathBuf>,
    /// Files the configuration names (see [`Inputs`]).
    pub secret_files: Vec<(String, PathBuf)>,
    /// The CA folder.
    pub ca_dir: Option<PathBuf>,
    /// The licensing identity folder.
    pub license_dir: Option<PathBuf>,
    /// A private folder for the dump and the archive while they are made.
    pub scratch: PathBuf,
    /// The key ([`derive_key`]).
    pub key: [u8; KEY_LEN],
    /// The panel's version.
    pub version: String,
}

/// What a finished system backup is.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    /// Everything stored, manifest included.
    pub total_bytes: i64,
    /// The parts, as the `backups` row keeps them.
    pub parts: serde_json::Value,
    /// SHA-256 of the stored manifest.
    pub manifest_sha256: String,
}

struct HashRead<R: Read> {
    inner: R,
    hasher: Sha256,
    size: u64,
}

impl<R: Read> Read for HashRead<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.size += n as u64;
        Ok(n)
    }
}

/// Dumps the database, packs everything, stores the archive and then the manifest at the
/// destination; what was stored is removed again when a later step fails. Blocking.
///
/// # Errors
/// A message (never a secret) for the row of the backup.
pub fn run_job(job: &Job) -> Result<Done, String> {
    use builtpanel_backup::{
        Manifest, Part, PartKind,
        manifest::{Counting, hex},
    };
    use std::os::unix::fs::DirBuilderExt as _;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&job.scratch)
        .map_err(|e| format!("cannot make the scratch folder: {}", e.kind()))?;
    let (dump, archive) = (
        job.scratch.join(format!("{}.dump", job.id)),
        job.scratch.join(format!("{}.enc", job.id)),
    );
    let result = (|| -> Result<Done, String> {
        pg_dump(&job.connection, &dump).map_err(|e| e.to_string())?;
        let inputs = Inputs {
            dump: dump.clone(),
            config_file: job.config_file.clone(),
            secrets: job.secret_files.clone(),
            ca_dir: job.ca_dir.clone(),
            license_dir: job.license_dir.clone(),
            created_at: job.created_at,
            version: job.version.clone(),
        };
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&archive)
            .map_err(|e| format!("cannot make the archive: {}", e.kind()))?;
        pack(&inputs, &job.key, &mut out).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.kind().to_string())?;
        let _ = std::fs::remove_file(&dump);
        let dest = builtpanel_backup_dest::open::open(&job.kind, &job.settings, &job.secrets)
            .map_err(|e| e.to_string())?;
        let object = format!("{}/{PART}", job.id);
        let mut counted = HashRead {
            inner: File::open(&archive).map_err(|e| e.kind().to_string())?,
            hasher: Sha256::new(),
            size: 0,
        };
        dest.put(&object, &mut counted).map_err(|e| e.to_string())?;
        let (size, sha) = (counted.size, hex(&counted.hasher.finalize()));
        let part = Part {
            name: PART.into(),
            kind: PartKind::System,
            stored_bytes: size,
            sha256: sha,
            database: None,
        };
        let manifest_stored = (|| -> Result<(u64, String), String> {
            let mut manifest = Manifest::new(&job.id, "panel", job.created_at);
            manifest.parts = vec![part.clone()];
            manifest.check().map_err(|e| e.to_string())?;
            let json = manifest.to_json().map_err(|e| e.to_string())?;
            let mut w = Counting::new(Vec::new());
            {
                let mut enc = StreamWriter::new(&mut w, &job.key).map_err(|e| e.to_string())?;
                enc.write_all(&json).map_err(|e| e.to_string())?;
                enc.finish().map_err(|e| e.to_string())?;
            }
            let (sealed, size, sha) = w.finish();
            dest.put(&format!("{}/manifest.json.enc", job.id), &mut &sealed[..])
                .map_err(|e| e.to_string())?;
            Ok((size, sha))
        })();
        match manifest_stored {
            Ok((manifest_size, manifest_sha)) => Ok(Done {
                total_bytes: i64::try_from(size + manifest_size).unwrap_or(i64::MAX),
                parts: serde_json::json!([{
                    "name": part.name, "kind": "system", "stored_bytes": part.stored_bytes,
                    "sha256": part.sha256, "database": null,
                }]),
                manifest_sha256: manifest_sha,
            }),
            Err(e) => {
                let _ = dest.delete(&object);
                Err(e)
            }
        }
    })();
    let _ = std::fs::remove_file(&dump);
    let _ = std::fs::remove_file(&archive);
    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_key_depends_on_the_master_key_only_and_ignores_whitespace() {
        assert_eq!(derive_key("abc"), derive_key(" abc\n"));
        assert_ne!(derive_key("abc"), derive_key("abd"));
    }

    #[test]
    fn a_database_url_is_read_with_its_escapes() {
        let c = Connection::parse("postgres://bp:p%40ss%2Fw@db.example:6543/panel?sslmode=require")
            .unwrap();
        assert_eq!(
            (
                c.host.as_str(),
                c.port.as_str(),
                c.user.as_str(),
                c.password.as_str(),
                c.database.as_str()
            ),
            ("db.example", "6543", "bp", "p@ss/w", "panel")
        );
        assert_eq!(c.sslmode.as_deref(), Some("require"));
        let c = Connection::parse("postgresql://u:p@localhost/db").unwrap();
        assert_eq!((c.port.as_str(), c.sslmode.clone()), ("5432", None));
        for bad in [
            "mysql://u@h/d",
            "postgres://h/d",
            "postgres://u:p@h",
            "postgres://u:p@h:x/d",
            "postgres://u:p@h/",
        ] {
            assert!(Connection::parse(bad).is_err(), "{bad}");
        }
        let secret = Connection::parse("postgres://u:hunter2@h/d").unwrap();
        assert!(!format!("{secret:?}").contains("hunter2"));
    }

    #[test]
    fn an_archive_unpacks_the_same_and_only_with_its_key() {
        let dir = std::env::temp_dir().join(format!("bp-sysbk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("ca")).unwrap();
        std::fs::write(dir.join("dump"), b"PGDMP-pretend").unwrap();
        std::fs::write(dir.join("control.toml"), "listen = '127.0.0.1:1'").unwrap();
        std::fs::write(dir.join("db.url"), "postgres://secret").unwrap();
        std::fs::write(dir.join("ca/node_ca.crt"), "CERT").unwrap();
        std::fs::write(dir.join("ca/node_ca.key"), "KEY").unwrap();
        // Not part of the backup, even though it lies in the CA folder.
        std::fs::write(dir.join("ca/master.key"), "MASTER").unwrap();
        // The licensing identity: both files are kept; anything else in the folder is not.
        std::fs::create_dir_all(dir.join("lic")).unwrap();
        std::fs::write(dir.join("lic/installation.id"), "ID").unwrap();
        std::fs::write(dir.join("lic/installation.key"), "LICENSE-PRIVATE").unwrap();
        std::fs::write(dir.join("lic/other.txt"), "OTHER").unwrap();
        let inputs = Inputs {
            dump: dir.join("dump"),
            config_file: Some(dir.join("control.toml")),
            secrets: vec![("database_url".into(), dir.join("db.url"))],
            ca_dir: Some(dir.join("ca")),
            license_dir: Some(dir.join("lic")),
            created_at: 1_700_000_000,
            version: "0.1.0".into(),
        };
        let key = derive_key("master");
        let mut stored = Vec::new();
        let packed = pack(&inputs, &key, &mut stored).unwrap();
        assert!(!stored.windows(4).any(|w| w == b"CERT" || w == b"KEY\0"));
        assert!(
            !stored.windows(15).any(|w| w == b"LICENSE-PRIVATE"),
            "encrypted"
        );
        assert_eq!(packed.entries.len(), 8, "{packed:?}");
        let out = dir.join("out");
        let names = unpack(&key, &stored[..], &out).unwrap();
        assert!(
            names.contains(&"pki/node_ca.key".to_owned())
                && !names.iter().any(|n| n.contains("master"))
        );
        assert_eq!(
            std::fs::read(out.join("database.dump")).unwrap(),
            b"PGDMP-pretend"
        );
        assert_eq!(
            std::fs::read_to_string(out.join("secrets/database_url")).unwrap(),
            "postgres://secret"
        );
        assert!(!out.join("pki/master.key").exists());
        assert_eq!(
            std::fs::read_to_string(out.join("license/installation.id")).unwrap(),
            "ID"
        );
        assert_eq!(
            std::fs::read_to_string(out.join("license/installation.key")).unwrap(),
            "LICENSE-PRIVATE"
        );
        assert!(!out.join("license/other.txt").exists());
        // Another key opens nothing.
        assert!(unpack(&derive_key("other"), &stored[..], &dir.join("out2")).is_err());
        // A secret named like a path is refused.
        let mut bad = inputs.clone();
        bad.secrets = vec![("../x".into(), dir.join("db.url"))];
        assert!(pack(&bad, &key, Vec::new()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
