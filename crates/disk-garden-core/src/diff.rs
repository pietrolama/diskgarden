use crate::model::{AccessStatus, EntryNode, EntryType, ScanResult, ScanStatus, SizeMetric};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Errors returned when attempting to compare two snapshots or scans.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DiffError {
    #[error("Una scansione incompleta o annullata non può essere confrontata come se fosse completa (lato {side}: {status:?})")]
    IncompleteScanCannotBeCompared {
        side: &'static str,
        status: ScanStatus,
    },
    #[error("Impossibile confrontare scansioni di radici diverse: '{base_root}' vs '{target_root}'")]
    RootPathMismatch {
        base_root: String,
        target_root: String,
    },
}

/// Sort order supported by [`DiffReport`] views (Section 4.2: variazione assoluta, dimensione attuale, percorso).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffSortField {
    /// Sort by `|delta|` (largest magnitude of change first when descending).
    AbsoluteDelta,
    /// Sort by signed `delta` (largest positive growth first when descending).
    SignedDelta,
    /// Sort by current (`target`) size.
    CurrentSize,
    /// Sort lexicographically by relative path.
    RelativePath,
}

/// Classification of directory subtree change between two scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryChangeKind {
    Added,
    Removed,
    Grown,
    Shrunk,
}

/// Directory subtree delta entry in [`DiffReport`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryDiffItem {
    pub rel_path_display: String,
    pub name_display: String,
    pub depth: u32,
    pub change_kind: DirectoryChangeKind,
    pub base_allocated_bytes: u64,
    pub target_allocated_bytes: u64,
    pub delta_allocated_bytes: i64,
    pub base_apparent_bytes: u64,
    pub target_apparent_bytes: u64,
    pub delta_apparent_bytes: i64,
    pub base_files_count: u64,
    pub target_files_count: u64,
    pub delta_files_count: i64,
}

/// Added or removed file entry in [`DiffReport`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiffItem {
    pub rel_path_display: String,
    pub name_display: String,
    pub allocated_bytes: u64,
    pub apparent_bytes: u64,
    pub delta_allocated_bytes: i64,
    pub delta_apparent_bytes: i64,
    pub mtime_sec: Option<i64>,
    pub ino: u64,
}

/// Existing file whose size (`allocated_bytes` or `apparent_bytes`) changed between scans.
///
/// Per Section 4.2, files whose metadata (`mtime`/`ino`) changed *without* a size change
/// are strictly excluded from this list and reported separately in `metadata_only_modified_files`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileResizeItem {
    pub rel_path_display: String,
    pub name_display: String,
    pub base_allocated_bytes: u64,
    pub target_allocated_bytes: u64,
    pub delta_allocated_bytes: i64,
    pub base_apparent_bytes: u64,
    pub target_apparent_bytes: u64,
    pub delta_apparent_bytes: i64,
    pub base_mtime_sec: Option<i64>,
    pub target_mtime_sec: Option<i64>,
    /// True if `st_ino` changed at the same relative path (e.g., atomic save/replace via rename).
    pub replaced_inode: bool,
}

/// Existing file whose size (`allocated_bytes` and `apparent_bytes`) remained identical,
/// but whose metadata (`mtime` or `ino`) changed.
///
/// Kept separate from [`FileResizeItem`] so DiskGarden never confuses a modified file
/// with a file that increased in size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataOnlyChangeItem {
    pub rel_path_display: String,
    pub name_display: String,
    pub allocated_bytes: u64,
    pub apparent_bytes: u64,
    pub base_mtime_sec: Option<i64>,
    pub target_mtime_sec: Option<i64>,
    pub replaced_inode: bool,
    pub technical_note: String,
}

/// Reason why a path could not be reliably compared between the two scans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnreliableReason {
    /// The entry or an ancestor directory had `PermissionDenied`, `IoError`, or `Vanished` in the base scan.
    BaseAccessIncomplete(AccessStatus),
    /// The entry or an ancestor directory had `PermissionDenied`, `IoError`, or `Vanished` in the target scan.
    TargetAccessIncomplete(AccessStatus),
    /// Both scans encountered access errors on this path.
    BothAccessIncomplete {
        base: AccessStatus,
        target: AccessStatus,
    },
    /// A path changed entry type between scans (e.g. a file was replaced by a directory of the same name).
    EntryTypeChanged {
        base_type: EntryType,
        target_type: EntryType,
    },
}

/// A path that could not be reliably compared between the base and target scans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnreliableDiffItem {
    pub rel_path_display: String,
    pub name_display: String,
    pub reason: UnreliableReason,
    pub explanation: String,
}

/// Complete temporal comparison report between a `base` scan/snapshot and a `target` scan/snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffReport {
    pub root_path_display: String,
    pub base_snapshot_id: Option<i64>,
    pub target_snapshot_id: Option<i64>,
    pub base_label: Option<String>,
    pub target_label: Option<String>,
    pub base_created_at_unix_ms: i64,
    pub target_created_at_unix_ms: i64,
    pub base_status: ScanStatus,
    pub target_status: ScanStatus,

    // Aggregate totals & net deltas
    pub base_total_allocated_bytes: u64,
    pub target_total_allocated_bytes: u64,
    pub net_allocated_delta_bytes: i64,
    pub base_total_apparent_bytes: u64,
    pub target_total_apparent_bytes: u64,
    pub net_apparent_delta_bytes: i64,
    pub base_total_files_count: u64,
    pub target_total_files_count: u64,
    pub net_files_delta_count: i64,
    pub base_total_dirs_count: u64,
    pub target_total_dirs_count: u64,
    pub net_dirs_delta_count: i64,

    // Categorized diff results
    pub directory_deltas: Vec<DirectoryDiffItem>,
    pub added_files: Vec<FileDiffItem>,
    pub removed_files: Vec<FileDiffItem>,
    pub resized_files: Vec<FileResizeItem>,
    pub metadata_only_modified_files: Vec<MetadataOnlyChangeItem>,
    pub unreliable_paths: Vec<UnreliableDiffItem>,

    // Warnings & methodological notes (Section 4.2)
    pub compatibility_warnings: Vec<String>,
    pub methodology_note: String,
}

/// Options for computing and sorting a [`DiffReport`].
#[derive(Debug, Clone)]
pub struct DiffOptions {
    pub sort_field: DiffSortField,
    pub descending: bool,
    pub metric: SizeMetric,
    pub max_items_per_category: usize,
    pub base_snapshot_id: Option<i64>,
    pub target_snapshot_id: Option<i64>,
    pub base_label: Option<String>,
    pub target_label: Option<String>,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            sort_field: DiffSortField::SignedDelta,
            descending: true,
            metric: SizeMetric::Allocated,
            max_items_per_category: 300,
            base_snapshot_id: None,
            target_snapshot_id: None,
            base_label: None,
            target_label: None,
        }
    }
}

/// Compares `base` (older snapshot) and `target` (newer snapshot or current live scan).
///
/// Enforces Section 4.2 rules:
/// - Rejects `Cancelled` or `Failed` scans so an incomplete scan is never compared as if complete.
/// - Requires matching `root_path_raw` between `base` and `target`.
/// - Matches entries by exact raw POSIX relative path bytes (`rel_path_raw`).
/// - Strictly separates `resized_files` (actual byte delta != 0) from `metadata_only_modified_files`
///   (`delta == 0` with `mtime`/`ino` change).
/// - Excludes unreadable subtrees (`PermissionDenied`, `IoError`, `Vanished`) from false "Added/Removed"
///   claims and lists them explicitly in `unreliable_paths`.
pub fn compare_scans(
    base: &ScanResult,
    target: &ScanResult,
    options: &DiffOptions,
) -> Result<DiffReport, DiffError> {
    if base.status == ScanStatus::Cancelled || base.status == ScanStatus::Failed {
        return Err(DiffError::IncompleteScanCannotBeCompared {
            side: "base",
            status: base.status,
        });
    }
    if target.status == ScanStatus::Cancelled || target.status == ScanStatus::Failed {
        return Err(DiffError::IncompleteScanCannotBeCompared {
            side: "target",
            status: target.status,
        });
    }

    if base.root_path_raw != target.root_path_raw {
        return Err(DiffError::RootPathMismatch {
            base_root: base.root_path_display.clone(),
            target_root: target.root_path_display.clone(),
        });
    }

    let mut compatibility_warnings = Vec::new();
    if base.status == ScanStatus::Partial || target.status == ScanStatus::Partial {
        compatibility_warnings.push(
            "Almeno una delle due scansioni ha stato 'Partial' (directory con permessi negati o errori I/O). \
             I sottoalberi non leggibili sono stati isolati nella sezione 'Percorsi non confrontabili in modo affidabile'."
                .to_string(),
        );
    }

    // Index entries by raw relative path bytes (`rel_path_raw`)
    let mut base_by_path: HashMap<&[u8], &EntryNode> = HashMap::with_capacity(base.entries.len());
    let mut target_by_path: HashMap<&[u8], &EntryNode> =
        HashMap::with_capacity(target.entries.len());

    // Track unreadable directory prefixes so children inside an unreadable directory on one side
    // are not falsely reported as added/removed on the other side.
    let mut base_unreadable_prefixes: Vec<&[u8]> = Vec::new();
    let mut target_unreadable_prefixes: Vec<&[u8]> = Vec::new();

    for node in &base.entries {
        base_by_path.insert(node.rel_path_raw.as_slice(), node);
        if node.entry_type == EntryType::Directory && !is_reliable_access(node.access_status) {
            base_unreadable_prefixes.push(node.rel_path_raw.as_slice());
        }
    }
    for node in &target.entries {
        target_by_path.insert(node.rel_path_raw.as_slice(), node);
        if node.entry_type == EntryType::Directory && !is_reliable_access(node.access_status) {
            target_unreadable_prefixes.push(node.rel_path_raw.as_slice());
        }
    }

    let mut all_paths: HashSet<&[u8]> =
        HashSet::with_capacity(base_by_path.len() + target_by_path.len());
    for &k in base_by_path.keys() {
        all_paths.insert(k);
    }
    for &k in target_by_path.keys() {
        all_paths.insert(k);
    }

    let mut directory_deltas = Vec::new();
    let mut added_files = Vec::new();
    let mut removed_files = Vec::new();
    let mut resized_files = Vec::new();
    let mut metadata_only_modified_files = Vec::new();
    let mut unreliable_paths = Vec::new();

    for raw_path in all_paths {
        let b_opt = base_by_path.get(raw_path).copied();
        let t_opt = target_by_path.get(raw_path).copied();

        match (b_opt, t_opt) {
            (Some(b), Some(t)) => {
                // Both scans contain this relative path
                let b_ok = is_reliable_access(b.access_status);
                let t_ok = is_reliable_access(t.access_status);
                if !b_ok || !t_ok {
                    let reason = match (b_ok, t_ok) {
                        (false, true) => UnreliableReason::BaseAccessIncomplete(b.access_status),
                        (true, false) => UnreliableReason::TargetAccessIncomplete(t.access_status),
                        _ => UnreliableReason::BothAccessIncomplete {
                            base: b.access_status,
                            target: t.access_status,
                        },
                    };
                    unreliable_paths.push(UnreliableDiffItem {
                        rel_path_display: display_rel_path(t),
                        name_display: t.name_display.clone(),
                        explanation: format!(
                            "Stato accesso incompleto (base: {:?}, attuale: {:?}). Il contenuto non può essere confrontato con certezza.",
                            b.access_status, t.access_status
                        ),
                        reason,
                    });
                    continue;
                }

                if b.entry_type != t.entry_type {
                    unreliable_paths.push(UnreliableDiffItem {
                        rel_path_display: display_rel_path(t),
                        name_display: t.name_display.clone(),
                        reason: UnreliableReason::EntryTypeChanged {
                            base_type: b.entry_type,
                            target_type: t.entry_type,
                        },
                        explanation: format!(
                            "Il percorso ha cambiato natura da {:?} a {:?} tra le due scansioni.",
                            b.entry_type, t.entry_type
                        ),
                    });
                    continue;
                }

                match t.entry_type {
                    EntryType::Directory => {
                        let d_alloc = t.subtree_allocated_bytes as i64 - b.subtree_allocated_bytes as i64;
                        let d_app = t.subtree_apparent_bytes as i64 - b.subtree_apparent_bytes as i64;
                        let d_files = t.subtree_files_count as i64 - b.subtree_files_count as i64;
                        if d_alloc != 0 || d_app != 0 || d_files != 0 {
                            let primary_delta = match options.metric {
                                SizeMetric::Allocated => d_alloc,
                                SizeMetric::Apparent => d_app,
                            };
                            let change_kind = if primary_delta >= 0 {
                                DirectoryChangeKind::Grown
                            } else {
                                DirectoryChangeKind::Shrunk
                            };
                            directory_deltas.push(DirectoryDiffItem {
                                rel_path_display: display_rel_path(t),
                                name_display: t.name_display.clone(),
                                depth: t.depth,
                                change_kind,
                                base_allocated_bytes: b.subtree_allocated_bytes,
                                target_allocated_bytes: t.subtree_allocated_bytes,
                                delta_allocated_bytes: d_alloc,
                                base_apparent_bytes: b.subtree_apparent_bytes,
                                target_apparent_bytes: t.subtree_apparent_bytes,
                                delta_apparent_bytes: d_app,
                                base_files_count: b.subtree_files_count,
                                target_files_count: t.subtree_files_count,
                                delta_files_count: d_files,
                            });
                        }
                    }
                    EntryType::File => {
                        let d_alloc = t.allocated_bytes as i64 - b.allocated_bytes as i64;
                        let d_app = t.apparent_bytes as i64 - b.apparent_bytes as i64;
                        let replaced_inode = b.ino != 0 && t.ino != 0 && b.ino != t.ino;
                        let mtime_changed = b.mtime_sec != t.mtime_sec || b.mtime_nsec != t.mtime_nsec;

                        if d_alloc != 0 || d_app != 0 {
                            resized_files.push(FileResizeItem {
                                rel_path_display: display_rel_path(t),
                                name_display: t.name_display.clone(),
                                base_allocated_bytes: b.allocated_bytes,
                                target_allocated_bytes: t.allocated_bytes,
                                delta_allocated_bytes: d_alloc,
                                base_apparent_bytes: b.apparent_bytes,
                                target_apparent_bytes: t.apparent_bytes,
                                delta_apparent_bytes: d_app,
                                base_mtime_sec: b.mtime_sec,
                                target_mtime_sec: t.mtime_sec,
                                replaced_inode,
                            });
                        } else if mtime_changed || replaced_inode {
                            metadata_only_modified_files.push(MetadataOnlyChangeItem {
                                rel_path_display: display_rel_path(t),
                                name_display: t.name_display.clone(),
                                allocated_bytes: t.allocated_bytes,
                                apparent_bytes: t.apparent_bytes,
                                base_mtime_sec: b.mtime_sec,
                                target_mtime_sec: t.mtime_sec,
                                replaced_inode,
                                technical_note: if replaced_inode {
                                    "Dimensione invariata (delta 0 B), ma inode e/o timestamp modificati (possibile riscrittura atomica con stessa lunghezza).".to_string()
                                } else {
                                    "Dimensione invariata (delta 0 B), ma timestamp mtime aggiornato. Nessun incremento di spazio.".to_string()
                                },
                            });
                        }
                    }
                    _ => {}
                }
            }
            (None, Some(t)) => {
                // Present only in target (newer scan)
                if is_under_any_prefix(raw_path, &base_unreadable_prefixes) {
                    // Do not claim as newly added if its parent directory was unreadable in base!
                    continue;
                }
                if !is_reliable_access(t.access_status) {
                    unreliable_paths.push(UnreliableDiffItem {
                        rel_path_display: display_rel_path(t),
                        name_display: t.name_display.clone(),
                        reason: UnreliableReason::TargetAccessIncomplete(t.access_status),
                        explanation: format!(
                            "Nuovo percorso rilevato ma non accessibile nella scansione attuale ({:?}).",
                            t.access_status
                        ),
                    });
                    continue;
                }
                match t.entry_type {
                    EntryType::Directory => {
                        directory_deltas.push(DirectoryDiffItem {
                            rel_path_display: display_rel_path(t),
                            name_display: t.name_display.clone(),
                            depth: t.depth,
                            change_kind: DirectoryChangeKind::Added,
                            base_allocated_bytes: 0,
                            target_allocated_bytes: t.subtree_allocated_bytes,
                            delta_allocated_bytes: t.subtree_allocated_bytes as i64,
                            base_apparent_bytes: 0,
                            target_apparent_bytes: t.subtree_apparent_bytes,
                            delta_apparent_bytes: t.subtree_apparent_bytes as i64,
                            base_files_count: 0,
                            target_files_count: t.subtree_files_count,
                            delta_files_count: t.subtree_files_count as i64,
                        });
                    }
                    EntryType::File => {
                        added_files.push(FileDiffItem {
                            rel_path_display: display_rel_path(t),
                            name_display: t.name_display.clone(),
                            allocated_bytes: t.allocated_bytes,
                            apparent_bytes: t.apparent_bytes,
                            delta_allocated_bytes: t.allocated_bytes as i64,
                            delta_apparent_bytes: t.apparent_bytes as i64,
                            mtime_sec: t.mtime_sec,
                            ino: t.ino,
                        });
                    }
                    _ => {}
                }
            }
            (Some(b), None) => {
                // Present only in base (older scan) -> removed in target
                if is_under_any_prefix(raw_path, &target_unreadable_prefixes) {
                    // Do not claim as removed if its parent directory became unreadable in target!
                    continue;
                }
                if !is_reliable_access(b.access_status) {
                    unreliable_paths.push(UnreliableDiffItem {
                        rel_path_display: display_rel_path(b),
                        name_display: b.name_display.clone(),
                        reason: UnreliableReason::BaseAccessIncomplete(b.access_status),
                        explanation: format!(
                            "Percorso con errore di accesso nella scansione base ({:?}) e assente nella scansione attuale.",
                            b.access_status
                        ),
                    });
                    continue;
                }
                match b.entry_type {
                    EntryType::Directory => {
                        directory_deltas.push(DirectoryDiffItem {
                            rel_path_display: display_rel_path(b),
                            name_display: b.name_display.clone(),
                            depth: b.depth,
                            change_kind: DirectoryChangeKind::Removed,
                            base_allocated_bytes: b.subtree_allocated_bytes,
                            target_allocated_bytes: 0,
                            delta_allocated_bytes: -(b.subtree_allocated_bytes as i64),
                            base_apparent_bytes: b.subtree_apparent_bytes,
                            target_apparent_bytes: 0,
                            delta_apparent_bytes: -(b.subtree_apparent_bytes as i64),
                            base_files_count: b.subtree_files_count,
                            target_files_count: 0,
                            delta_files_count: -(b.subtree_files_count as i64),
                        });
                    }
                    EntryType::File => {
                        removed_files.push(FileDiffItem {
                            rel_path_display: display_rel_path(b),
                            name_display: b.name_display.clone(),
                            allocated_bytes: b.allocated_bytes,
                            apparent_bytes: b.apparent_bytes,
                            delta_allocated_bytes: -(b.allocated_bytes as i64),
                            delta_apparent_bytes: -(b.apparent_bytes as i64),
                            mtime_sec: b.mtime_sec,
                            ino: b.ino,
                        });
                    }
                    _ => {}
                }
            }
            (None, None) => {}
        }
    }

    // Apply requested sorting
    sort_directory_diffs(
        &mut directory_deltas,
        options.sort_field,
        options.descending,
        options.metric,
    );
    sort_file_diffs(
        &mut added_files,
        options.sort_field,
        options.descending,
        options.metric,
        true,
    );
    sort_file_diffs(
        &mut removed_files,
        options.sort_field,
        options.descending,
        options.metric,
        false,
    );
    sort_resized_files(
        &mut resized_files,
        options.sort_field,
        options.descending,
        options.metric,
    );
    metadata_only_modified_files.sort_by(|a, b| a.rel_path_display.cmp(&b.rel_path_display));
    unreliable_paths.sort_by(|a, b| a.rel_path_display.cmp(&b.rel_path_display));

    let limit = options.max_items_per_category.max(1);
    directory_deltas.truncate(limit);
    added_files.truncate(limit);
    removed_files.truncate(limit);
    resized_files.truncate(limit);
    metadata_only_modified_files.truncate(limit);
    unreliable_paths.truncate(limit);

    Ok(DiffReport {
        root_path_display: target.root_path_display.clone(),
        base_snapshot_id: options.base_snapshot_id,
        target_snapshot_id: options.target_snapshot_id,
        base_label: options.base_label.clone(),
        target_label: options.target_label.clone(),
        base_created_at_unix_ms: base.started_at_unix_ms,
        target_created_at_unix_ms: target.started_at_unix_ms,
        base_status: base.status,
        target_status: target.status,
        base_total_allocated_bytes: base.total_allocated_bytes,
        target_total_allocated_bytes: target.total_allocated_bytes,
        net_allocated_delta_bytes: target.total_allocated_bytes as i64
            - base.total_allocated_bytes as i64,
        base_total_apparent_bytes: base.total_apparent_bytes,
        target_total_apparent_bytes: target.total_apparent_bytes,
        net_apparent_delta_bytes: target.total_apparent_bytes as i64
            - base.total_apparent_bytes as i64,
        base_total_files_count: base.total_files_count,
        target_total_files_count: target.total_files_count,
        net_files_delta_count: target.total_files_count as i64 - base.total_files_count as i64,
        base_total_dirs_count: base.total_dirs_count,
        target_total_dirs_count: target.total_dirs_count,
        net_dirs_delta_count: target.total_dirs_count as i64 - base.total_dirs_count as i64,
        directory_deltas,
        added_files,
        removed_files,
        resized_files,
        metadata_only_modified_files,
        unreliable_paths,
        compatibility_warnings,
        methodology_note: "Confronto basato su percorsi relativi POSIX (rel_path_raw), dimensioni allocate/apparenti e metadati (inode, mtime) senza hashing crittografico dei contenuti. \
            I file rinominati o spostati appaiono come Rimossi nel vecchio percorso e Aggiunti nel nuovo percorso; i file con mtime/inode modificato ma dimensione identica sono separati dai file cresciuti."
            .to_string(),
    })
}

fn is_reliable_access(status: AccessStatus) -> bool {
    matches!(status, AccessStatus::Ok | AccessStatus::SymlinkSkipped)
}

fn display_rel_path(node: &EntryNode) -> String {
    if node.depth == 0 || node.rel_path_display.is_empty() {
        ".".to_string()
    } else {
        node.rel_path_display.clone()
    }
}

fn is_under_any_prefix(path: &[u8], prefixes: &[&[u8]]) -> bool {
    for &prefix in prefixes {
        if prefix.is_empty() {
            return true;
        }
        if path.len() > prefix.len()
            && path.starts_with(prefix)
            && path[prefix.len()] == b'/'
        {
            return true;
        }
    }
    false
}

fn sort_directory_diffs(
    items: &mut [DirectoryDiffItem],
    sort_field: DiffSortField,
    descending: bool,
    metric: SizeMetric,
) {
    items.sort_by(|a, b| {
        let delta_a = match metric {
            SizeMetric::Allocated => a.delta_allocated_bytes,
            SizeMetric::Apparent => a.delta_apparent_bytes,
        };
        let delta_b = match metric {
            SizeMetric::Allocated => b.delta_allocated_bytes,
            SizeMetric::Apparent => b.delta_apparent_bytes,
        };
        let cur_a = match metric {
            SizeMetric::Allocated => a.target_allocated_bytes,
            SizeMetric::Apparent => a.target_apparent_bytes,
        };
        let cur_b = match metric {
            SizeMetric::Allocated => b.target_allocated_bytes,
            SizeMetric::Apparent => b.target_apparent_bytes,
        };

        let cmp = match sort_field {
            DiffSortField::AbsoluteDelta => delta_a
                .unsigned_abs()
                .cmp(&delta_b.unsigned_abs())
                .then_with(|| a.rel_path_display.cmp(&b.rel_path_display)),
            DiffSortField::SignedDelta => delta_a
                .cmp(&delta_b)
                .then_with(|| a.rel_path_display.cmp(&b.rel_path_display)),
            DiffSortField::CurrentSize => cur_a
                .cmp(&cur_b)
                .then_with(|| a.rel_path_display.cmp(&b.rel_path_display)),
            DiffSortField::RelativePath => a.rel_path_display.cmp(&b.rel_path_display),
        };
        if descending {
            cmp.reverse()
        } else {
            cmp
        }
    });
}

fn sort_file_diffs(
    items: &mut [FileDiffItem],
    sort_field: DiffSortField,
    descending: bool,
    metric: SizeMetric,
    is_added: bool,
) {
    items.sort_by(|a, b| {
        let size_a = match metric {
            SizeMetric::Allocated => a.allocated_bytes,
            SizeMetric::Apparent => a.apparent_bytes,
        };
        let size_b = match metric {
            SizeMetric::Allocated => b.allocated_bytes,
            SizeMetric::Apparent => b.apparent_bytes,
        };
        let delta_a = match metric {
            SizeMetric::Allocated => a.delta_allocated_bytes,
            SizeMetric::Apparent => a.delta_apparent_bytes,
        };
        let delta_b = match metric {
            SizeMetric::Allocated => b.delta_allocated_bytes,
            SizeMetric::Apparent => b.delta_apparent_bytes,
        };
        let cur_a = if is_added { size_a } else { 0 };
        let cur_b = if is_added { size_b } else { 0 };

        let cmp = match sort_field {
            DiffSortField::AbsoluteDelta => size_a
                .cmp(&size_b)
                .then_with(|| a.rel_path_display.cmp(&b.rel_path_display)),
            DiffSortField::SignedDelta => delta_a
                .cmp(&delta_b)
                .then_with(|| a.rel_path_display.cmp(&b.rel_path_display)),
            DiffSortField::CurrentSize => cur_a
                .cmp(&cur_b)
                .then_with(|| size_a.cmp(&size_b))
                .then_with(|| a.rel_path_display.cmp(&b.rel_path_display)),
            DiffSortField::RelativePath => a.rel_path_display.cmp(&b.rel_path_display),
        };
        if descending {
            cmp.reverse()
        } else {
            cmp
        }
    });
}

fn sort_resized_files(
    items: &mut [FileResizeItem],
    sort_field: DiffSortField,
    descending: bool,
    metric: SizeMetric,
) {
    items.sort_by(|a, b| {
        let delta_a = match metric {
            SizeMetric::Allocated => a.delta_allocated_bytes,
            SizeMetric::Apparent => a.delta_apparent_bytes,
        };
        let delta_b = match metric {
            SizeMetric::Allocated => b.delta_allocated_bytes,
            SizeMetric::Apparent => b.delta_apparent_bytes,
        };
        let cur_a = match metric {
            SizeMetric::Allocated => a.target_allocated_bytes,
            SizeMetric::Apparent => a.target_apparent_bytes,
        };
        let cur_b = match metric {
            SizeMetric::Allocated => b.target_allocated_bytes,
            SizeMetric::Apparent => b.target_apparent_bytes,
        };

        let cmp = match sort_field {
            DiffSortField::AbsoluteDelta => delta_a
                .unsigned_abs()
                .cmp(&delta_b.unsigned_abs())
                .then_with(|| a.rel_path_display.cmp(&b.rel_path_display)),
            DiffSortField::SignedDelta => delta_a
                .cmp(&delta_b)
                .then_with(|| a.rel_path_display.cmp(&b.rel_path_display)),
            DiffSortField::CurrentSize => cur_a
                .cmp(&cur_b)
                .then_with(|| a.rel_path_display.cmp(&b.rel_path_display)),
            DiffSortField::RelativePath => a.rel_path_display.cmp(&b.rel_path_display),
        };
        if descending {
            cmp.reverse()
        } else {
            cmp
        }
    });
}
