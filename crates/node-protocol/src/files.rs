//! The file manager's requests and answers (T08.02-T08.06).
//!
//! Every path is relative to the tenant's files and is checked again where it is
//! used (`builtpanel-safe-fs`). A request and its answer travel as one JSON object
//! each, so both stay small: file contents here are text for the editor, and
//! uploads and downloads of bulk data use their own channel.

use serde::{Deserialize, Serialize};

/// Largest page of a directory listing.
pub const MAX_LIST_LIMIT: u32 = 1000;
/// Most entries one directory listing looks at.
pub const MAX_DIR_ENTRIES: usize = 100_000;
/// Largest text the editor reads or writes in one request (it must fit a frame).
pub const MAX_EDIT_BYTES: u32 = 512 * 1024;
/// Name of the folder deleted things go to (at the root of the tenant's files).
pub const TRASH_DIR: &str = ".trash";

/// What a file manager asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileRequest {
    /// A page of a directory, by name (folders first).
    List {
        /// The directory (`""` for the root).
        path: String,
        /// Entries to skip.
        offset: u32,
        /// Entries to give (1..=[`MAX_LIST_LIMIT`]).
        limit: u32,
    },
    /// What one path is.
    Stat {
        /// The path.
        path: String,
    },
    /// A text file.
    Read {
        /// The file.
        path: String,
        /// Refuse (`too_large`) a file bigger than this.
        max_bytes: u32,
    },
    /// Writes a text file, replacing it whole and atomically.
    Write {
        /// The file.
        path: String,
        /// What to write.
        content: String,
        /// Refuse (`too_large`) content bigger than this.
        max_bytes: u32,
        /// Refuse (`conflict`) when the file changed since this modification time
        /// (Unix seconds), which is what an editor saw when it opened it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_mtime: Option<i64>,
        /// Refuse (`exists`) when the file is already there.
        #[serde(default)]
        create_only: bool,
    },
    /// A new directory.
    Mkdir {
        /// The directory.
        path: String,
    },
    /// Renames or moves; never replaces.
    Move {
        /// What to move.
        from: String,
        /// Where to (the new path, not the folder).
        to: String,
    },
    /// Copies a file or a whole directory; never replaces.
    Copy {
        /// What to copy.
        from: String,
        /// The copy's path.
        to: String,
    },
    /// Moves to the trash (the way back is [`FileRequest::Restore`]).
    Delete {
        /// What to delete.
        path: String,
    },
    /// What is in the trash.
    TrashList,
    /// Puts a deleted thing back where it was.
    Restore {
        /// Its trash id.
        id: String,
    },
    /// Removes from the trash for good: one item, everything, or what is older.
    EmptyTrash {
        /// Only this item.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// Only items deleted more than this many days ago.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        older_than_days: Option<u32>,
    },
    /// Sets the permission bits (at most `0777`; no setuid, setgid or sticky bit).
    Chmod {
        /// The path.
        path: String,
        /// The bits, for example `0o644` as 420.
        mode: u32,
        /// For a directory: everything below it too.
        #[serde(default)]
        recursive: bool,
    },
    /// Packs files and folders into one archive file; never replaces.
    Archive {
        /// What to pack (each a file or a folder).
        paths: Vec<String>,
        /// The archive to make (its path, ending in `.zip` or `.tar.gz`).
        dest: String,
        /// Its format.
        format: ArchiveFormat,
        /// Refuse (`too_large`) when the files add up to more than this.
        max_bytes: u64,
    },
    /// Unpacks an archive (`.zip`, `.tar.gz`, `.tgz`) into a folder. The whole
    /// archive is checked first (names that leave the folder, sizes, how many
    /// entries) and nothing is unpacked when anything is wrong; files that are
    /// already there are never replaced; links and special files in it are skipped.
    Extract {
        /// The archive.
        path: String,
        /// The folder to unpack into (made when missing).
        dest: String,
        /// Refuse (`too_large`) when it unpacks to more than this.
        max_bytes: u64,
        /// Refuse (`too_many`) when it has more entries than this.
        max_entries: u32,
    },
    /// How much disk and how many files the tenant's files use, and which folders use
    /// the most. Done at the lowest priority (it walks everything).
    DiskUsage {
        /// How many of the biggest folders to name (1..=100).
        top: u32,
    },
    /// Sets sane bits on a whole tree: directories 755, files 644 (755 if the owner
    /// could run them). Links are left alone.
    FixPermissions {
        /// The path.
        path: String,
    },
}

/// An archive format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    /// `.zip`.
    Zip,
    /// `.tar.gz`.
    TarGz,
}

/// What a path is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A regular file.
    File,
    /// A directory.
    Dir,
    /// A symbolic link (never followed).
    Link,
    /// Anything else (socket, device, pipe).
    Other,
}

/// One thing in a directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Its name.
    pub name: String,
    /// What it is.
    pub kind: EntryKind,
    /// Size in bytes (of a directory: of the directory itself).
    pub size: u64,
    /// Last change of content (Unix seconds).
    pub mtime: i64,
    /// Permission bits (`0o755` is 493).
    pub mode: u32,
}

/// What one folder uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderUsage {
    /// The folder, relative to the tenant's files.
    pub path: String,
    /// Disk used below it, in bytes.
    pub bytes: u64,
    /// Files and folders below it.
    pub inodes: u64,
}

/// Something in the trash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashItem {
    /// Its id.
    pub id: String,
    /// Where it was.
    pub path: String,
    /// What it is.
    pub kind: EntryKind,
    /// Size in bytes (of a directory: of the directory itself).
    pub size: u64,
    /// When it was deleted (Unix seconds).
    pub deleted_at: i64,
}

/// What the node answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum FileResult {
    /// A page of a directory.
    Entries {
        /// The entries asked for.
        entries: Vec<FileEntry>,
        /// How many the directory has (the first [`MAX_DIR_ENTRIES`] are counted).
        total: u32,
    },
    /// One path.
    Entry {
        /// What it is.
        entry: FileEntry,
    },
    /// A text file.
    Content {
        /// The text; none when the file is not UTF-8 text.
        content: Option<String>,
        /// Whether the file is not text.
        binary: bool,
        /// Its size.
        size: u64,
        /// Its modification time (Unix seconds), to give back in a write.
        mtime: i64,
    },
    /// Done; how many files and directories it touched.
    Done {
        /// Items touched.
        count: u32,
    },
    /// What the files use.
    Usage {
        /// Disk used, in bytes (blocks, as the disk counts them), links not followed.
        total_bytes: u64,
        /// Files, folders and links.
        inodes: u64,
        /// The biggest folders (every level counted, the top one left out), biggest first.
        folders: Vec<FolderUsage>,
        /// Whether the walk stopped at the limit (the figures are then a lower bound).
        partial: bool,
    },
    /// The trash.
    Trash {
        /// Newest first.
        items: Vec<TrashItem>,
    },
    /// Refused or failed.
    Error {
        /// Why.
        code: FileErrorCode,
        /// In words.
        message: String,
    },
}

/// Why a file request did not work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileErrorCode {
    /// The path is not acceptable (`..`, absolute, too long, the trash folder).
    InvalidPath,
    /// A link was in the way (never followed).
    Link,
    /// Nothing there.
    NotFound,
    /// Already there.
    Exists,
    /// Not a directory.
    NotDir,
    /// Is a directory.
    IsDir,
    /// The files' owner is not allowed.
    Denied,
    /// Bigger than the limit given.
    TooLarge,
    /// The file changed since the editor opened it.
    Conflict,
    /// Permission bits that are not allowed.
    InvalidMode,
    /// Other bad arguments.
    InvalidArguments,
    /// Too many items.
    TooMany,
    /// The archive is damaged, of another format, or has an entry that is not allowed.
    BadArchive,
    /// Anything else.
    Failed,
}

impl FileResult {
    /// An error answer.
    #[must_use]
    pub fn error(code: FileErrorCode, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
        }
    }
}
