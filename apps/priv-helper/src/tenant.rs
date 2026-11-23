//! Tenant directories (T04.07): the one thing about tenants that needs root.
//!
//! Containers run in the agent's rootless user namespace, where tenant id `N`
//! (10000 and up) is the host id `subuid_start + N - 1` of the agent user's
//! range in `/etc/subuid`. A tenant's files therefore have to be owned by that
//! host id. Only root can give files to such an id, so the agent asks the helper.
//!
//! What this module does, and nothing else: create `<root>/<tenant id>` and
//! `<root>/<tenant id>/www` (mode 0750, owner the tenant's host id, group the
//! agent). The path comes only from a validated UUID, so no request can point it
//! anywhere else; a symlink or anything that is not a directory in the way is a
//! failure, never followed. Doing it twice is fine; a directory that exists with
//! another owner is a failure (it would mean two tenants share an id).

use std::{
    fs, io,
    os::unix::fs::{MetadataExt as _, PermissionsExt as _},
    path::PathBuf,
};

use crate::{
    protocol::{ErrorCode, SkeletonFile},
    quota, skeleton,
};

/// First and one-past-last tenant id (kept in step with `builtpanel_domain::UidRange::DEFAULT`).
pub const UID_START: u32 = 10_000;
/// See [`UID_START`].
pub const UID_END: u32 = 60_000;

/// Where tenants live and how ids map to the host.
#[derive(Debug, Clone)]
pub struct TenantEnv {
    /// The directory holding every tenant's directory.
    pub root: PathBuf,
    /// The agent user's name (to find its range in `subuid_file`).
    pub agent_name: String,
    /// `/etc/subuid`.
    pub subuid_file: PathBuf,
    /// The group tenant directories get (the agent's).
    pub agent_gid: u32,
    /// `/proc/self/mountinfo`: where quotas are looked for.
    pub mountinfo: PathBuf,
    /// The containers run without a user namespace of their own (a root container engine, Kiri): tenant
    /// id `N` is host id `N`, and `subuid_file` is not looked at.
    pub identity: bool,
}

impl TenantEnv {
    /// The ids the agent's containers use on the host, as `(start, count)`: the agent's subordinate range
    /// for a rootless engine, every id as it is (`N` is `N`) for a root one.
    ///
    /// # Errors
    /// The subordinate id file cannot be read, or the agent has no range in it.
    pub fn id_range(&self) -> Result<(u32, u32), Failure> {
        if self.identity {
            return Ok((1, UID_END));
        }
        let text = fs::read_to_string(&self.subuid_file)
            .map_err(|_| (ErrorCode::Failed, "cannot read the subordinate id file"))?;
        subuid_range(&text, &self.agent_name)
            .ok_or((ErrorCode::Failed, "the agent has no subordinate id range"))
    }
}

/// What is wrong, in a form that goes back to the agent and into the log.
pub type Failure = (ErrorCode, &'static str);

/// Whether `id` is a lower-case UUID, the only shape allowed in a path.
#[must_use]
pub fn valid_tenant_id(id: &str) -> bool {
    id.len() == 36
        && id.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_digit() || ('a'..='f').contains(&c),
        })
}

/// The agent's range in a `subuid` file: `(start, count)`.
#[must_use]
pub fn subuid_range(text: &str, agent: &str) -> Option<(u32, u32)> {
    text.lines().find_map(|line| {
        let mut fields = line.split(':');
        (fields.next()? == agent).then_some(())?;
        Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
    })
}

/// The host id of tenant id `uid`.
///
/// # Errors
/// The id is outside the tenant range, or the agent's subordinate range does
/// not reach that far.
pub fn host_uid(range: (u32, u32), uid: u32) -> Result<u32, Failure> {
    if !(UID_START..UID_END).contains(&uid) {
        return Err((
            ErrorCode::InvalidArguments,
            "the user id is outside the tenant range",
        ));
    }
    let (start, count) = range;
    let offset = uid - 1;
    if offset >= count {
        return Err((
            ErrorCode::Failed,
            "the agent's subordinate id range is too small",
        ));
    }
    start
        .checked_add(offset)
        .ok_or((ErrorCode::Failed, "the subordinate id range is invalid"))
}

/// Creates the tenant's directories.
///
/// # Errors
/// [`Failure`] with a fixed message.
pub fn prepare(env: &TenantEnv, tenant_id: &str, uid: u32) -> Result<(), Failure> {
    prepare_with_files(env, tenant_id, uid, &[])
}

/// [`prepare`], also putting default files into `www` when the directories are
/// created now. A tenant that was already finished keeps its files as they are.
///
/// # Errors
/// [`Failure`].
pub fn prepare_with_files(
    env: &TenantEnv,
    tenant_id: &str,
    uid: u32,
    skeleton: &[SkeletonFile],
) -> Result<(), Failure> {
    if !valid_tenant_id(tenant_id) {
        return Err((ErrorCode::InvalidArguments, "the tenant id is not a UUID"));
    }
    // Checked before any file is read: a bad request costs nothing.
    skeleton::check(skeleton)?;
    if !(UID_START..UID_END).contains(&uid) {
        return Err((
            ErrorCode::InvalidArguments,
            "the user id is outside the tenant range",
        ));
    }
    let range = env.id_range()?;
    let owner = host_uid(range, uid)?;
    // When the tenants live on a filesystem with project quotas, the directory
    // joins the quota project of its id before it is handed over.
    let project = fs::read_to_string(&env.mountinfo)
        .ok()
        .and_then(|text| quota::detect(&text, &env.root.join(tenant_id)))
        .map(|backend| (backend, uid));
    prepare_as_with(
        env,
        tenant_id,
        owner,
        project.as_ref().map(|(b, u)| (b, *u)),
        skeleton,
    )
}

/// Sets the limits of a tenant that was prepared on a quota-capable filesystem.
///
/// # Errors
/// [`Failure`]: bad arguments, [`ErrorCode::Unsupported`] on a filesystem
/// without project quotas, or a failure to apply them.
pub fn set_storage_quota(
    env: &TenantEnv,
    tenant_id: &str,
    uid: u32,
    disk_bytes: Option<u64>,
    inodes: Option<u64>,
) -> Result<(), Failure> {
    if !valid_tenant_id(tenant_id) {
        return Err((ErrorCode::InvalidArguments, "the tenant id is not a UUID"));
    }
    if !(UID_START..UID_END).contains(&uid) {
        return Err((
            ErrorCode::InvalidArguments,
            "the user id is outside the tenant range",
        ));
    }
    quota::check_limits(disk_bytes, inodes)?;
    let dir = env.root.join(tenant_id);
    let mountinfo = fs::read_to_string(&env.mountinfo)
        .map_err(|_| (ErrorCode::Failed, "cannot read the mount table"))?;
    let backend = quota::detect(&mountinfo, &dir).ok_or((
        ErrorCode::Unsupported,
        "this node's filesystem has no project quotas",
    ))?;
    // The directory must exist and belong to this tenant's id: a limit for a
    // tenant that was never prepared would otherwise be set on nothing.
    let range = env.id_range()?;
    let owner = host_uid(range, uid)?;
    match fs::symlink_metadata(&dir) {
        Ok(meta) if meta.file_type().is_dir() && meta.uid() == owner => {}
        _ => {
            return Err((
                ErrorCode::Failed,
                "the tenant has not been prepared on this node",
            ));
        }
    }
    quota::run(&quota::set_limits(&backend, uid, disk_bytes, inodes))
        .map_err(|_| (ErrorCode::Failed, "applying the quota failed"))
}

/// Removes the (empty) directories of a deleted tenant.
///
/// # Errors
/// [`Failure`]: bad arguments, a directory that is not the tenant's, or one
/// that still holds files (the agent empties it first).
pub fn remove(env: &TenantEnv, tenant_id: &str, uid: u32) -> Result<(), Failure> {
    if !valid_tenant_id(tenant_id) {
        return Err((ErrorCode::InvalidArguments, "the tenant id is not a UUID"));
    }
    let range = env.id_range()?;
    let owner = host_uid(range, uid)?;
    remove_as(env, tenant_id, owner)
}

/// The filesystem part of [`remove`], with the owner already worked out.
///
/// The helper holds only `CAP_CHOWN`, so it first takes the tenant directory
/// back (the tenant's containers are gone, nobody else can be inside), which
/// lets it look inside again; then both directories go. `remove_dir` only ever
/// removes an empty directory, so a tenant's files can never be deleted here.
///
/// # Errors
/// [`Failure`].
pub fn remove_as(env: &TenantEnv, tenant_id: &str, owner: u32) -> Result<(), Failure> {
    let failed = |_: io::Error| (ErrorCode::Failed, "cannot remove the tenant directory");
    let clash = (
        ErrorCode::Failed,
        "the tenant directory has a different owner or type",
    );
    let me = rustix::process::geteuid().as_raw();
    let my_gid = rustix::process::getegid().as_raw();
    let tenant = env.root.join(tenant_id);
    let www = tenant.join("www");
    match fs::symlink_metadata(&tenant) {
        // Already gone: a repeated request is fine.
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(failed(err)),
        Ok(meta) if meta.file_type().is_dir() && (meta.uid() == owner || meta.uid() == me) => {
            if meta.uid() != me {
                // lchown: never through a link.
                std::os::unix::fs::lchown(&tenant, Some(me), Some(my_gid)).map_err(failed)?;
            }
        }
        Ok(_) => return Err(clash),
    }
    // Now ours: nobody can change what is inside while we look at it.
    match fs::symlink_metadata(&www) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(failed(err)),
        Ok(meta) if meta.file_type().is_dir() => {
            if meta.uid() != me {
                std::os::unix::fs::lchown(&www, Some(me), Some(my_gid)).map_err(failed)?;
            }
            fs::remove_dir(&www).map_err(|_| {
                (
                    ErrorCode::Failed,
                    "the tenant's files must be deleted before its directories",
                )
            })?;
        }
        Ok(_) => return Err(clash),
    }
    fs::remove_dir(&tenant).map_err(|_| {
        (
            ErrorCode::Failed,
            "the tenant directory holds something besides the site files",
        )
    })
}

/// The filesystem part, with the owner already worked out.
///
/// The helper holds only `CAP_CHOWN`: once a directory belongs to the tenant,
/// the helper (not the owner, not in the group) can no longer look inside it.
/// So everything inside is made first, the inner directory is handed over
/// before the outer one, and "the tenant directory belongs to the tenant" means
/// "this was finished". A directory still owned by the helper's own user is an
/// earlier attempt that stopped half way, and is finished now.
///
/// # Errors
/// [`Failure`].
pub fn prepare_as(
    env: &TenantEnv,
    tenant_id: &str,
    owner: u32,
    project: Option<(&quota::Backend, u32)>,
) -> Result<(), Failure> {
    prepare_as_with(env, tenant_id, owner, project, &[])
}

/// [`prepare_as`] with default files.
///
/// # Errors
/// [`Failure`].
pub fn prepare_as_with(
    env: &TenantEnv,
    tenant_id: &str,
    owner: u32,
    project: Option<(&quota::Backend, u32)>,
    skeleton: &[SkeletonFile],
) -> Result<(), Failure> {
    let failed = |_: io::Error| (ErrorCode::Failed, "cannot create the tenant directory");
    let clash = (
        ErrorCode::Failed,
        "the tenant directory exists with a different owner or type",
    );
    // The parent exists and is the helper's own: created here if missing, never
    // inside a directory anybody else controls.
    match fs::symlink_metadata(&env.root) {
        Ok(meta) if meta.file_type().is_dir() => {}
        Ok(_) => return Err(clash),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(&env.root).map_err(failed)?;
            fs::set_permissions(&env.root, fs::Permissions::from_mode(0o755)).map_err(failed)?;
        }
        Err(err) => return Err(failed(err)),
    }
    let me = rustix::process::geteuid().as_raw();
    let tenant = env.root.join(tenant_id);
    let www = tenant.join("www");
    match fs::symlink_metadata(&tenant) {
        // Finished before: owned by the tenant (and so, by construction, complete).
        Ok(meta)
            if meta.file_type().is_dir() && meta.uid() == owner && meta.gid() == env.agent_gid =>
        {
            return Ok(());
        }
        // Started before and stopped: still ours. Finish it below.
        Ok(meta) if meta.file_type().is_dir() && meta.uid() == me => {}
        Ok(_) => return Err(clash),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&tenant).map_err(failed)?;
        }
        Err(err) => return Err(failed(err)),
    }
    // Inside first, while the helper can still reach it.
    match fs::symlink_metadata(&www) {
        Ok(meta) if meta.file_type().is_dir() => {}
        Ok(_) => return Err(clash),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&www).map_err(failed)?
        }
        Err(err) => return Err(failed(err)),
    }
    for dir in [&www, &tenant] {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o750)).map_err(failed)?;
    }
    // Default files, while everything here is still ours; they join the quota
    // project below and are handed over with the directory.
    skeleton::write(&www, skeleton, owner, env.agent_gid)
        .map_err(|_| (ErrorCode::Failed, "cannot write the tenant's default files"))?;
    // Quota project: while the tree is empty and ours, everything created in it
    // later joins the project.
    if let Some((backend, id)) = project {
        quota::ensure_state_dir().map_err(failed)?;
        quota::run(&quota::project_setup(backend, &tenant, id)).map_err(|_| {
            (
                ErrorCode::Failed,
                "cannot set up the tenant's quota project",
            )
        })?;
    }
    // Hand over: inner first, outer last (the outer one is the "finished" mark).
    for dir in [&www, &tenant] {
        std::os::unix::fs::chown(dir, Some(owner), Some(env.agent_gid)).map_err(failed)?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    const ID: &str = "0190b6a2-0000-7000-8000-000000000001";

    #[test]
    fn a_root_engine_uses_the_ids_as_they_are() {
        let env = TenantEnv {
            root: "/nonexistent".into(),
            agent_name: "builtpanel-agent".into(),
            // Not read at all in this mode.
            subuid_file: "/nonexistent".into(),
            agent_gid: 1,
            mountinfo: "/nonexistent".into(),
            identity: true,
        };
        let range = env.id_range().unwrap();
        for uid in [UID_START, 10_042, UID_END - 1] {
            assert_eq!(host_uid(range, uid).unwrap(), uid, "{uid}");
        }
        assert!(host_uid(range, UID_START - 1).is_err());
        assert!(host_uid(range, UID_END).is_err());
        // A rootless engine still needs its range.
        let rootless = TenantEnv {
            identity: false,
            ..env
        };
        assert!(rootless.id_range().is_err());
    }

    #[test]
    fn ids_are_uuids_and_nothing_else() {
        assert!(valid_tenant_id(ID));
        for bad in [
            "",
            "..",
            "../etc",
            "0190B6A2-0000-7000-8000-000000000001",
            "0190b6a2-0000-7000-8000-00000000000",
            "0190b6a2_0000_7000_8000_000000000001",
            "0190b6a2-0000-7000-8000-00000000000g",
            "0190b6a2-0000-7000-8000-000000000001/x",
            "0190b6a2-0000-7000-8000-000000000001\n",
        ] {
            assert!(!valid_tenant_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn tenant_ids_map_into_the_agents_subordinate_range() {
        let text = "dev:100000:65536\nbuiltpanel-agent:165536:65536\n";
        let range = subuid_range(text, "builtpanel-agent").unwrap();
        assert_eq!(range, (165_536, 65_536));
        assert_eq!(host_uid(range, 10_000).unwrap(), 165_536 + 9_999);
        assert_eq!(host_uid(range, 59_999).unwrap(), 165_536 + 59_998);
        for bad in [0, 1, 9_999, 60_000, 65_534] {
            assert_eq!(
                host_uid(range, bad).unwrap_err().0,
                ErrorCode::InvalidArguments,
                "{bad}"
            );
        }
        // A range that is too small, or none at all.
        assert_eq!(
            host_uid((165_536, 20_000), 30_000).unwrap_err().0,
            ErrorCode::Failed
        );
        assert_eq!(subuid_range(text, "nobody"), None);
        assert_eq!(
            subuid_range("builtpanel-agent:x:y", "builtpanel-agent"),
            None
        );
        assert_eq!(
            subuid_range("builtpanel-agent-two:1:2", "builtpanel-agent"),
            None,
            "whole names only"
        );
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dhph-tenants-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn me() -> (u32, u32) {
        let meta = fs::metadata(std::env::temp_dir()).unwrap();
        let probe = std::env::temp_dir().join(format!("dhph-who-{}", std::process::id()));
        fs::write(&probe, b"").unwrap();
        let m = fs::metadata(&probe).unwrap();
        let _ = fs::remove_file(&probe);
        let _ = meta;
        (m.uid(), m.gid())
    }

    #[test]
    fn directories_are_created_private_owned_and_the_second_time_is_fine() {
        let root = scratch("create");
        let (uid, gid) = me();
        let env = TenantEnv {
            root: root.clone(),
            agent_name: "agent".into(),
            subuid_file: "/nonexistent".into(),
            agent_gid: gid,
            mountinfo: "/nonexistent".into(),
            identity: false,
        };
        prepare_as(&env, ID, uid, None).unwrap();
        for path in [root.join(ID), root.join(ID).join("www")] {
            let meta = fs::symlink_metadata(&path).unwrap();
            assert!(meta.is_dir());
            assert_eq!(meta.permissions().mode() & 0o777, 0o750, "{path:?}");
            assert_eq!((meta.uid(), meta.gid()), (uid, gid));
        }
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o755
        );
        prepare_as(&env, ID, uid, None).unwrap(); // again: nothing to do
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn default_files_land_in_www_owned_by_the_tenant_and_only_once() {
        let root = scratch("skeleton");
        let (uid, gid) = me();
        let env = TenantEnv {
            root: root.clone(),
            agent_name: "agent".into(),
            subuid_file: "/nonexistent".into(),
            agent_gid: gid,
            mountinfo: "/nonexistent".into(),
            identity: false,
        };
        let files = [
            SkeletonFile {
                path: "index.html".into(),
                content: "<h1>hi</h1>".into(),
            },
            SkeletonFile {
                path: "css/site.css".into(),
                content: "a{}".into(),
            },
        ];
        prepare_as_with(&env, ID, uid, None, &files).unwrap();
        let www = root.join(ID).join("www");
        assert_eq!(
            fs::read_to_string(www.join("index.html")).unwrap(),
            "<h1>hi</h1>"
        );
        assert_eq!(fs::read_to_string(www.join("css/site.css")).unwrap(), "a{}");
        // A tenant that was finished before keeps what it has, even if it deleted them.
        fs::remove_file(www.join("index.html")).unwrap();
        prepare_as_with(&env, ID, uid, None, &files).unwrap();
        assert!(!www.join("index.html").exists());
        // A bad list is refused before anything is made.
        let other = "0190b6a2-0000-7000-8000-000000000002";
        let bad = [SkeletonFile {
            path: "../escape".into(),
            content: String::new(),
        }];
        assert_eq!(
            prepare_with_files(&env, other, 10_000, &bad).unwrap_err().0,
            ErrorCode::InvalidArguments
        );
        assert!(!root.join(other).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn removal_takes_empty_directories_only_and_is_repeatable() {
        let root = scratch("remove");
        let (uid, gid) = me();
        let env = TenantEnv {
            root: root.clone(),
            agent_name: "agent".into(),
            subuid_file: "/nonexistent".into(),
            agent_gid: gid,
            mountinfo: "/nonexistent".into(),
            identity: false,
        };
        // Nothing there: fine.
        fs::create_dir_all(&root).unwrap();
        remove_as(&env, ID, uid).unwrap();
        // Files inside: refused, nothing deleted.
        prepare_as(&env, ID, uid, None).unwrap();
        let file = root.join(ID).join("www").join("index.html");
        fs::write(&file, b"keep me").unwrap();
        let err = remove_as(&env, ID, uid).unwrap_err();
        assert_eq!(err.0, ErrorCode::Failed);
        assert_eq!(fs::read(&file).unwrap(), b"keep me");
        // Emptied: both directories go, and asking again is fine.
        fs::remove_file(&file).unwrap();
        remove_as(&env, ID, uid).unwrap();
        assert!(!root.join(ID).exists());
        remove_as(&env, ID, uid).unwrap();
        // A symlink in the place of the tenant directory is never followed or removed.
        let target = scratch("remove-target");
        fs::create_dir_all(target.join("www")).unwrap();
        std::os::unix::fs::symlink(&target, root.join(ID)).unwrap();
        assert_eq!(remove_as(&env, ID, uid).unwrap_err().0, ErrorCode::Failed);
        assert!(target.join("www").is_dir());
        assert!(fs::symlink_metadata(root.join(ID)).unwrap().is_symlink());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn a_symlink_a_file_or_a_foreign_owner_in_the_way_is_a_failure() {
        let root = scratch("blocked");
        let (uid, gid) = me();
        let env = TenantEnv {
            root: root.clone(),
            agent_name: "agent".into(),
            subuid_file: "/nonexistent".into(),
            agent_gid: gid,
            mountinfo: "/nonexistent".into(),
            identity: false,
        };
        fs::create_dir_all(&root).unwrap();
        // A symlink where the tenant directory should be: never followed.
        let target = scratch("elsewhere");
        fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, root.join(ID)).unwrap();
        let err = prepare_as(&env, ID, uid, None).unwrap_err();
        assert_eq!(err.0, ErrorCode::Failed);
        assert!(
            fs::read_dir(&target).unwrap().next().is_none(),
            "nothing was created behind the link"
        );
        fs::remove_file(root.join(ID)).unwrap();
        // A plain file.
        fs::write(root.join(ID), b"x").unwrap();
        assert_eq!(
            prepare_as(&env, ID, uid, None).unwrap_err().0,
            ErrorCode::Failed
        );
        fs::remove_file(root.join(ID)).unwrap();
        // The right directory but owned by someone else (when we are not that someone).
        prepare_as(&env, ID, uid, None).unwrap();
        let other = if uid == 0 { 1 } else { uid.wrapping_add(1) };
        assert!(
            prepare_as(&env, ID, other, None).is_err(),
            "a directory owned by another id is not accepted"
        );
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn requests_with_bad_arguments_never_touch_the_disk() {
        let root = scratch("bad");
        let env = TenantEnv {
            root: root.clone(),
            agent_name: "builtpanel-agent".into(),
            subuid_file: "/nonexistent".into(),
            agent_gid: 0,
            mountinfo: "/nonexistent".into(),
            identity: false,
        };
        assert_eq!(
            prepare(&env, "../../etc", 10_000).unwrap_err().0,
            ErrorCode::InvalidArguments
        );
        assert_eq!(
            prepare(&env, ID, 0).unwrap_err().0,
            ErrorCode::InvalidArguments
        );
        assert!(!root.exists());
        // A good request that cannot read the subuid file fails clearly.
        assert_eq!(prepare(&env, ID, 10_000).unwrap_err().0, ErrorCode::Failed);
        assert!(!root.exists());
    }

    #[test]
    fn storage_quota_requests_are_checked_and_unsupported_filesystems_say_so() {
        let root = scratch("quota");
        let (_, gid) = me();
        let env = TenantEnv {
            root: root.clone(),
            agent_name: "builtpanel-agent".into(),
            subuid_file: "/nonexistent".into(),
            agent_gid: gid,
            mountinfo: "/proc/self/mountinfo".into(),
            identity: false,
        };
        let code = |r: Result<(), Failure>| r.unwrap_err().0;
        let ok_limits = (Some(5 << 20), Some(2_000));
        assert_eq!(
            code(set_storage_quota(
                &env,
                "../x",
                10_000,
                ok_limits.0,
                ok_limits.1
            )),
            ErrorCode::InvalidArguments
        );
        assert_eq!(
            code(set_storage_quota(&env, ID, 9_999, ok_limits.0, ok_limits.1)),
            ErrorCode::InvalidArguments
        );
        assert_eq!(
            code(set_storage_quota(&env, ID, 10_000, Some(5), None)),
            ErrorCode::InvalidArguments
        );
        assert_eq!(
            code(set_storage_quota(&env, ID, 10_000, None, Some(1))),
            ErrorCode::InvalidArguments
        );
        // Valid, on a filesystem without project quotas (a temp directory).
        assert_eq!(
            code(set_storage_quota(
                &env,
                ID,
                10_000,
                ok_limits.0,
                ok_limits.1
            )),
            ErrorCode::Unsupported
        );
        // And when the mount table cannot be read at all.
        let unreadable = TenantEnv {
            mountinfo: "/nonexistent".into(),
            ..env
        };
        assert_eq!(
            code(set_storage_quota(
                &unreadable,
                ID,
                10_000,
                ok_limits.0,
                ok_limits.1
            )),
            ErrorCode::Failed
        );
        assert!(!root.exists(), "nothing was created");
    }
}
