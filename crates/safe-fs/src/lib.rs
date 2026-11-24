//! File access that cannot leave a directory (T08.01).
//!
//! Every file operation of the file manager starts from a [`Root`]: a directory
//! held open by its descriptor. A path given to it is a plain relative path
//! (`www/index.php`); it is checked as text first (no `..`, no absolute path, no NUL
//! byte, sane lengths) and then followed **by the kernel** with `openat2` and
//! `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV`,
//! so a symlink a tenant made (to `/etc`, to `/proc/self/...`, to a sibling's
//! directory) is never followed, whatever the path says and whenever it was made.
//! A kernel without `openat2` (before 5.6) gets the same result from walking the
//! path one name at a time with `O_NOFOLLOW`.
//!
//! The functions return descriptors; the caller does the rest (read, write, `fstat`,
//! `unlinkat`, `renameat`) relative to them, never with a path string again.

use std::{
    ffi::OsStr,
    os::fd::{AsFd, OwnedFd},
    path::Path,
};

use rustix::{
    fs::{AtFlags, Mode, OFlags, ResolveFlags, openat, openat2},
    io::Errno,
};

/// Longest path accepted, in bytes.
pub const MAX_PATH: usize = 4096;
/// Longest single name accepted, in bytes.
pub const MAX_NAME: usize = 255;
/// Deepest nesting accepted.
pub const MAX_DEPTH: usize = 64;

/// A path that cannot be used, said before anything is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    /// Starts with `/`: paths are relative to the root.
    #[error("the path must be relative")]
    Absolute,
    /// Has a `..` part.
    #[error("the path must not go up (`..`)")]
    Up,
    /// Has a NUL byte.
    #[error("the path has a NUL byte")]
    Nul,
    /// Longer than [`MAX_PATH`], a name longer than [`MAX_NAME`], or deeper than [`MAX_DEPTH`].
    #[error("the path is too long")]
    TooLong,
    /// Names nothing (for an operation that needs a name, like creating `""`).
    #[error("the path names nothing")]
    Empty,
}

/// Why a file operation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FsError {
    /// The path is not acceptable.
    #[error(transparent)]
    Path(#[from] PathError),
    /// The path goes through a symbolic link (or would leave the root): never followed.
    #[error("the path goes through a link")]
    Link,
    /// Nothing there.
    #[error("not found")]
    NotFound,
    /// Already there.
    #[error("already exists")]
    Exists,
    /// A part of the path is a file, not a directory.
    #[error("not a directory")]
    NotDir,
    /// It is a directory.
    #[error("is a directory")]
    IsDir,
    /// Not allowed (the rights of the user the work is done as).
    #[error("permission denied")]
    Denied,
    /// Any other failure of the system call.
    #[error("system error {0}")]
    Other(i32),
}

impl From<Errno> for FsError {
    fn from(e: Errno) -> Self {
        match e {
            // `openat2` says ELOOP for a link and EXDEV for leaving the root or
            // crossing a mount; the walk says ELOOP for a final link and ENOTDIR for
            // one opened as a directory (so `NotDir` can also mean a link).
            Errno::LOOP | Errno::XDEV => Self::Link,
            Errno::NOENT => Self::NotFound,
            Errno::EXIST => Self::Exists,
            Errno::NOTDIR => Self::NotDir,
            Errno::ISDIR => Self::IsDir,
            Errno::ACCESS | Errno::PERM | Errno::ROFS => Self::Denied,
            other => Self::Other(other.raw_os_error()),
        }
    }
}

/// How paths are followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `openat2` when the kernel has it, the walk when it does not.
    Auto,
    /// Always the walk (for tests, and for kernels without `openat2`).
    Walk,
}

/// The parts of a checked relative path.
///
/// # Errors
/// [`PathError`]: absolute, `..`, NUL, too long or too deep. Empty parts and `.` are
/// dropped (`a//b/./c` is `a/b/c`); `""` and `"."` are the root itself (no parts).
pub fn parts(path: &str) -> Result<Vec<&str>, PathError> {
    if path.len() > MAX_PATH {
        return Err(PathError::TooLong);
    }
    if path.contains('\0') {
        return Err(PathError::Nul);
    }
    if path.starts_with('/') {
        return Err(PathError::Absolute);
    }
    let mut out = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(PathError::Up),
            name if name.len() > MAX_NAME => return Err(PathError::TooLong),
            name => out.push(name),
        }
    }
    if out.len() > MAX_DEPTH {
        return Err(PathError::TooLong);
    }
    Ok(out)
}

/// A directory that everything below is opened from.
#[derive(Debug)]
pub struct Root {
    fd: OwnedFd,
    method: Method,
}

impl Root {
    /// Opens `dir` as the root. The directory itself must not be a symbolic link.
    ///
    /// # Errors
    /// [`FsError`]: not there, not a directory, a link, or not allowed.
    pub fn open(dir: &Path) -> Result<Self, FsError> {
        Self::open_with(dir, Method::Auto)
    }

    /// [`Root::open`] with a chosen [`Method`].
    ///
    /// # Errors
    /// As [`Root::open`].
    pub fn open_with(dir: &Path, method: Method) -> Result<Self, FsError> {
        let fd = rustix::fs::open(
            dir,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(Self { fd, method })
    }

    /// The root's own descriptor (for operations on the root itself).
    #[must_use]
    pub fn fd(&self) -> &OwnedFd {
        &self.fd
    }

    /// Opens `path` below the root with `flags`. Links are never followed, not in
    /// the middle and not at the end (so a link itself cannot be opened for reading;
    /// it can be removed or replaced through [`Root::parent`]).
    ///
    /// # Errors
    /// [`FsError`].
    pub fn open_at(&self, path: &str, flags: OFlags, mode: Mode) -> Result<OwnedFd, FsError> {
        let parts = parts(path)?;
        self.open_parts(&parts, flags, mode)
    }

    /// Opens a directory below the root (the root itself for `""`).
    ///
    /// # Errors
    /// [`FsError`].
    pub fn open_dir(&self, path: &str) -> Result<OwnedFd, FsError> {
        self.open_at(path, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty())
    }

    /// The directory that holds `path` and the name it has there, for creating,
    /// removing and renaming: the work is then `*at(dir, name)`.
    ///
    /// # Errors
    /// [`PathError::Empty`] for the root itself, otherwise as [`Root::open_at`].
    pub fn parent(&self, path: &str) -> Result<(OwnedFd, String), FsError> {
        let mut parts = parts(path)?;
        let name = parts.pop().ok_or(PathError::Empty)?;
        let dir = self.open_parts(&parts, OFlags::RDONLY | OFlags::DIRECTORY, Mode::empty())?;
        Ok((dir, name.to_owned()))
    }

    fn open_parts(&self, parts: &[&str], flags: OFlags, mode: Mode) -> Result<OwnedFd, FsError> {
        let flags = flags | OFlags::CLOEXEC | OFlags::NOFOLLOW;
        if parts.is_empty() {
            // The root itself: a new descriptor with the flags asked for.
            return Ok(openat(&self.fd, ".", flags, mode)?);
        }
        let joined = parts.join("/");
        if self.method == Method::Auto {
            let resolve = ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV;
            match openat2(&self.fd, joined.as_str(), flags, mode, resolve) {
                Err(Errno::NOSYS) => {}
                other => return Ok(other?),
            }
        }
        walk(&self.fd, parts, flags, mode)
    }
}

/// The same as `openat2` with the flags above, one name at a time.
fn walk(start: &impl AsFd, parts: &[&str], flags: OFlags, mode: Mode) -> Result<OwnedFd, FsError> {
    let (last, dirs) = parts.split_last().ok_or(PathError::Empty)?;
    let mut current: Option<OwnedFd> = None;
    for name in dirs {
        let from = current.as_ref().map_or_else(|| start.as_fd(), AsFd::as_fd);
        let next = openat(
            from,
            OsStr::new(name),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        current = Some(next);
    }
    let from = current.as_ref().map_or_else(|| start.as_fd(), AsFd::as_fd);
    Ok(openat(from, OsStr::new(last), flags, mode)?)
}

/// Removes the file or (empty) directory `name` of `dir`, links included, without
/// following anything.
///
/// # Errors
/// [`FsError`].
pub fn remove(dir: &impl AsFd, name: &str, directory: bool) -> Result<(), FsError> {
    check_name(name)?;
    let flags = if directory {
        AtFlags::REMOVEDIR
    } else {
        AtFlags::empty()
    };
    Ok(rustix::fs::unlinkat(dir, name, flags)?)
}

/// A single name for use in a directory (not empty, no `/`, not `.` or `..`, no NUL).
///
/// # Errors
/// [`PathError`].
pub fn check_name(name: &str) -> Result<(), PathError> {
    if name.is_empty() || name == "." {
        return Err(PathError::Empty);
    }
    if name == ".." {
        return Err(PathError::Up);
    }
    if name.contains('\0') {
        return Err(PathError::Nul);
    }
    if name.contains('/') {
        return Err(PathError::Absolute);
    }
    if name.len() > MAX_NAME {
        return Err(PathError::TooLong);
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::{
        io::{Read as _, Write as _},
        os::unix::fs::symlink,
        path::PathBuf,
    };

    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("bp-safefs-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A tenant's directory with ordinary things and every kind of trap, and a
    /// secret next to it that must stay out of reach.
    fn world(s: &Scratch) -> (PathBuf, PathBuf) {
        let outside = s.0.join("outside");
        let root = s.0.join("tenant");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret"), "top secret").unwrap();
        std::fs::create_dir_all(root.join("www/sub")).unwrap();
        std::fs::write(root.join("www/index.php"), "<?php").unwrap();
        std::fs::write(root.join("www/sub/deep.txt"), "deep").unwrap();
        symlink(&outside, root.join("out")).unwrap();
        symlink(outside.join("secret"), root.join("www/leak")).unwrap();
        symlink("/proc/self", root.join("proc")).unwrap();
        symlink("/etc", root.join("www/etc")).unwrap();
        symlink("..", root.join("www/up")).unwrap();
        symlink("index.php", root.join("www/inside")).unwrap();
        (root, outside)
    }

    fn read(fd: OwnedFd) -> String {
        let mut text = String::new();
        std::fs::File::from(fd).read_to_string(&mut text).unwrap();
        text
    }

    #[test]
    fn plain_paths_are_checked_as_text() {
        assert_eq!(parts("a/b//c/./d").unwrap(), ["a", "b", "c", "d"]);
        assert!(parts("").unwrap().is_empty());
        assert!(parts(".").unwrap().is_empty());
        assert_eq!(parts("/etc/passwd"), Err(PathError::Absolute));
        assert_eq!(parts("../x"), Err(PathError::Up));
        assert_eq!(parts("a/../x"), Err(PathError::Up));
        assert_eq!(parts("a/b/.."), Err(PathError::Up));
        assert_eq!(parts("a\0b"), Err(PathError::Nul));
        assert_eq!(parts(&"x/".repeat(2100)), Err(PathError::TooLong));
        assert_eq!(parts(&"a".repeat(256)), Err(PathError::TooLong));
        assert_eq!(parts(&"a/".repeat(65)), Err(PathError::TooLong));
        assert!(parts(&"a".repeat(255)).is_ok());
        assert!(check_name("ok.txt").is_ok());
        for bad in ["", ".", "..", "a/b", "a\0b"] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn ordinary_files_open_and_links_never_do() {
        for method in [Method::Auto, Method::Walk] {
            let s = Scratch::new("basic");
            let (root_dir, _) = world(&s);
            let root = Root::open_with(&root_dir, method).unwrap();
            let read_only = |p: &str| root.open_at(p, OFlags::RDONLY, Mode::empty());
            assert_eq!(read(read_only("www/index.php").unwrap()), "<?php");
            assert_eq!(read(read_only("www//sub/./deep.txt").unwrap()), "deep");
            assert!(root.open_dir("").is_ok());
            assert!(root.open_dir("www/sub").is_ok());
            // Every way out or through a link is refused, with the same answer.
            for trap in [
                "out/secret",
                "out",
                "www/leak",
                "proc/environ",
                "proc/cwd",
                "www/etc/passwd",
                "www/up/outside",
                "www/up/up/outside/secret",
                "www/inside",
            ] {
                assert!(
                    matches!(
                        read_only(trap).map(read),
                        Err(FsError::Link | FsError::NotDir)
                    ),
                    "{method:?} {trap}"
                );
            }
            // Paths that are wrong as text never get to the system.
            for bad in [
                "../outside/secret",
                "/etc/passwd",
                "www/../../outside/secret",
                "a\0",
            ] {
                assert!(matches!(read_only(bad), Err(FsError::Path(_))), "{bad:?}");
            }
            assert_eq!(read_only("nothing").map(read), Err(FsError::NotFound));
            assert_eq!(read_only("www/index.php/x").map(read), Err(FsError::NotDir));
        }
    }

    #[test]
    fn creating_and_removing_work_through_the_parent_and_cannot_be_redirected() {
        for method in [Method::Auto, Method::Walk] {
            let s = Scratch::new("create");
            let (root_dir, outside) = world(&s);
            let root = Root::open_with(&root_dir, method).unwrap();
            let (dir, name) = root.parent("www/sub/new.txt").unwrap();
            assert_eq!(name, "new.txt");
            let fd = openat(
                &dir,
                name.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o644),
            )
            .unwrap();
            std::fs::File::from(fd).write_all(b"hi").unwrap();
            assert_eq!(
                std::fs::read_to_string(root_dir.join("www/sub/new.txt")).unwrap(),
                "hi"
            );
            // A link in the way of the parent, or as the name itself.
            for trap in ["out/planted", "www/up/planted"] {
                assert!(
                    matches!(
                        root.parent(trap).err(),
                        Some(FsError::Link | FsError::NotDir)
                    ),
                    "{method:?} {trap}"
                );
            }
            assert_eq!(root.parent("").err(), Some(FsError::Path(PathError::Empty)));
            // Creating through a link that points at an existing file is refused too.
            let (dir, name) = root.parent("www/leak").unwrap();
            let e = openat(
                &dir,
                name.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o644),
            )
            .unwrap_err();
            assert_eq!(FsError::from(e), FsError::Link);
            assert_eq!(
                std::fs::read_to_string(outside.join("secret")).unwrap(),
                "top secret"
            );
            // Removing a link removes the link, not what it points at.
            remove(&dir, "leak", false).unwrap();
            assert!(!root_dir.join("www/leak").exists());
            assert!(outside.join("secret").exists());
            assert_eq!(
                remove(&dir, "../x", false),
                Err(FsError::Path(PathError::Absolute))
            );
            assert_eq!(remove(&dir, "..", false), Err(FsError::Path(PathError::Up)));
        }
    }

    /// Thousands of made-up paths over an alphabet of traps: whatever comes back
    /// open is inside the root, and nothing outside ever shows.
    #[test]
    fn made_up_paths_never_reach_outside_the_root() {
        let s = Scratch::new("fuzz");
        let (root_dir, _) = world(&s);
        let canonical = std::fs::canonicalize(&root_dir).unwrap();
        let alphabet = [
            "..",
            ".",
            "",
            "www",
            "sub",
            "out",
            "proc",
            "leak",
            "etc",
            "up",
            "inside",
            "self",
            "fd",
            "0",
            "root",
            "secret",
            "outside",
            "index.php",
            "deep.txt",
            "passwd",
            "cwd",
            "exe",
            "environ",
            "tenant",
        ];
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for method in [Method::Auto, Method::Walk] {
            let root = Root::open_with(&root_dir, method).unwrap();
            let (mut opened, mut refused) = (0, 0);
            for _ in 0..20_000 {
                let n = usize::try_from(next() % 7).unwrap() + 1;
                let mut path = String::new();
                if next() % 40 == 0 {
                    path.push('/');
                }
                for i in 0..n {
                    if i > 0 {
                        path.push('/');
                    }
                    path.push_str(alphabet[usize::try_from(next()).unwrap() % alphabet.len()]);
                }
                match root.open_at(&path, OFlags::RDONLY, Mode::empty()) {
                    Ok(fd) => {
                        opened += 1;
                        let real = std::fs::read_link(format!(
                            "/proc/self/fd/{}",
                            rustix::fd::AsRawFd::as_raw_fd(&fd)
                        ))
                        .unwrap();
                        assert!(
                            real.starts_with(&canonical),
                            "{method:?} {path:?} -> {real:?}"
                        );
                    }
                    Err(_) => refused += 1,
                }
            }
            assert!(
                opened > 100 && refused > 1000,
                "{method:?}: {opened} opened, {refused} refused"
            );
        }
    }

    #[test]
    fn the_root_itself_must_be_a_real_directory() {
        let s = Scratch::new("root");
        let (root_dir, outside) = world(&s);
        assert!(matches!(
            Root::open(&root_dir.join("out")).err(),
            Some(FsError::Link | FsError::NotDir)
        ));
        assert_eq!(
            Root::open(&root_dir.join("www/index.php")).err(),
            Some(FsError::NotDir)
        );
        assert_eq!(
            Root::open(&outside.join("nope")).err(),
            Some(FsError::NotFound)
        );
        assert!(Root::open(&root_dir).is_ok());
    }
}
