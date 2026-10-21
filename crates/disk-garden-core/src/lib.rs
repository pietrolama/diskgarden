//! # `disk-garden-core`
//!
//! Core library for **DiskGarden** containing:
//! - [`model`]: Authoritative domain types for filesystem entries, sizes, access statuses, and scans.
//! - [`scanner`]: Safe, non-blocking recursive filesystem scanner (`lstat`, hard-link deduplication,
//!   mount boundary guards, cycle prevention, and cooperative cancellation).
//! - [`analyzer`]: Bottom-up subtree aggregation, sorting, filtering, top-N extraction,
//!   bounded treemap layout preparation, and 3-tier recoverable space qualification.

pub mod analyzer;
pub mod diff;
pub mod model;
pub mod scanner;
pub mod snapshot;

pub use analyzer::{
    build_breadcrumbs, build_directory_navigation_view, build_entry_detail_view,
    build_recover_report, build_treemap_view, children_sorted, extension_distribution,
    filter_entries, qualify_entry_space, sort_entry_refs, top_directories, top_files,
    BreadcrumbItem, DirectoryNavigationView, EntryDetailView, EntryFilter, EntrySummary,
    ExtensionStat, RecoverCandidate, RecoverReport, SortField, SpaceQualification, TreemapSubTile,
    TreemapTile,
};
pub use diff::{
    compare_scans, DiffError, DiffOptions, DiffReport, DiffSortField, DirectoryChangeKind,
    DirectoryDiffItem, FileDiffItem, FileResizeItem, MetadataOnlyChangeItem, UnreliableDiffItem,
    UnreliableReason,
};
pub use model::{
    AccessStatus, EntryId, EntryNode, EntryType, ScanConfig, ScanProgress, ScanResult, ScanStatus,
    SizeMetric,
};
pub use scanner::{
    list_system_volumes, raw_bytes_to_path_buf, scan_directory, scan_directory_with_progress,
    CancelToken, DiskVolumeInfo, ScanEvent, ScanHandle,
};
pub use snapshot::{SnapshotError, SnapshotHeader, SnapshotStore, StoredSnapshot};
