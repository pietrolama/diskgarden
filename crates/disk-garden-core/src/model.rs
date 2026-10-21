use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Unique identifier of an entry inside [`ScanResult::entries`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntryId(pub usize);

/// Filesystem object type detected via `lstat` (`symlink_metadata`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryType {
    Directory,
    File,
    Symlink,
    Other,
}

/// Access and validity status of a filesystem entry during scanning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessStatus {
    /// Entry metadata (and directory listing, if a directory) was read completely.
    Ok,
    /// Access was denied by the OS (`EACCES` or `EPERM`). Never treated as an empty directory.
    PermissionDenied,
    /// A generic I/O error occurred while reading metadata or directory entries.
    IoError,
    /// The entry disappeared between directory enumeration and `lstat` (`ENOENT`).
    Vanished,
    /// The directory resides on a different mount point / device (`st_dev != root_dev`)
    /// and `cross_mounts` was `false`.
    ExcludedMount,
    /// The entry is a symbolic link and `follow_symlinks` was `false`.
    SymlinkSkipped,
    /// Following a symbolic link or bind mount would have caused a directory cycle.
    SymlinkCycleSkipped,
}

impl AccessStatus {
    /// Returns `true` if this status represents an access or I/O error that makes the scan partial.
    pub fn is_error(self) -> bool {
        matches!(self, AccessStatus::PermissionDenied | AccessStatus::IoError)
    }
}

/// Overall lifecycle status of a scan operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    /// Entire hierarchy was traversed without access or I/O errors.
    Completed,
    /// Traversal finished, but one or more entries could not be read (`PermissionDenied` or `IoError`).
    Partial,
    /// Traversal was explicitly cancelled via [`crate::scanner::CancelToken`].
    Cancelled,
    /// The root path itself did not exist or could not be opened at the start of the scan.
    Failed,
}

/// Which size metric to use when sorting, filtering, or computing visual layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeMetric {
    /// Physical space allocated by the filesystem (`st_blocks * 512`).
    Allocated,
    /// Logical byte length reported by `st_size`.
    Apparent,
}

/// Configuration parameters for a directory scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanConfig {
    /// Root directory to scan.
    pub root_path: PathBuf,
    /// Whether to follow symbolic links. Defaults to `false` for safety.
    pub follow_symlinks: bool,
    /// Whether to cross into different filesystem mount points (`st_dev != root_dev`). Defaults to `false`.
    pub cross_mounts: bool,
    /// Minimum interval in milliseconds between emitted progress updates.
    pub progress_interval_ms: u64,
}

impl ScanConfig {
    /// Creates a safe default scan configuration for `root_path`.
    pub fn new(root_path: impl Into<PathBuf>) -> Self {
        Self {
            root_path: root_path.into(),
            follow_symlinks: false,
            cross_mounts: false,
            progress_interval_ms: 75,
        }
    }
}

/// Real-time non-blocking progress snapshot emitted during an active scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanProgress {
    pub dirs_scanned: u64,
    pub files_scanned: u64,
    pub symlinks_scanned: u64,
    pub apparent_bytes_so_far: u64,
    pub allocated_bytes_so_far: u64,
    pub errors_count: u64,
    pub current_rel_path: String,
    pub elapsed_ms: u64,
}

/// A single node (directory, file, symlink, or special file) in the scanned tree arena.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryNode {
    pub id: EntryId,
    pub parent_id: Option<EntryId>,
    pub children_ids: Vec<EntryId>,
    /// Raw POSIX bytes of the path relative to the scan root (empty `b""` for root).
    pub rel_path_raw: Vec<u8>,
    /// Lossy UTF-8 string representation of the relative path for display and search.
    pub rel_path_display: String,
    /// Raw POSIX bytes of the entry's base name.
    pub name_raw: Vec<u8>,
    /// Lossy UTF-8 string representation of the entry's base name.
    pub name_display: String,
    /// `true` if `rel_path_raw` is valid UTF-8, `false` if it contains non-UTF-8 bytes.
    pub is_valid_utf8: bool,
    /// Depth relative to root (`0` for the root directory).
    pub depth: u32,
    pub entry_type: EntryType,
    /// Own logical byte size (`st_size`). For directories, this is the directory inode metadata size.
    pub apparent_bytes: u64,
    /// Own physical allocated byte size (`st_blocks * 512`).
    pub allocated_bytes: u64,
    /// Total logical bytes of this node and all its descendants (deduplicated for hard links).
    pub subtree_apparent_bytes: u64,
    /// Total physical allocated bytes of this node and all its descendants (deduplicated for hard links).
    pub subtree_allocated_bytes: u64,
    /// Sum of `apparent_bytes` of direct child files in this directory.
    pub direct_files_apparent_bytes: u64,
    /// Sum of `allocated_bytes` of direct child files in this directory.
    pub direct_files_allocated_bytes: u64,
    /// Number of regular files in this node's subtree.
    pub subtree_files_count: u64,
    /// Number of subdirectories in this node's subtree (excluding self).
    pub subtree_dirs_count: u64,
    /// Number of symbolic links in this node's subtree.
    pub subtree_symlinks_count: u64,
    /// Number of access/IO errors encountered in this node or its descendants.
    pub subtree_error_count: u64,
    /// POSIX device ID (`st_dev`).
    pub dev: u64,
    /// POSIX inode number (`st_ino`).
    pub ino: u64,
    /// POSIX hard link count (`st_nlink`).
    pub nlink: u64,
    /// `true` if another entry with the same `(dev, ino)` was already counted earlier in this scan.
    pub is_hardlink_duplicate: bool,
    /// If `is_hardlink_duplicate` is `true`, points to the first [`EntryId`] that counted this inode.
    pub primary_hardlink_id: Option<EntryId>,
    /// Total number of paths in this scan that share this `(dev, ino)`.
    pub shared_link_paths_in_scan: u32,
    /// Last modification time (seconds since Unix epoch).
    pub mtime_sec: Option<i64>,
    /// Last modification time nanoseconds part.
    pub mtime_nsec: Option<u32>,
    pub access_status: AccessStatus,
    pub error_message: Option<String>,
}

impl EntryNode {
    /// Returns the effective size of this entry (subtree size for directories, own size for files)
    /// according to the chosen [`SizeMetric`].
    pub fn effective_bytes(&self, metric: SizeMetric) -> u64 {
        match (self.entry_type, metric) {
            (EntryType::Directory, SizeMetric::Allocated) => self.subtree_allocated_bytes,
            (EntryType::Directory, SizeMetric::Apparent) => self.subtree_apparent_bytes,
            (_, SizeMetric::Allocated) => self.allocated_bytes,
            (_, SizeMetric::Apparent) => self.apparent_bytes,
        }
    }

    /// Returns `true` if this is a regular file whose physical allocated space is smaller than
    /// its logical apparent size (sparse file).
    pub fn is_sparse(&self) -> bool {
        self.entry_type == EntryType::File
            && self.apparent_bytes > 0
            && self.allocated_bytes < self.apparent_bytes
    }

    /// Returns `true` if this non-directory entry has hard links (`st_nlink > 1`) that are not
    /// all accounted for inside the scanned root directory.
    pub fn has_external_hardlinks(&self) -> bool {
        self.entry_type != EntryType::Directory
            && self.nlink > 1
            && (self.shared_link_paths_in_scan as u64) < self.nlink
    }
}

/// Complete, authoritative result of a directory scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub root_path_raw: Vec<u8>,
    pub root_path_display: String,
    pub root_dev: u64,
    pub config: ScanConfig,
    pub status: ScanStatus,
    pub started_at_unix_ms: i64,
    pub duration_ms: u64,
    pub root_id: Option<EntryId>,
    pub entries: Vec<EntryNode>,
    /// Total logical bytes across the scanned root (deduplicated for hard links).
    pub total_apparent_bytes: u64,
    /// Total physical allocated bytes across the scanned root (deduplicated for hard links).
    pub total_allocated_bytes: u64,
    pub total_files_count: u64,
    /// Total subdirectories inside the scanned root (excluding the root directory itself).
    pub total_dirs_count: u64,
    pub total_symlinks_count: u64,
    /// Number of additional hard-link paths detected that share an already-counted inode.
    pub hardlink_duplicate_files_count: u64,
    /// Physical allocated bytes saved from double-counting due to hard-link deduplication.
    pub hardlink_saved_allocated_bytes: u64,
    /// Apparent bytes saved from double-counting due to hard-link deduplication.
    pub hardlink_saved_apparent_bytes: u64,
    pub unreadable_dirs_count: u64,
    pub permission_denied_count: u64,
    pub io_errors_count: u64,
    pub vanished_count: u64,
    pub excluded_mounts_count: u64,
    pub symlink_cycles_count: u64,
    pub root_error_message: Option<String>,
}

impl ScanResult {
    /// Looks up an [`EntryNode`] by its [`EntryId`].
    pub fn get(&self, id: EntryId) -> Option<&EntryNode> {
        self.entries.get(id.0)
    }

    /// Returns the root [`EntryNode`] if the scan created one.
    pub fn root(&self) -> Option<&EntryNode> {
        self.root_id.and_then(|id| self.get(id))
    }
}
