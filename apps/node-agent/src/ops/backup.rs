//! `create_backup` (T09.04): an account's files, databases and setup data, packed,
//! compressed, encrypted and stored at a destination.
//!
//! * The **files** are packed by the privileged helper as the tenant's own user (see
//!   `builtpanel_fileops::backup`) into a staged file in the agent's transfer folder, then
//!   stored and the staged file removed.
//! * Each **database** is dumped by `mariadb-dump` in the database container as a
//!   temporary read-only account (a consistent snapshot: `--single-transaction`) and flows
//!   dump -> zstd -> encryption -> the destination without touching the disk.
//! * **DNS** and **meta** are small JSON documents, encrypted like the rest.
//! * The **manifest** lists every part with its stored size and SHA-256 and is stored last:
//!   a backup without a manifest is not a backup, and a failure removes what was stored.
//!
//! The key and the destination's credentials arrive with the one operation that needs them.

use std::{
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt as _,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use builtpanel_backup::{
    KEY_LEN, Manifest, Part, PartKind, StreamReader, StreamWriter,
    manifest::{Counting, hex},
};
use builtpanel_backup_dest::Destination;
use builtpanel_node_protocol::{
    AgentError, BackupPartInfo, BackupTarget, DbPassword, Operation, Output,
};
use builtpanel_priv_helper::protocol::Action;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    db,
    sqlexport::{MAX_DURATION, drop_account, reader_account},
    sqlimport::{failed, invalid},
    tenant::{TenantContext, ask_helper, docker_error},
};

/// A destination shared by the pieces of one backup (used one object at a time).
pub(super) type Shared = Arc<Mutex<Box<dyn Destination>>>;

fn transfers_dir(context: &TenantContext) -> PathBuf {
    std::env::var_os("BUILTPANEL_AGENT_TRANSFERS").map_or_else(
        || context.vhosts_root.with_file_name("transfers"),
        PathBuf::from,
    )
}

/// Opens the destination a backup goes to (blocking: an SFTP login happens here).
pub(super) fn open_destination(
    target: &BackupTarget,
    credentials: Option<&DbPassword>,
) -> Result<Box<dyn Destination>, AgentError> {
    let secrets: serde_json::Value = credentials
        .map(|c| {
            serde_json::from_str(&c.0)
                .map_err(|_| invalid("the destination: unreadable credentials"))
        })
        .transpose()?
        .unwrap_or(serde_json::Value::Null);
    builtpanel_backup_dest::open::open(&target.kind, &target.settings, &secrets).map_err(
        |e| match e {
            builtpanel_backup_dest::open::OpenError::Settings(_) => invalid(&e.to_string()),
            builtpanel_backup_dest::open::OpenError::Destination(_) => {
                failed(&e.to_string()).retryable()
            }
        },
    )
}

/// Reads another reader and counts and hashes what passes (the stored size and checksum).
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

/// Stores what `reader` gives under `name`; returns its stored size and SHA-256.
fn store(dest: &Shared, name: &str, reader: impl Read) -> Result<(u64, String), AgentError> {
    let mut counted = HashRead {
        inner: reader,
        hasher: Sha256::new(),
        size: 0,
    };
    let guard = dest
        .lock()
        .map_err(|_| failed("the destination is not usable"))?;
    guard
        .put(name, &mut counted)
        .map_err(|e| failed(&e.to_string()).retryable())?;
    Ok((counted.size, hex(&counted.hasher.finalize())))
}

/// An encrypted JSON document, stored.
fn json_part(
    dest: &Shared,
    name: &str,
    key: &[u8; KEY_LEN],
    doc: &serde_json::Value,
) -> Result<(u64, String), AgentError> {
    let bytes = serde_json::to_vec(doc).map_err(|_| invalid("a document cannot be encoded"))?;
    let mut w = StreamWriter::new(Vec::new(), key).map_err(|e| failed(&e.to_string()))?;
    w.write_all(&bytes).map_err(|e| failed(&e.to_string()))?;
    let sealed = w.finish().map_err(|e| failed(&e.to_string()))?;
    store(dest, name, &sealed[..])
}

/// A writer into a bounded channel, so a stage can run in a thread of its own.
struct PipeWriter(std::sync::mpsc::SyncSender<Vec<u8>>);

impl Write for PipeWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.send(data.to_vec()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "nothing is taking it")
        })?;
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct PipeReader {
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    at: usize,
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.at >= self.buf.len() {
            match self.rx.recv() {
                Ok(next) => {
                    self.buf = next;
                    self.at = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.at);
        out[..n].copy_from_slice(&self.buf[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// The key from its 64 hex digits.
pub(super) fn parse_key(hex_key: &DbPassword) -> Result<[u8; KEY_LEN], AgentError> {
    let text = hex_key.0.trim();
    if text.len() != KEY_LEN * 2 {
        return Err(invalid("the key is not 64 hex digits"));
    }
    let mut key = [0u8; KEY_LEN];
    for (i, b) in key.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
            .map_err(|_| invalid("the key is not hex"))?;
    }
    Ok(key)
}

/// One database, dumped, compressed, encrypted and stored while it is made.
async fn database_part(
    context: &TenantContext,
    dest: &Shared,
    object: String,
    database: &str,
    key: [u8; KEY_LEN],
) -> Result<(u64, String), AgentError> {
    let tag = Uuid::now_v7().simple().to_string()[..12].to_owned();
    let (account, password) = reader_account(context, database, &tag).await?;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
    let (pipe_tx, pipe_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(8);
    let packer = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let encrypted = StreamWriter::new(PipeWriter(pipe_tx), &key)?;
        let mut zst = zstd::stream::Encoder::new(encrypted, 3)?;
        while let Some(piece) = rx.blocking_recv() {
            zst.write_all(&piece)?;
        }
        zst.finish()?.finish()?;
        Ok(())
    });
    let storer = {
        let (dest, object) = (dest.clone(), object.clone());
        tokio::task::spawn_blocking(move || {
            store(
                &dest,
                &object,
                PipeReader {
                    rx: pipe_rx,
                    buf: Vec::new(),
                    at: 0,
                },
            )
        })
    };
    let container = builtpanel_docker_manager::Name::parse(db::CONTAINER)
        .map_err(|e| failed(&e.to_string()))?;
    let ran = context
        .docker
        .dump_database(&container, database, &account, &password, tx, MAX_DURATION)
        .await;
    drop_account(context, &account).await;
    let packed = packer
        .await
        .map_err(|_| failed("the packing was cancelled"))?;
    let stored = storer
        .await
        .map_err(|_| failed("the storing was cancelled"))?;
    let (code, stderr) = ran.map_err(|e| docker_error(&e))?;
    if code != 0 {
        let said: String = stderr.lines().take(3).collect::<Vec<_>>().join(" ");
        return Err(failed(&format!(
            "the dump of a database failed (exit {code}): {}",
            said.chars().take(300).collect::<String>()
        )));
    }
    packed.map_err(|e| failed(&format!("cannot pack the dump: {}", e.kind())))?;
    stored
}

/// `create_backup`.
///
/// # Errors
/// [`AgentError`]: bad arguments (not retryable); the destination, the database server or
/// the helper failing (retryable). On a failure what was stored is removed.
pub async fn create_backup(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::CreateBackup {
        tenant_id,
        uid,
        backup_id,
        account,
        target,
        credentials,
        key,
        databases,
        db_users,
        files_since,
        dns,
        mut meta,
        created_at,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let tenant =
        builtpanel_domain::TenantUsername::parse(&account).map_err(|e| invalid(&e.to_string()))?;
    if db_users.len() > 200 {
        return Err(invalid("too many database users"));
    }
    for d in &databases {
        builtpanel_domain::databases::check_identifier(d).map_err(|e| invalid(&e.to_string()))?;
    }
    if databases.len() > 100 {
        return Err(invalid("too many databases"));
    }
    if serde_json::to_vec(&dns).map_or(usize::MAX, |v| v.len()) > 256 * 1024
        || serde_json::to_vec(&meta).map_or(usize::MAX, |v| v.len()) > 256 * 1024
    {
        return Err(invalid("the documents are too big"));
    }
    // The database users' passwords, as the servers hold them, go into the setup data (which is
    // encrypted like the rest), so that a transfer makes the users again with the same ones.
    if !db_users.is_empty() {
        let mut carried = Vec::new();
        for user in &db_users {
            builtpanel_domain::databases::check_identifier(&user.name)
                .map_err(|e| invalid(&e.to_string()))?;
            let credential = match user.engine {
                builtpanel_node_protocol::DbEngine::Mysql => {
                    db::user_hash(&context, &tenant, &user.name).await?
                }
                builtpanel_node_protocol::DbEngine::Postgres => {
                    super::pg::user_verifier(&context, &user.name).await?
                }
            };
            if let Some(credential) = credential {
                carried.push(serde_json::json!({
                    "name": user.name,
                    "engine": user.engine,
                    "credential": credential,
                }));
            }
        }
        if let Some(object) = meta.as_object_mut() {
            object.insert("db_users".to_owned(), serde_json::Value::Array(carried));
        }
    }
    if serde_json::to_vec(&meta).map_or(usize::MAX, |v| v.len()) > 256 * 1024 {
        return Err(invalid("the documents are too big"));
    }
    let key_text = key.ok_or_else(|| invalid("no key"))?;
    let key = parse_key(&key_text)?;
    let id = backup_id.simple().to_string();
    let dest: Shared = {
        let (target, credentials) = (target.clone(), credentials.clone());
        let opened =
            tokio::task::spawn_blocking(move || open_destination(&target, credentials.as_ref()))
                .await
                .map_err(|_| failed("the destination work was cancelled"))??;
        Arc::new(Mutex::new(opened))
    };
    let mut parts: Vec<Part> = Vec::new();
    let result = make_parts(
        &context,
        &dest,
        &id,
        tenant_id,
        uid,
        &key_text,
        key,
        &databases,
        &dns,
        &meta,
        files_since,
        &mut parts,
    )
    .await;
    let finish = match result {
        Ok(()) => finish_manifest(&dest, &id, &account, created_at, &parts, key).await,
        Err(e) => Err(e),
    };
    match finish {
        Ok((manifest_sha, manifest_size)) => {
            let size = parts.iter().map(|p| p.stored_bytes).sum::<u64>() + manifest_size;
            Ok(Output::BackupCreated {
                manifest_sha256: manifest_sha,
                size_bytes: size,
                parts: parts
                    .into_iter()
                    .map(|p| BackupPartInfo {
                        name: p.name,
                        kind: format!("{:?}", p.kind).to_ascii_lowercase(),
                        stored_bytes: p.stored_bytes,
                        sha256: p.sha256,
                        database: p.database,
                    })
                    .collect(),
            })
        }
        Err(e) => {
            // Nothing half done stays at the destination.
            let dest = dest.clone();
            let names: Vec<String> = parts
                .iter()
                .map(|p| format!("{id}/{}", p.name))
                .chain([format!("{id}/manifest.json.enc")])
                .collect();
            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(d) = dest.lock() {
                    for n in names {
                        let _ = d.delete(&n);
                    }
                }
            })
            .await;
            Err(e)
        }
    }
}

/// The tenant's files, packed by the helper into a staged file, stored as `object`.
#[allow(clippy::too_many_arguments)]
async fn files_part(
    context: &TenantContext,
    dest: &Shared,
    object: &str,
    staged_name: &str,
    tenant_id: Uuid,
    uid: u32,
    key_text: &DbPassword,
    since: Option<i64>,
) -> Result<(u64, String), AgentError> {
    let dir = transfers_dir(context);
    std::fs::create_dir_all(&dir)
        .map_err(|e| failed(&format!("cannot make the folder: {}", e.kind())).retryable())?;
    let staged = dir.join(staged_name);
    let _ = std::fs::remove_file(&staged);
    let socket = context.helper_socket.clone();
    let action = Action::BackupFiles {
        tenant_id: tenant_id.to_string(),
        uid,
        staged: staged_name.to_owned(),
        key: key_text.0.trim().to_owned(),
        since,
    };
    tokio::task::spawn_blocking(move || ask_helper(&socket, action))
        .await
        .map_err(|_| failed("the packing was cancelled"))??;
    let stored = {
        let (dest, staged2, object) = (dest.clone(), staged.clone(), object.to_owned());
        tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(&staged2)
                .map_err(|e| failed(&format!("cannot read the packed files: {}", e.kind())))?;
            store(&dest, &object, file)
        })
        .await
        .map_err(|_| failed("the storing was cancelled"))?
    };
    let _ = std::fs::remove_file(&staged);
    stored
}

#[allow(clippy::too_many_arguments)]
async fn make_parts(
    context: &Arc<TenantContext>,
    dest: &Shared,
    id: &str,
    tenant_id: Uuid,
    uid: u32,
    key_text: &DbPassword,
    key: [u8; KEY_LEN],
    databases: &[String],
    dns: &serde_json::Value,
    meta: &serde_json::Value,
    since: Option<i64>,
    parts: &mut Vec<Part>,
) -> Result<(), AgentError> {
    // 1. The files.
    let name = "files.tar.zst.enc";
    let (size, sha) = files_part(
        context,
        dest,
        &format!("{id}/{name}"),
        &format!("{id}.bk"),
        tenant_id,
        uid,
        key_text,
        since,
    )
    .await?;
    parts.push(Part {
        name: name.into(),
        kind: PartKind::Files,
        stored_bytes: size,
        sha256: sha,
        database: None,
    });
    // 2. The databases.
    for database in databases {
        let name = format!("db-{database}.sql.zst.enc");
        let (size, sha) =
            database_part(context, dest, format!("{id}/{name}"), database, key).await?;
        parts.push(Part {
            name,
            kind: PartKind::Database,
            stored_bytes: size,
            sha256: sha,
            database: Some(database.clone()),
        });
    }
    // 3. DNS and setup data.
    for (name, kind, doc) in [
        ("dns.json.enc", PartKind::Dns, dns),
        ("meta.json.enc", PartKind::Meta, meta),
    ] {
        let (dest2, object, doc) = (dest.clone(), format!("{id}/{name}"), doc.clone());
        let (size, sha) =
            tokio::task::spawn_blocking(move || json_part(&dest2, &object, &key, &doc))
                .await
                .map_err(|_| failed("the storing was cancelled"))??;
        parts.push(Part {
            name: name.into(),
            kind,
            stored_bytes: size,
            sha256: sha,
            database: None,
        });
    }
    Ok(())
}

async fn finish_manifest(
    dest: &Shared,
    id: &str,
    account: &str,
    created_at: i64,
    parts: &[Part],
    key: [u8; KEY_LEN],
) -> Result<(String, u64), AgentError> {
    let mut manifest = Manifest::new(id, account, created_at);
    manifest.parts = parts.to_vec();
    manifest.check().map_err(|e| failed(&e.to_string()))?;
    let json = manifest.to_json().map_err(|e| failed(&e.to_string()))?;
    let (dest, object) = (dest.clone(), format!("{id}/manifest.json.enc"));
    tokio::task::spawn_blocking(move || {
        let mut w = Counting::new(Vec::new());
        {
            let mut enc = StreamWriter::new(&mut w, &key).map_err(|e| failed(&e.to_string()))?;
            enc.write_all(&json).map_err(|e| failed(&e.to_string()))?;
            enc.finish().map_err(|e| failed(&e.to_string()))?;
        }
        let (sealed, size, sha) = w.finish();
        store(&dest, &object, &sealed[..])?;
        Ok((sha, size))
    })
    .await
    .map_err(|_| failed("the storing was cancelled"))?
}

/// Reads the backup's manifest (decrypted) from the destination.
fn read_manifest(dest: &Shared, id: &str, key: &[u8; KEY_LEN]) -> Result<Manifest, AgentError> {
    let guard = dest
        .lock()
        .map_err(|_| failed("the destination is not usable"))?;
    let raw = guard
        .get(&format!("{id}/manifest.json.enc"))
        .map_err(|e| failed(&format!("the manifest cannot be read: {e}")).retryable())?;
    let manifest = Manifest::from_reader(StreamReader::new(raw, key))
        .map_err(|e| invalid(&format!("the backup's manifest is not good: {e}")))?;
    if manifest.backup_id != id {
        return Err(invalid("the manifest belongs to another backup"));
    }
    Ok(manifest)
}

/// Reads one stored part and checks it against the manifest (size and SHA-256).
fn verify(dest: &Shared, id: &str, manifest: &Manifest, name: &str) -> Result<(), AgentError> {
    let guard = dest
        .lock()
        .map_err(|_| failed("the destination is not usable"))?;
    let stored = guard
        .get(&format!("{id}/{name}"))
        .map_err(|_| invalid(&format!("the part {name} is missing")))?;
    manifest
        .verify_part(name, stored)
        .map_err(|e| invalid(&format!("the backup is damaged: {e}")))
}

/// Puts a database part into a plain dump file (decrypted and decompressed), at most
/// `limit` bytes.
fn unpack_dump(
    dest: &Shared,
    object: &str,
    key: &[u8; KEY_LEN],
    path: &std::path::Path,
) -> Result<(), AgentError> {
    let guard = dest
        .lock()
        .map_err(|_| failed("the destination is not usable"))?;
    let stored = guard
        .get(object)
        .map_err(|e| failed(&format!("cannot read a part: {e}")).retryable())?;
    let mut plain = zstd::stream::Decoder::new(StreamReader::new(stored, key))
        .map_err(|e| failed(&format!("cannot open the dump: {}", e.kind())))?;
    let mut out = std::fs::File::create(path)
        .map_err(|e| failed(&format!("cannot write the dump: {}", e.kind())))?;
    std::io::copy(&mut plain, &mut out)
        .map_err(|e| failed(&format!("cannot unpack the dump: {}", e.kind())))?;
    Ok(())
}

/// `restore_backup`: see [`Operation::RestoreBackup`].
///
/// # Errors
/// [`AgentError`]: bad arguments or a damaged backup (not retryable, nothing was touched);
/// the destination, the database server or the helper failing (retryable).
pub async fn restore_backup(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::RestoreBackup {
        tenant_id,
        uid,
        backup_id,
        restore_id,
        account,
        target,
        credentials,
        key,
        files,
        folder,
        databases,
        dns,
        snapshot,
        verify_only,
        create_databases,
        restore_db_users,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let tenant =
        builtpanel_domain::TenantUsername::parse(&account).map_err(|e| invalid(&e.to_string()))?;
    for d in &databases {
        builtpanel_domain::databases::check_identifier(d).map_err(|e| invalid(&e.to_string()))?;
    }
    if databases.len() > 100 {
        return Err(invalid("too many databases"));
    }
    if let Some(f) = &folder
        && (!files || f.len() > 1024 || f.contains(['\0', '\\']) || f.split('/').any(|p| p == ".."))
    {
        return Err(invalid("the folder is not usable"));
    }
    if !files && databases.is_empty() && !dns && !verify_only {
        return Err(invalid("nothing to restore"));
    }
    let key_text = key.ok_or_else(|| invalid("no key"))?;
    let key = parse_key(&key_text)?;
    let id = backup_id.simple().to_string();
    let dest: Shared = {
        let (target, credentials) = (target.clone(), credentials.clone());
        let opened =
            tokio::task::spawn_blocking(move || open_destination(&target, credentials.as_ref()))
                .await
                .map_err(|_| failed("the destination work was cancelled"))??;
        Arc::new(Mutex::new(opened))
    };
    // 1. The manifest, and every part that is needed, checked before anything is touched.
    let (manifest, needed) = {
        let (dest, id, account, databases) =
            (dest.clone(), id.clone(), account.clone(), databases.clone());
        tokio::task::spawn_blocking(move || -> Result<(Manifest, Vec<String>), AgentError> {
            let manifest = read_manifest(&dest, &id, &key)?;
            if manifest.account != account {
                return Err(invalid("the backup belongs to another account"));
            }
            let mut needed = Vec::new();
            for part in &manifest.parts {
                let wanted = match part.kind {
                    PartKind::Files => files || verify_only,
                    PartKind::Database => {
                        verify_only
                            || part
                                .database
                                .as_ref()
                                .is_some_and(|d| databases.contains(d))
                    }
                    PartKind::Dns => verify_only || dns,
                    PartKind::Meta => verify_only || restore_db_users,
                    PartKind::System => verify_only,
                };
                if wanted {
                    needed.push(part.name.clone());
                }
            }
            if files && !manifest.parts.iter().any(|p| p.kind == PartKind::Files) {
                return Err(invalid("the backup has no files"));
            }
            if dns && !manifest.parts.iter().any(|p| p.kind == PartKind::Dns) {
                return Err(invalid("the backup has no DNS records"));
            }
            for d in &databases {
                if !manifest
                    .parts
                    .iter()
                    .any(|p| p.database.as_deref() == Some(d.as_str()))
                {
                    return Err(invalid(&format!("the backup has no database {d}")));
                }
            }
            for name in &needed {
                verify(&dest, &id, &manifest, name)?;
            }
            Ok((manifest, needed))
        })
        .await
        .map_err(|_| failed("the check was cancelled"))??
    };
    let verified = u32::try_from(needed.len()).unwrap_or(u32::MAX);
    if verify_only {
        return Ok(Output::BackupRestored {
            verified_parts: verified,
            files_restored: false,
            databases: Vec::new(),
            snapshot: Vec::new(),
            dns: None,
        });
    }
    // The DNS records, read now so that a part that cannot be read stops everything before
    // anything is changed. The panel puts them back; the node holds no DNS.
    let dns_records = if dns {
        let name = manifest
            .parts
            .iter()
            .find(|p| p.kind == PartKind::Dns)
            .map(|p| p.name.clone())
            .ok_or_else(|| invalid("the backup has no DNS records"))?;
        let (dest, object) = (dest.clone(), format!("{id}/{name}"));
        Some(
            tokio::task::spawn_blocking(move || read_json_part(&dest, &object, &key))
                .await
                .map_err(|_| failed("the reading was cancelled"))??,
        )
    } else {
        None
    };
    // 2. The present state, kept (at the destination) so that a restore can be undone.
    let rid = restore_id.simple().to_string();
    let mut snap: Vec<BackupPartInfo> = Vec::new();
    if snapshot {
        let result = make_snapshot(
            &context, &dest, &rid, tenant_id, uid, &key_text, key, files, &databases, &mut snap,
        )
        .await;
        if let Err(e) = result {
            remove_snapshot(&dest, &rid, &snap).await;
            return Err(e);
        }
    }
    // 3. The files, over the present ones, as the tenant.
    if files {
        let name = manifest
            .parts
            .iter()
            .find(|p| p.kind == PartKind::Files)
            .map(|p| p.name.clone())
            .ok_or_else(|| invalid("the backup has no files"))?;
        restore_files_part(
            &context,
            &dest,
            &format!("{id}/{name}"),
            &rid,
            tenant_id,
            uid,
            &key_text,
            folder.as_deref(),
        )
        .await?;
    }
    // 4. The databases, replaced by their dumps (made first on a node that has none yet, with
    // the users the backup carries).
    if create_databases {
        for database in &databases {
            db::create_database(
                context.clone(),
                Operation::CreateDatabase {
                    name: database.clone(),
                    engine: builtpanel_node_protocol::DbEngine::Mysql,
                    dedicated: None,
                },
            )
            .await?;
        }
    }
    if restore_db_users {
        let name = manifest
            .parts
            .iter()
            .find(|p| p.kind == PartKind::Meta)
            .map(|p| p.name.clone())
            .ok_or_else(|| invalid("the backup has no setup data"))?;
        let (dest2, object) = (dest.clone(), format!("{id}/{name}"));
        let meta = tokio::task::spawn_blocking(move || read_json_part(&dest2, &object, &key))
            .await
            .map_err(|_| failed("the reading was cancelled"))??;
        for user in meta
            .get("db_users")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(user_name), Some(credential)) = (
                user.get("name").and_then(serde_json::Value::as_str),
                user.get("credential").and_then(serde_json::Value::as_str),
            ) else {
                continue;
            };
            builtpanel_domain::databases::check_identifier(user_name)
                .map_err(|e| invalid(&e.to_string()))?;
            // Only MariaDB users are carried by a restore today.
            if user
                .get("engine")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|e| e == "mysql")
            {
                db::set_user_hash(&context, &tenant, user_name, credential).await?;
            }
        }
    }
    for database in &databases {
        let name = manifest
            .parts
            .iter()
            .find(|p| p.database.as_deref() == Some(database.as_str()))
            .map(|p| p.name.clone())
            .ok_or_else(|| invalid("the backup has no such database"))?;
        let dir = super::sqlimport::imports_dir(&context);
        std::fs::create_dir_all(&dir)
            .map_err(|e| failed(&format!("cannot prepare the folder: {}", e.kind())).retryable())?;
        let path = dir.join(format!("{rid}-{database}.sql"));
        let unpacked = {
            let (dest, object, path) = (dest.clone(), format!("{id}/{name}"), path.clone());
            tokio::task::spawn_blocking(move || unpack_dump(&dest, &object, &key, &path))
                .await
                .map_err(|_| failed("the unpacking was cancelled"))?
        };
        let ran = match unpacked {
            Ok(()) => {
                super::sqlimport::import_file(&context, database, restore_id, &path, false).await
            }
            Err(e) => Err(e),
        };
        let _ = std::fs::remove_file(&path);
        ran?;
    }
    Ok(Output::BackupRestored {
        verified_parts: verified,
        files_restored: files,
        databases,
        snapshot: snap,
        dns: dns_records,
    })
}

/// The most a stored JSON part may be when it is read back (the part is a few kilobytes).
const JSON_PART_LIMIT: u64 = 16 * 1024 * 1024;

/// Reads a stored JSON part (decrypted).
fn read_json_part(
    dest: &Shared,
    object: &str,
    key: &[u8; KEY_LEN],
) -> Result<serde_json::Value, AgentError> {
    use std::io::Read;
    let guard = dest
        .lock()
        .map_err(|_| failed("the destination is not usable"))?;
    let stored = guard
        .get(object)
        .map_err(|e| failed(&format!("cannot read a part: {e}")).retryable())?;
    let mut text = Vec::new();
    StreamReader::new(stored, key)
        .take(JSON_PART_LIMIT + 1)
        .read_to_end(&mut text)
        .map_err(|_| invalid("the DNS part cannot be opened"))?;
    if text.len() as u64 > JSON_PART_LIMIT {
        return Err(invalid("the DNS part is too big"));
    }
    serde_json::from_slice(&text).map_err(|_| invalid("the DNS part is not good"))
}

#[allow(clippy::too_many_arguments)]
async fn make_snapshot(
    context: &Arc<TenantContext>,
    dest: &Shared,
    rid: &str,
    tenant_id: Uuid,
    uid: u32,
    key_text: &DbPassword,
    key: [u8; KEY_LEN],
    files: bool,
    databases: &[String],
    snap: &mut Vec<BackupPartInfo>,
) -> Result<(), AgentError> {
    if files {
        let name = "files.tar.zst.enc";
        let (size, sha) = files_part(
            context,
            dest,
            &format!("pre-{rid}/{name}"),
            &format!("pre-{rid}.bk"),
            tenant_id,
            uid,
            key_text,
            None,
        )
        .await?;
        snap.push(BackupPartInfo {
            name: name.into(),
            kind: "files".into(),
            stored_bytes: size,
            sha256: sha,
            database: None,
        });
    }
    for database in databases {
        let name = format!("db-{database}.sql.zst.enc");
        let (size, sha) =
            database_part(context, dest, format!("pre-{rid}/{name}"), database, key).await?;
        snap.push(BackupPartInfo {
            name,
            kind: "database".into(),
            stored_bytes: size,
            sha256: sha,
            database: Some(database.clone()),
        });
    }
    Ok(())
}

async fn remove_snapshot(dest: &Shared, rid: &str, snap: &[BackupPartInfo]) {
    let dest = dest.clone();
    let names: Vec<String> = snap
        .iter()
        .map(|p| format!("pre-{rid}/{}", p.name))
        .chain([format!("pre-{rid}/files.tar.zst.enc")])
        .collect();
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(d) = dest.lock() {
            for n in names {
                let _ = d.delete(&n);
            }
        }
    })
    .await;
}

/// The stored files part, fetched to a staged file and put back by the helper.
#[allow(clippy::too_many_arguments)]
async fn restore_files_part(
    context: &TenantContext,
    dest: &Shared,
    object: &str,
    rid: &str,
    tenant_id: Uuid,
    uid: u32,
    key_text: &DbPassword,
    folder: Option<&str>,
) -> Result<(), AgentError> {
    let dir = transfers_dir(context);
    std::fs::create_dir_all(&dir)
        .map_err(|e| failed(&format!("cannot make the folder: {}", e.kind())).retryable())?;
    let staged_name = format!("{rid}.rs");
    let staged = dir.join(&staged_name);
    let _ = std::fs::remove_file(&staged);
    let fetched = {
        let (dest, object, staged) = (dest.clone(), object.to_owned(), staged.clone());
        tokio::task::spawn_blocking(move || -> Result<(), AgentError> {
            let guard = dest
                .lock()
                .map_err(|_| failed("the destination is not usable"))?;
            let mut reader = guard
                .get(&object)
                .map_err(|e| failed(&format!("cannot read the files: {e}")).retryable())?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&staged)
                .map_err(|e| failed(&format!("cannot stage the files: {}", e.kind())))?;
            std::io::copy(&mut reader, &mut file)
                .map_err(|e| failed(&format!("cannot stage the files: {}", e.kind())))?;
            Ok(())
        })
        .await
        .map_err(|_| failed("the fetching was cancelled"))?
    };
    let result = match fetched {
        Ok(()) => {
            let socket = context.helper_socket.clone();
            let action = Action::RestoreFiles {
                tenant_id: tenant_id.to_string(),
                uid,
                staged: staged_name,
                key: key_text.0.trim().to_owned(),
                folder: folder.map(str::to_owned),
            };
            tokio::task::spawn_blocking(move || ask_helper(&socket, action))
                .await
                .map_err(|_| failed("the restoring was cancelled"))?
                .map(|_| ())
        }
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_file(&staged);
    result
}

/// `delete_backup`: every object of the backup is removed from the destination.
///
/// # Errors
/// [`AgentError`]: bad arguments (not retryable); the destination failing (retryable).
pub async fn delete_backup(op: Operation) -> Result<Output, AgentError> {
    let Operation::DeleteBackup {
        backup_id,
        target,
        credentials,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let id = backup_id.simple().to_string();
    tokio::task::spawn_blocking(move || -> Result<Output, AgentError> {
        let dest = open_destination(&target, credentials.as_ref())?;
        let names = dest
            .list(&format!("{id}/"))
            .map_err(|e| failed(&e.to_string()).retryable())?;
        let mut removed = 0u32;
        for name in names {
            // Only what lies below this backup's own folder.
            if !name.starts_with(&format!("{id}/")) {
                continue;
            }
            dest.delete(&name)
                .map_err(|e| failed(&e.to_string()).retryable())?;
            removed += 1;
        }
        Ok(Output::BackupDeleted { removed })
    })
    .await
    .map_err(|_| failed("the removal was cancelled"))?
}

/// `export_backup`: the backup's objects in one tar file at the destination.
///
/// # Errors
/// [`AgentError`]: bad arguments or a backup that is not whole (not retryable); the
/// destination or the disk failing (retryable). The staged file is removed either way, and a
/// half-written export is removed from the destination.
pub async fn export_backup(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::ExportBackup {
        backup_id,
        export_id,
        target,
        credentials,
        header,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    if header.len() > 16 * 1024
        || builtpanel_backup::export::Header::from_reader(header.as_bytes()).is_err()
    {
        return Err(invalid("the export header is not usable"));
    }
    let dir = transfers_dir(&context);
    std::fs::create_dir_all(&dir)
        .map_err(|e| failed(&format!("cannot make the folder: {}", e.kind())).retryable())?;
    let id = backup_id.simple().to_string();
    let object = format!("exports/{}.tar", export_id.simple());
    let staged = dir.join(format!("{}.ex", export_id.simple()));
    let (staged2, object2) = (staged.clone(), object.clone());
    let result = tokio::task::spawn_blocking(move || -> Result<u64, AgentError> {
        let dest = open_destination(&target, credentials.as_ref())?;
        let dest_error = |e: builtpanel_backup_dest::DestError| failed(&e.to_string()).retryable();
        let names = dest.list(&format!("{id}/")).map_err(dest_error)?;
        let mut parts: Vec<String> = Vec::new();
        for name in names {
            let Some(plain) = name.strip_prefix(&format!("{id}/")) else {
                continue;
            };
            if plain == "manifest.json.enc" || builtpanel_backup::manifest::valid_part_name(plain) {
                parts.push(plain.to_owned());
            }
        }
        if !parts.iter().any(|p| p == "manifest.json.enc") {
            return Err(invalid("the backup has no manifest"));
        }
        parts.sort();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&staged2)
            .map_err(|e| failed(&format!("cannot stage the export: {}", e.kind())).retryable())?;
        let io = |e: std::io::Error| {
            failed(&format!("cannot pack the export: {}", e.kind())).retryable()
        };
        let mut tar = tar::Builder::new(file);
        let add = |tar: &mut tar::Builder<std::fs::File>,
                   name: &str,
                   size: u64,
                   data: &mut dyn Read|
         -> Result<(), AgentError> {
            let mut h = tar::Header::new_gnu();
            h.set_size(size);
            h.set_mode(0o600);
            h.set_mtime(0);
            h.set_entry_type(tar::EntryType::Regular);
            tar.append_data(&mut h, name, data.take(size)).map_err(io)
        };
        add(
            &mut tar,
            builtpanel_backup::export::HEADER,
            header.len() as u64,
            &mut header.as_bytes(),
        )?;
        for plain in &parts {
            let object_name = format!("{id}/{plain}");
            // The size first (the tar header needs it), then the bytes.
            let (size, _) =
                builtpanel_backup::manifest::hash_of(dest.get(&object_name).map_err(dest_error)?)
                    .map_err(io)?;
            let mut data = dest.get(&object_name).map_err(dest_error)?;
            add(&mut tar, plain, size, &mut data)?;
        }
        let file = tar.into_inner().map_err(io)?;
        let size = file.metadata().map_err(io)?.len();
        drop(file);
        let mut reader = std::fs::File::open(&staged2).map_err(io)?;
        dest.put(&object2, &mut reader).map_err(dest_error)?;
        Ok(size)
    })
    .await
    .map_err(|_| failed("the export was cancelled"))?;
    let _ = std::fs::remove_file(&staged);
    result.map(|size_bytes| Output::BackupExported { object, size_bytes })
}

/// `import_export`: an export file unpacked into a backup folder at the destination.
///
/// # Errors
/// [`AgentError`]: bad arguments, a file that is not an export, or a backup that is there
/// already (not retryable); the destination failing (retryable). What was put in place is
/// removed again on a failure.
pub async fn import_export(op: Operation) -> Result<Output, AgentError> {
    let Operation::ImportExport {
        object,
        backup_id,
        target,
        credentials,
    } = op
    else {
        return Err(super::wrong_operation());
    };
    let plain_object = object
        .strip_prefix("exports/")
        .and_then(|n| n.strip_suffix(".tar"))
        .is_some_and(|n| {
            !n.is_empty() && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    if !plain_object {
        return Err(invalid("the export's name is not usable"));
    }
    let id = backup_id.simple().to_string();
    tokio::task::spawn_blocking(move || -> Result<Output, AgentError> {
        let dest = open_destination(&target, credentials.as_ref())?;
        let dest_error = |e: builtpanel_backup_dest::DestError| failed(&e.to_string()).retryable();
        if !dest.list(&format!("{id}/")).map_err(dest_error)?.is_empty() {
            return Err(invalid("a backup with this id is there already"));
        }
        let input = dest.get(&object).map_err(dest_error)?;
        let mut archive = tar::Archive::new(input);
        let mut parts: Vec<BackupPartInfo> = Vec::new();
        let mut total = 0u64;
        let mut put_all = || -> Result<(), AgentError> {
            let mut seen_header = false;
            for entry in archive
                .entries()
                .map_err(|_| invalid("this is not an export file"))?
            {
                let mut entry = entry.map_err(|_| invalid("this is not an export file"))?;
                let name = entry
                    .path()
                    .map_err(|_| invalid("this is not an export file"))?
                    .to_string_lossy()
                    .into_owned();
                if name == builtpanel_backup::export::HEADER {
                    seen_header = true;
                    continue;
                }
                if !seen_header
                    || !(name == "manifest.json.enc"
                        || builtpanel_backup::manifest::valid_part_name(&name))
                {
                    return Err(invalid("this is not an export file"));
                }
                let mut counted = HashRead {
                    inner: &mut entry,
                    hasher: Sha256::new(),
                    size: 0,
                };
                dest.put(&format!("{id}/{name}"), &mut counted)
                    .map_err(dest_error)?;
                let (size, sha) = (counted.size, hex(&counted.hasher.finalize()));
                total += size;
                let kind = match name.as_str() {
                    "files.tar.zst.enc" => "files",
                    "dns.json.enc" => "dns",
                    "meta.json.enc" => "meta",
                    "manifest.json.enc" => continue,
                    _ => "database",
                };
                parts.push(BackupPartInfo {
                    database: name
                        .strip_prefix("db-")
                        .and_then(|r| r.strip_suffix(".sql.zst.enc"))
                        .map(str::to_owned),
                    name,
                    kind: kind.to_owned(),
                    stored_bytes: size,
                    sha256: sha,
                });
            }
            if !seen_header || parts.is_empty() {
                return Err(invalid("this is not an export file"));
            }
            Ok(())
        };
        match put_all() {
            Ok(()) => Ok(Output::ExportImported {
                parts,
                size_bytes: total,
            }),
            Err(e) => {
                for n in dest.list(&format!("{id}/")).unwrap_or_default() {
                    let _ = dest.delete(&n);
                }
                Err(e)
            }
        }
    })
    .await
    .map_err(|_| failed("the import was cancelled"))?
}
