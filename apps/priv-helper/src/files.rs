//! The file manager's work on a tenant's files (T08.01-T08.06).
//!
//! The helper itself never touches a tenant's files. For each request it starts a
//! copy of its own program (`builtpanel-priv-helper fileop --root DIR`, through
//! `setpriv`) **as the tenant's host user id and group**, with no supplementary groups
//! and none of the helper's capabilities, so the worker can do exactly what the tenant could do with
//! its own files and nothing else: another tenant's files, the agent's state and the
//! system are as closed to it as they are to the tenant. The worker reads one request
//! from its input, opens the tenant's directory with `builtpanel-safe-fs`, does it with
//! `builtpanel-fileops` and prints one answer. It is given a time limit and killed after.

use std::{
    io::{BufRead as _, Read, Write as _},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use builtpanel_node_protocol::files::{FileErrorCode, FileRequest, FileResult};

use crate::{
    protocol::ErrorCode,
    tenant::{Failure, TenantEnv, host_uid, valid_tenant_id},
};

/// Longest a worker may run (a big archive takes a while).
pub const TIME_LIMIT: Duration = Duration::from_secs(600);
/// Most a worker's answer may be.
const MAX_ANSWER: u64 = 2 * 1024 * 1024;

/// Runs a request as the tenant and returns the answer.
///
/// # Errors
/// The id or uid is not acceptable, the worker could not be started, ran too long or
/// said nothing usable.
pub fn run(
    env: &TenantEnv,
    tenant_id: &str,
    uid: u32,
    request: &FileRequest,
) -> Result<FileResult, Failure> {
    if !valid_tenant_id(tenant_id) {
        return Err((ErrorCode::InvalidArguments, "bad tenant id"));
    }
    let range = env.id_range()?;
    let host = host_uid(range, uid)?;
    let root = env.root.join(tenant_id);
    work(&root, host, host, request)
}

/// What to put before the worker: a walk over everything runs at the lowest disk and
/// processor priority, so it does not slow down the sites.
fn low_priority(request: &FileRequest) -> Vec<&'static str> {
    if !matches!(request, FileRequest::DiskUsage { .. }) {
        return Vec::new();
    }
    let mut prefix = Vec::new();
    for (program, args) in [
        ("/usr/bin/ionice", ["-c3"].as_slice()),
        ("/usr/bin/nice", ["-n19"].as_slice()),
    ] {
        if Path::new(program).exists() {
            prefix.push(program);
            prefix.extend_from_slice(args);
        }
    }
    prefix
}

/// Starts the worker as `uid`/`gid` (the caller has already checked both).
///
/// # Errors
/// See [`run`].
pub fn work(root: &Path, uid: u32, gid: u32, request: &FileRequest) -> Result<FileResult, Failure> {
    let program = std::env::current_exe()
        .map_err(|_| (ErrorCode::Failed, "cannot find the helper itself"))?;
    let input = serde_json::to_vec(request)
        .map_err(|_| (ErrorCode::InvalidArguments, "the request cannot be encoded"))?;
    // `setpriv` (util-linux, on every Linux) drops to the tenant's user and group,
    // clears the supplementary groups and forbids gaining anything later, then runs
    // the worker.
    let setpriv = ["/usr/bin/setpriv", "/bin/setpriv"]
        .into_iter()
        .find(|p| Path::new(p).exists())
        .ok_or((ErrorCode::Failed, "setpriv (util-linux) is missing"))?;
    let mut child = Command::new(setpriv)
        .arg(format!("--reuid={uid}"))
        .arg(format!("--regid={gid}"))
        .arg("--clear-groups")
        .arg("--no-new-privs")
        .arg("--")
        .args(low_priority(request))
        .arg(program)
        .arg("fileop")
        .arg("--root")
        .arg(root)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| (ErrorCode::Failed, "cannot start the file worker"))?;
    let (mut stdin, stdout) = child
        .stdin
        .take()
        .zip(child.stdout.take())
        .ok_or((ErrorCode::Failed, "cannot talk to the file worker"))?;
    let stderr = child.stderr.take();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.take(MAX_ANSWER).read_to_end(&mut out);
        out
    });
    let started = Instant::now();
    let finished = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status.success()),
            Ok(None) if started.elapsed() > TIME_LIMIT => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break Some(false),
        }
    };
    let _ = writer.join();
    let answer = reader.join().unwrap_or_default();
    // What the worker said about itself goes to the log (never to the caller).
    let said = stderr.map_or_else(String::new, |mut e| {
        let mut text = String::new();
        let _ = e.by_ref().take(300).read_to_string(&mut text);
        text
    });
    if finished != Some(true) && !said.is_empty() {
        eprintln!(
            "builtpanel-priv-helper: file worker said: {}",
            said.escape_debug()
        );
    }
    match finished {
        None => Ok(FileResult::error(
            FileErrorCode::Failed,
            "the request took too long and was stopped",
        )),
        Some(true) => serde_json::from_slice(&answer)
            .map_err(|_| (ErrorCode::Failed, "the file worker's answer is unreadable")),
        Some(false) => Err((ErrorCode::Failed, "the file worker failed")),
    }
}

/// The worker's side: one request in, one answer out.
///
/// # Errors
/// The root cannot be opened or the request cannot be read.
pub fn worker(
    root: &Path,
    input: impl std::io::Read,
    mut out: impl std::io::Write,
) -> Result<(), String> {
    let mut bytes = Vec::new();
    input
        .take(crate::protocol::MAX_REQUEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let request: FileRequest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let result = match builtpanel_safe_fs::Root::open(root) {
        Ok(root) => builtpanel_fileops::handle(&root, request),
        Err(_) => FileResult::error(FileErrorCode::NotFound, "the files are not there"),
    };
    let json = serde_json::to_vec(&result).map_err(|e| e.to_string())?;
    out.write_all(&json).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- transfers

use std::{fs::File, os::fd::OwnedFd};

use builtpanel_safe_fs::Root;

/// Opens the transfer folder (the agent's, handed to us by the unit).
fn staged_root(dir: &Path) -> Result<Root, Failure> {
    Root::open(dir).map_err(|_| (ErrorCode::Failed, "the transfer folder is not usable"))
}

fn host_ids(
    env: &TenantEnv,
    tenant_id: &str,
    uid: u32,
) -> Result<(u32, std::path::PathBuf), Failure> {
    if !valid_tenant_id(tenant_id) {
        return Err((ErrorCode::InvalidArguments, "bad tenant id"));
    }
    let range = env.id_range()?;
    Ok((host_uid(range, uid)?, env.root.join(tenant_id)))
}

/// Runs `setpriv ... helper <mode> --root DIR <args>` as `uid`, with the given input
/// and output, and waits for it. Returns whether it succeeded and what it printed
/// on stdout (only if it was a pipe) and stderr.
fn run_worker(
    uid: u32,
    mode: &str,
    root: &Path,
    args: &[String],
    stdin: Stdio,
    stdout: Stdio,
) -> Result<(bool, Vec<u8>, String), Failure> {
    run_worker_with(uid, mode, root, args, stdin, stdout, None)
}

/// [`run_worker`], giving the worker `input` on its standard input (when `Some`; the
/// `stdin` given is then ignored).
fn run_worker_with(
    uid: u32,
    mode: &str,
    root: &Path,
    args: &[String],
    stdin: Stdio,
    stdout: Stdio,
    input: Option<Box<dyn Read + Send>>,
) -> Result<(bool, Vec<u8>, String), Failure> {
    let program = std::env::current_exe()
        .map_err(|_| (ErrorCode::Failed, "cannot find the helper itself"))?;
    let setpriv = ["/usr/bin/setpriv", "/bin/setpriv"]
        .into_iter()
        .find(|p| Path::new(p).exists())
        .ok_or((ErrorCode::Failed, "setpriv (util-linux) is missing"))?;
    let mut child = Command::new(setpriv)
        .arg(format!("--reuid={uid}"))
        .arg(format!("--regid={uid}"))
        .arg("--clear-groups")
        .arg("--no-new-privs")
        .arg("--")
        .arg(program)
        .arg(mode)
        .arg("--root")
        .arg(root)
        .args(args)
        .env_clear()
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            stdin
        })
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| (ErrorCode::Failed, "cannot start the file worker"))?;
    let feeder = input.zip(child.stdin.take()).map(|(mut data, mut pipe)| {
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut data, &mut pipe);
        })
    });
    let out = child.stdout.take();
    let err = child.stderr.take();
    let out_reader = std::thread::spawn(move || {
        let mut text = Vec::new();
        if let Some(o) = out {
            let _ = o.take(MAX_ANSWER).read_to_end(&mut text);
        }
        text
    });
    let err_reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(e) = err {
            let _ = e.take(4096).read_to_string(&mut text);
        }
        text
    });
    let started = Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if started.elapsed() > TIME_LIMIT => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break false,
        }
    };
    if let Some(feeder) = feeder {
        let _ = feeder.join();
    }
    Ok((
        ok,
        out_reader.join().unwrap_or_default(),
        err_reader.join().unwrap_or_default(),
    ))
}

/// What the worker said (as JSON on its stdout or stderr) as a result.
fn said(ok: bool, out: &[u8], err: &str) -> Result<(), Failure> {
    let parsed = serde_json::from_slice::<FileResult>(out)
        .ok()
        .or_else(|| serde_json::from_str::<FileResult>(err.trim()).ok());
    match (ok, parsed) {
        (true, Some(FileResult::Done { .. })) | (true, None) => Ok(()),
        (_, Some(FileResult::Error { code, .. })) => Err((
            match code {
                FileErrorCode::TooLarge => ErrorCode::InvalidArguments,
                FileErrorCode::Exists | FileErrorCode::Failed => ErrorCode::Failed,
                _ => ErrorCode::InvalidArguments,
            },
            match code {
                FileErrorCode::Exists => "a file is already there",
                FileErrorCode::TooLarge => "the file is bigger than the limit",
                FileErrorCode::NotFound => "not found",
                FileErrorCode::Denied => "not allowed",
                FileErrorCode::IsDir => "it is a directory",
                FileErrorCode::InvalidPath | FileErrorCode::Link | FileErrorCode::NotDir => {
                    "the path is not usable"
                }
                _ => "the file worker refused",
            },
        )),
        _ => Err((ErrorCode::Failed, "the file worker failed")),
    }
}

/// `place_file`.
///
/// # Errors
/// Bad arguments, the worker's refusal (for example `exists`) or a failure.
#[allow(clippy::too_many_arguments)]
pub fn place_file(
    env: &TenantEnv,
    transfers: &Path,
    tenant_id: &str,
    uid: u32,
    staged: &str,
    dest: &str,
    overwrite: bool,
    max_bytes: u64,
) -> Result<(), Failure> {
    let (host, root) = host_ids(env, tenant_id, uid)?;
    builtpanel_safe_fs::check_name(staged)
        .map_err(|_| (ErrorCode::InvalidArguments, "bad staged name"))?;
    let fd = staged_root(transfers)?
        .open_at(
            staged,
            rustix::fs::OFlags::RDONLY,
            rustix::fs::Mode::empty(),
        )
        .map_err(|_| (ErrorCode::Failed, "the staged file is not usable"))?;
    let mut args = vec![
        "--dest".to_owned(),
        dest.to_owned(),
        "--max-bytes".to_owned(),
        max_bytes.to_string(),
    ];
    if overwrite {
        args.push("--overwrite".to_owned());
    }
    let (ok, out, err) = run_worker(
        host,
        "fileput",
        &root,
        &args,
        Stdio::from(File::from(fd)),
        Stdio::piped(),
    )?;
    said(ok, &out, &err)
}

/// `take_file`.
///
/// # Errors
/// As [`place_file`].
#[allow(clippy::too_many_arguments)]
pub fn take_file(
    env: &TenantEnv,
    transfers: &Path,
    agent: (u32, u32),
    tenant_id: &str,
    uid: u32,
    path: &str,
    staged: &str,
    max_bytes: u64,
) -> Result<(), Failure> {
    let (host, root) = host_ids(env, tenant_id, uid)?;
    builtpanel_safe_fs::check_name(staged)
        .map_err(|_| (ErrorCode::InvalidArguments, "bad staged name"))?;
    let folder = staged_root(transfers)?;
    let out: OwnedFd = rustix::fs::openat(
        folder.fd(),
        staged,
        rustix::fs::OFlags::WRONLY
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::EXCL
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::from_raw_mode(0o600),
    )
    .map_err(|_| (ErrorCode::Failed, "cannot make the staged file"))?;
    // The agent reads it afterwards; the worker only writes to the descriptor it is given.
    rustix::fs::fchown(
        &out,
        Some(rustix::fs::Uid::from_raw(agent.0)),
        Some(rustix::fs::Gid::from_raw(agent.1)),
    )
    .map_err(|_| (ErrorCode::Failed, "cannot hand the staged file over"))?;
    let args = vec![
        "--path".to_owned(),
        path.to_owned(),
        "--max-bytes".to_owned(),
        max_bytes.to_string(),
    ];
    let result = run_worker(
        host,
        "fileget",
        &root,
        &args,
        Stdio::null(),
        Stdio::from(File::from(out)),
    )
    .and_then(|(ok, out, err)| said(ok, &out, &err));
    if result.is_err() {
        let _ = builtpanel_safe_fs::remove(folder.fd(), staged, false);
    }
    result
}

/// `backup_files`: the tenant's files, packed and encrypted, into a new staged file.
///
/// # Errors
/// As [`place_file`].
#[allow(clippy::too_many_arguments)]
pub fn backup_files(
    env: &TenantEnv,
    transfers: &Path,
    agent: (u32, u32),
    tenant_id: &str,
    uid: u32,
    staged: &str,
    key: &str,
    since: Option<i64>,
) -> Result<(), Failure> {
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err((ErrorCode::InvalidArguments, "the key is 64 hex digits"));
    }
    let (host, root) = host_ids(env, tenant_id, uid)?;
    builtpanel_safe_fs::check_name(staged)
        .map_err(|_| (ErrorCode::InvalidArguments, "bad staged name"))?;
    let folder = staged_root(transfers)?;
    let out: OwnedFd = rustix::fs::openat(
        folder.fd(),
        staged,
        rustix::fs::OFlags::WRONLY
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::EXCL
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::from_raw_mode(0o600),
    )
    .map_err(|_| (ErrorCode::Failed, "cannot make the staged file"))?;
    rustix::fs::fchown(
        &out,
        Some(rustix::fs::Uid::from_raw(agent.0)),
        Some(rustix::fs::Gid::from_raw(agent.1)),
    )
    .map_err(|_| (ErrorCode::Failed, "cannot hand the staged file over"))?;
    // Packing everything is slow work: lowest priority, like the disk usage walk.
    let result = run_worker_with(
        host,
        "filebackup",
        &root,
        &[],
        Stdio::null(),
        Stdio::from(File::from(out)),
        Some(Box::new(std::io::Cursor::new(
            since
                .map_or_else(|| format!("{key}\n"), |t| format!("{key}\n{t}\n"))
                .into_bytes(),
        ))),
    )
    .and_then(|(ok, out, err)| said(ok, &out, &err));
    if result.is_err() {
        let _ = builtpanel_safe_fs::remove(folder.fd(), staged, false);
    }
    result
}

/// The worker of `filebackup`: the key arrives on stdin, the encrypted stream goes to stdout.
///
/// # Errors
/// The refusal, as the answer to print on stderr.
pub fn backup_worker(
    root: &Path,
    mut input: impl std::io::Read,
    out: &mut dyn std::io::Write,
) -> Result<(), FileResult> {
    let mut text = String::new();
    input
        .by_ref()
        .take(256)
        .read_to_string(&mut text)
        .map_err(|_| FileResult::error(FileErrorCode::InvalidArguments, "no key"))?;
    // The key, and on a second line the time to pack only what is newer than.
    let mut lines = text.lines();
    let hex = lines.next().unwrap_or_default().trim();
    let since: Option<i64> = lines.next().and_then(|l| l.trim().parse().ok());
    let mut key = [0u8; builtpanel_backup::KEY_LEN];
    if hex.len() != key.len() * 2 {
        return Err(FileResult::error(
            FileErrorCode::InvalidArguments,
            "the key is not 64 hex digits",
        ));
    }
    for (i, b) in key.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| {
            FileResult::error(FileErrorCode::InvalidArguments, "the key is not hex")
        })?;
    }
    let root = Root::open(root)
        .map_err(|_| FileResult::error(FileErrorCode::NotFound, "the files are not there"))?;
    builtpanel_fileops::backup::pack_files(&root, &key, since, out)
        .map(|_| ())
        .map_err(|(code, message)| FileResult::error(code, message))
}

/// The worker of `fileput`: the staged file arrives on stdin.
///
/// # Errors
/// Never; the answer says what happened.
pub fn put_worker(
    root: &Path,
    dest: &str,
    overwrite: bool,
    max_bytes: u64,
    mut input: impl std::io::Read,
    mut out: impl std::io::Write,
) {
    let result = match Root::open(root) {
        Ok(root) => {
            match builtpanel_fileops::place(&root, dest, overwrite, max_bytes, &mut input) {
                Ok(_) => FileResult::Done { count: 1 },
                Err((code, message)) => FileResult::error(code, message),
            }
        }
        Err(_) => FileResult::error(FileErrorCode::NotFound, "the files are not there"),
    };
    let _ = out.write_all(&serde_json::to_vec(&result).unwrap_or_default());
}

/// The worker of `fileget`: the file goes to stdout, a refusal is said on stderr.
///
/// # Errors
/// The refusal, as the answer to print on stderr.
pub fn get_worker(
    root: &Path,
    path: &str,
    max_bytes: u64,
    out: &mut dyn std::io::Write,
) -> Result<(), FileResult> {
    let root = Root::open(root)
        .map_err(|_| FileResult::error(FileErrorCode::NotFound, "the files are not there"))?;
    builtpanel_fileops::take(&root, path, max_bytes, out)
        .map(|_| ())
        .map_err(|(code, message)| FileResult::error(code, message))
}

/// `restore_files`: puts the files of an encrypted stream (a staged file of the agent's
/// transfer folder) back into the tenant's folder, as the tenant's own user. The key goes to
/// the worker first on its input, then the staged file, never through a command line.
///
/// # Errors
/// As [`place_file`].
pub fn restore_files(
    env: &TenantEnv,
    transfers: &Path,
    tenant_id: &str,
    uid: u32,
    staged: &str,
    key: &str,
    only: Option<&str>,
) -> Result<(), Failure> {
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err((ErrorCode::InvalidArguments, "the key is 64 hex digits"));
    }
    let (host, root) = host_ids(env, tenant_id, uid)?;
    builtpanel_safe_fs::check_name(staged)
        .map_err(|_| (ErrorCode::InvalidArguments, "bad staged name"))?;
    if let Some(folder) = only
        && (folder.len() > 1024
            || folder.contains('\0')
            || builtpanel_safe_fs::parts(folder).is_err())
    {
        return Err((ErrorCode::InvalidArguments, "bad folder"));
    }
    let fd = staged_root(transfers)?
        .open_at(
            staged,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|_| (ErrorCode::Failed, "the staged file is not usable"))?;
    let mut args = Vec::new();
    if let Some(folder) = only {
        args.push("--folder".to_owned());
        args.push(folder.to_owned());
    }
    let input = std::io::Cursor::new(format!("{key}\n").into_bytes()).chain(File::from(fd));
    let (ok, out, err) = run_worker_with(
        host,
        "filerestore",
        &root,
        &args,
        Stdio::null(),
        Stdio::piped(),
        Some(Box::new(input)),
    )?;
    said(ok, &out, &err)
}

/// The worker of `filerestore`: the key (one line), then the encrypted stream, arrive on
/// stdin.
///
/// # Errors
/// The refusal, as the answer to print on stderr.
pub fn restore_worker(
    root: &Path,
    mut input: impl std::io::BufRead,
    only: Option<&str>,
) -> Result<(), FileResult> {
    let mut text = String::new();
    input
        .by_ref()
        .take(256)
        .read_line(&mut text)
        .map_err(|_| FileResult::error(FileErrorCode::InvalidArguments, "no key"))?;
    let hex = text.trim();
    let mut key = [0u8; builtpanel_backup::KEY_LEN];
    if hex.len() != key.len() * 2 {
        return Err(FileResult::error(
            FileErrorCode::InvalidArguments,
            "the key is not 64 hex digits",
        ));
    }
    for (i, b) in key.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| {
            FileResult::error(FileErrorCode::InvalidArguments, "the key is not hex")
        })?;
    }
    let root = Root::open(root)
        .map_err(|_| FileResult::error(FileErrorCode::NotFound, "the files are not there"))?;
    builtpanel_fileops::backup::unpack_files(&root, &key, input, only)
        .map(|_| ())
        .map_err(|(code, message)| FileResult::error(code, message))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_worker_answers_a_request_on_a_directory() {
        let dir = std::env::temp_dir().join(format!("dhph-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let request = serde_json::to_vec(&FileRequest::List {
            path: String::new(),
            offset: 0,
            limit: 10,
        })
        .unwrap();
        let mut out = Vec::new();
        worker(&dir, &request[..], &mut out).unwrap();
        let FileResult::Entries { entries, total } = serde_json::from_slice(&out).unwrap() else {
            panic!("{}", String::from_utf8_lossy(&out));
        };
        assert_eq!((total, entries[0].name.as_str()), (1, "a.txt"));
        // A directory that is not there, and a request that is not one.
        let mut out = Vec::new();
        worker(&dir.join("nope"), &request[..], &mut out).unwrap();
        assert!(matches!(
            serde_json::from_slice::<FileResult>(&out).unwrap(),
            FileResult::Error {
                code: FileErrorCode::NotFound,
                ..
            }
        ));
        assert!(worker(&dir, &b"not json"[..], Vec::new()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_ids_never_start_a_worker() {
        let env = TenantEnv {
            root: "/nonexistent".into(),
            agent_name: "builtpanel-agent".into(),
            subuid_file: "/nonexistent".into(),
            agent_gid: 1,
            mountinfo: "/nonexistent".into(),
            identity: false,
        };
        let request = FileRequest::TrashList;
        assert_eq!(
            run(&env, "../../etc", 10_000, &request).unwrap_err().0,
            ErrorCode::InvalidArguments
        );
    }
}
