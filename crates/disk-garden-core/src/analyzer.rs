use crate::model::{AccessStatus, EntryId, EntryNode, EntryType, ScanResult, SizeMetric};
use crate::scanner::raw_bytes_to_path_buf;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;

/// Field used to sort filesystem entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    AllocatedBytes,
    ApparentBytes,
    Name,
    FilesCount,
    ModifiedTime,
}

/// Filter criteria for searching and inspecting entries inside a [`ScanResult`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntryFilter {
    /// Case-insensitive substring matched against `name_display` or `rel_path_display`.
    pub query: Option<String>,
    /// Optional restriction to a specific [`EntryType`].
    pub entry_type: Option<EntryType>,
    /// Minimum effective size in bytes according to `metric`.
    pub min_bytes: Option<u64>,
    /// Metric used for `min_bytes`.
    pub metric: Option<SizeMetric>,
    /// Only return entries with an access error (`PermissionDenied` or `IoError`) or non-zero `subtree_error_count`.
    pub only_errors: bool,
    /// Only return sparse files (`allocated_bytes < apparent_bytes`).
    pub only_sparse: bool,
    /// Only return files with multiple hard links (`nlink > 1`).
    pub only_hardlinks: bool,
    /// Optional ancestor directory restriction (`None` searches the entire scan).
    pub ancestor_id: Option<EntryId>,
}

/// Aggregated statistics for a file extension across a scanned tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionStat {
    pub extension: String,
    pub files_count: u64,
    pub allocated_bytes: u64,
    pub apparent_bytes: u64,
}

/// A visual tile prepared by [`build_treemap_view`] for the Presentation Layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreemapTile {
    /// `Some(id)` for a real entry node, or `None` for the synthetic tail-bucket tile.
    pub entry_id: Option<EntryId>,
    pub label: String,
    pub rel_path_display: String,
    pub entry_type: EntryType,
    pub is_tail_bucket: bool,
    pub bucketed_items_count: usize,
    pub bucketed_entry_ids: Vec<EntryId>,
    pub allocated_bytes: u64,
    pub apparent_bytes: u64,
    /// Share (`0.0..=1.0`) of the parent directory's total under the chosen metric.
    pub fraction_of_parent: f64,
    pub access_status: AccessStatus,
    pub subtree_error_count: u64,
    /// Nested preview tiles for the largest children of a directory tile (up to 6 items).
    pub children_preview: Vec<TreemapSubTile>,
}

/// Second-level preview tile inside a directory's [`TreemapTile`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreemapSubTile {
    pub entry_id: EntryId,
    pub label: String,
    pub entry_type: EntryType,
    pub allocated_bytes: u64,
    pub apparent_bytes: u64,
    pub fraction_of_dir: f64,
}

/// Breadcrumb item representing an ancestor directory from the scan root to the current node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreadcrumbItem {
    pub id: EntryId,
    pub name_display: String,
    pub rel_path_display: String,
    pub depth: u32,
}

/// Compact summary of an [`EntryNode`] for lists, tables, and search results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntrySummary {
    pub id: EntryId,
    pub parent_id: Option<EntryId>,
    pub name_display: String,
    pub rel_path_display: String,
    pub is_valid_utf8: bool,
    pub depth: u32,
    pub entry_type: EntryType,
    pub apparent_bytes: u64,
    pub allocated_bytes: u64,
    pub subtree_apparent_bytes: u64,
    pub subtree_allocated_bytes: u64,
    pub direct_files_apparent_bytes: u64,
    pub direct_files_allocated_bytes: u64,
    pub subtree_files_count: u64,
    pub subtree_dirs_count: u64,
    pub subtree_symlinks_count: u64,
    pub subtree_error_count: u64,
    pub nlink: u64,
    pub is_hardlink_duplicate: bool,
    pub shared_link_paths_in_scan: u32,
    pub is_sparse: bool,
    pub has_external_hardlinks: bool,
    pub mtime_sec: Option<i64>,
    pub access_status: AccessStatus,
    pub error_message: Option<String>,
}

impl EntrySummary {
    pub fn from_node(node: &EntryNode) -> Self {
        Self {
            id: node.id,
            parent_id: node.parent_id,
            name_display: node.name_display.clone(),
            rel_path_display: node.rel_path_display.clone(),
            is_valid_utf8: node.is_valid_utf8,
            depth: node.depth,
            entry_type: node.entry_type,
            apparent_bytes: node.apparent_bytes,
            allocated_bytes: node.allocated_bytes,
            subtree_apparent_bytes: node.subtree_apparent_bytes,
            subtree_allocated_bytes: node.subtree_allocated_bytes,
            direct_files_apparent_bytes: node.direct_files_apparent_bytes,
            direct_files_allocated_bytes: node.direct_files_allocated_bytes,
            subtree_files_count: node.subtree_files_count,
            subtree_dirs_count: node.subtree_dirs_count,
            subtree_symlinks_count: node.subtree_symlinks_count,
            subtree_error_count: node.subtree_error_count,
            nlink: node.nlink,
            is_hardlink_duplicate: node.is_hardlink_duplicate,
            shared_link_paths_in_scan: node.shared_link_paths_in_scan,
            is_sparse: node.is_sparse(),
            has_external_hardlinks: node.has_external_hardlinks(),
            mtime_sec: node.mtime_sec,
            access_status: node.access_status,
            error_message: node.error_message.clone(),
        }
    }
}

/// Complete navigation payload for a selected directory in `Explore` mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryNavigationView {
    pub directory: EntrySummary,
    pub abs_path_display: String,
    pub breadcrumbs: Vec<BreadcrumbItem>,
    pub treemap_tiles: Vec<TreemapTile>,
    pub total_children_count: usize,
    pub children: Vec<EntrySummary>,
}

/// Detailed inspector payload for any selected entry (`Directory`, `File`, `Symlink`, `Other`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryDetailView {
    pub summary: EntrySummary,
    pub abs_path_display: String,
    pub dev: u64,
    pub ino: u64,
    pub share_of_root_allocated: f64,
    pub share_of_root_apparent: f64,
    pub share_of_parent_allocated: f64,
    /// Three-tier space qualification for `Recover` / Inspector.
    pub space_qualification: SpaceQualification,
}

/// Three-tier qualification required by Section 4.3 (`Recover`):
/// 1. Detected occupied space (`detected_allocated_bytes`, `detected_apparent_bytes`)
/// 2. Potentially recoverable space (`potentially_recoverable_bytes`)
/// 3. Verified recoverable space (`verified_recoverable_bytes`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceQualification {
    pub detected_allocated_bytes: u64,
    pub detected_apparent_bytes: u64,
    pub potentially_recoverable_bytes: u64,
    pub verified_recoverable_bytes: u64,
    pub category_code: String,
    pub category_label: String,
    pub explanation: String,
}

/// A candidate surfaced in `Recover` mode for manual user review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoverCandidate {
    pub entry: EntrySummary,
    pub abs_path_display: String,
    pub qualification: SpaceQualification,
}

/// Complete report for `Recover` mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoverReport {
    pub total_detected_allocated_bytes: u64,
    pub total_potentially_recoverable_bytes: u64,
    pub total_verified_recoverable_bytes: u64,
    pub cache_and_temp_candidates: Vec<RecoverCandidate>,
    pub largest_files: Vec<EntrySummary>,
    pub largest_directories: Vec<EntrySummary>,
}

/// Builds the full [`DirectoryNavigationView`] for `dir_id`, capping `children` to `max_children`
/// and `treemap_tiles` to `max_tiles` so huge directories render effortlessly.
pub fn build_directory_navigation_view(
    scan: &ScanResult,
    dir_id: EntryId,
    sort_field: SortField,
    descending: bool,
    metric: SizeMetric,
    max_tiles: usize,
    max_children: usize,
) -> Option<DirectoryNavigationView> {
    let dir_node = scan.get(dir_id)?;
    let breadcrumbs = build_breadcrumbs(scan, dir_id);
    let treemap_tiles = build_treemap_view(scan, dir_id, metric, max_tiles, 0.004);
    let sorted_children = children_sorted(scan, dir_id, sort_field, descending);
    let total_children_count = sorted_children.len();
    let children: Vec<EntrySummary> = sorted_children
        .into_iter()
        .take(max_children.max(1))
        .map(EntrySummary::from_node)
        .collect();

    Some(DirectoryNavigationView {
        directory: EntrySummary::from_node(dir_node),
        abs_path_display: resolve_abs_path_display(scan, dir_node),
        breadcrumbs,
        treemap_tiles,
        total_children_count,
        children,
    })
}

/// Builds the [`EntryDetailView`] for the inspector panel when an entry is selected.
pub fn build_entry_detail_view(scan: &ScanResult, entry_id: EntryId) -> Option<EntryDetailView> {
    let node = scan.get(entry_id)?;
    let eff_alloc = node.effective_bytes(SizeMetric::Allocated);
    let eff_app = node.effective_bytes(SizeMetric::Apparent);

    let share_of_root_allocated = if scan.total_allocated_bytes > 0 {
        (eff_alloc as f64) / (scan.total_allocated_bytes as f64)
    } else {
        0.0
    };
    let share_of_root_apparent = if scan.total_apparent_bytes > 0 {
        (eff_app as f64) / (scan.total_apparent_bytes as f64)
    } else {
        0.0
    };

    let share_of_parent_allocated = if let Some(pid) = node.parent_id {
        if let Some(parent) = scan.get(pid) {
            let p_alloc = parent.effective_bytes(SizeMetric::Allocated);
            if p_alloc > 0 {
                (eff_alloc as f64) / (p_alloc as f64)
            } else {
                0.0
            }
        } else {
            1.0
        }
    } else {
        1.0
    };

    let space_qualification = qualify_entry_space(scan, node);

    Some(EntryDetailView {
        summary: EntrySummary::from_node(node),
        abs_path_display: resolve_abs_path_display(scan, node),
        dev: node.dev,
        ino: node.ino,
        share_of_root_allocated,
        share_of_root_apparent,
        share_of_parent_allocated,
        space_qualification,
    })
}

/// Returns the breadcrumb trail from the root directory down to `target_id`.
pub fn build_breadcrumbs(scan: &ScanResult, target_id: EntryId) -> Vec<BreadcrumbItem> {
    let mut trail = Vec::new();
    let mut cursor = Some(target_id);
    while let Some(cid) = cursor {
        let Some(node) = scan.get(cid) else {
            break;
        };
        trail.push(BreadcrumbItem {
            id: node.id,
            name_display: if node.depth == 0 {
                scan.root_path_display.clone()
            } else {
                node.name_display.clone()
            },
            rel_path_display: node.rel_path_display.clone(),
            depth: node.depth,
        });
        cursor = node.parent_id;
    }
    trail.reverse();
    trail
}

/// Returns the immediate children of `dir_id` sorted according to `sort_field` and `descending`.
pub fn children_sorted<'a>(
    scan: &'a ScanResult,
    dir_id: EntryId,
    sort_field: SortField,
    descending: bool,
) -> Vec<&'a EntryNode> {
    let Some(dir_node) = scan.get(dir_id) else {
        return Vec::new();
    };
    let mut children: Vec<&'a EntryNode> = dir_node
        .children_ids
        .iter()
        .filter_map(|&cid| scan.get(cid))
        .collect();

    sort_entry_refs(&mut children, sort_field, descending);
    children
}

/// Returns the top `limit` largest regular files across the entire scan, sorted descending by `metric`.
pub fn top_files(
    scan: &ScanResult,
    limit: usize,
    metric: SizeMetric,
    include_hardlink_duplicates: bool,
) -> Vec<&EntryNode> {
    if limit == 0 {
        return Vec::new();
    }
    let mut files: Vec<&EntryNode> = scan
        .entries
        .iter()
        .filter(|e| {
            e.entry_type == EntryType::File
                && (include_hardlink_duplicates || !e.is_hardlink_duplicate)
        })
        .collect();

    let sort_field = match metric {
        SizeMetric::Allocated => SortField::AllocatedBytes,
        SizeMetric::Apparent => SortField::ApparentBytes,
    };
    sort_entry_refs(&mut files, sort_field, true);
    files.truncate(limit);
    files
}

/// Returns the top `limit` largest directories (excluding the root directory itself)
/// sorted descending by their subtree size under `metric`.
pub fn top_directories(scan: &ScanResult, limit: usize, metric: SizeMetric) -> Vec<&EntryNode> {
    if limit == 0 {
        return Vec::new();
    }
    let mut dirs: Vec<&EntryNode> = scan
        .entries
        .iter()
        .filter(|e| e.entry_type == EntryType::Directory && e.depth > 0)
        .collect();

    let sort_field = match metric {
        SizeMetric::Allocated => SortField::AllocatedBytes,
        SizeMetric::Apparent => SortField::ApparentBytes,
    };
    sort_entry_refs(&mut dirs, sort_field, true);
    dirs.truncate(limit);
    dirs
}

/// Searches and filters entries across the scan, returning results sorted by `sort_field`.
pub fn filter_entries<'a>(
    scan: &'a ScanResult,
    filter: &EntryFilter,
    sort_field: SortField,
    descending: bool,
) -> Vec<&'a EntryNode> {
    let query_lower = filter
        .query
        .as_ref()
        .map(|q| q.trim().to_lowercase())
        .filter(|q| !q.is_empty());
    let metric = filter.metric.unwrap_or(SizeMetric::Allocated);

    let ancestor_prefix: Option<&[u8]> = filter
        .ancestor_id
        .and_then(|aid| scan.get(aid))
        .and_then(|anode| {
            if anode.depth == 0 {
                None
            } else {
                Some(anode.rel_path_raw.as_slice())
            }
        });

    let mut matched: Vec<&'a EntryNode> = scan
        .entries
        .iter()
        .filter(|node| {
            if node.depth == 0 {
                return false;
            }
            if let Some(prefix) = ancestor_prefix {
                if !is_strict_descendant_raw(&node.rel_path_raw, prefix) {
                    return false;
                }
            }
            if let Some(et) = filter.entry_type {
                if node.entry_type != et {
                    return false;
                }
            }
            if let Some(min_b) = filter.min_bytes {
                if node.effective_bytes(metric) < min_b {
                    return false;
                }
            }
            if filter.only_errors
                && !node.access_status.is_error()
                && node.subtree_error_count == 0
            {
                return false;
            }
            if filter.only_sparse && !node.is_sparse() {
                return false;
            }
            if filter.only_hardlinks && (node.entry_type == EntryType::Directory || node.nlink <= 1)
            {
                return false;
            }
            if let Some(ref q) = query_lower {
                let name_matches = node.name_display.to_lowercase().contains(q);
                let path_matches = node.rel_path_display.to_lowercase().contains(q);
                if !name_matches && !path_matches {
                    return false;
                }
            }
            true
        })
        .collect();

    sort_entry_refs(&mut matched, sort_field, descending);
    matched
}

/// Computes file size breakdown by lowercase file extension across all non-duplicate regular files.
pub fn extension_distribution(scan: &ScanResult) -> Vec<ExtensionStat> {
    let mut map: HashMap<String, ExtensionStat> = HashMap::new();

    for node in &scan.entries {
        if node.entry_type != EntryType::File || node.is_hardlink_duplicate {
            continue;
        }
        let ext = extract_extension(&node.name_display);
        let entry = map.entry(ext.clone()).or_insert(ExtensionStat {
            extension: ext,
            files_count: 0,
            allocated_bytes: 0,
            apparent_bytes: 0,
        });
        entry.files_count += 1;
        entry.allocated_bytes = entry.allocated_bytes.saturating_add(node.allocated_bytes);
        entry.apparent_bytes = entry.apparent_bytes.saturating_add(node.apparent_bytes);
    }

    let mut stats: Vec<ExtensionStat> = map.into_values().collect();
    stats.sort_by(|a, b| {
        b.allocated_bytes
            .cmp(&a.allocated_bytes)
            .then_with(|| b.apparent_bytes.cmp(&a.apparent_bytes))
            .then_with(|| a.extension.cmp(&b.extension))
    });
    stats
}

/// Prepares a bounded list of [`TreemapTile`]s for `dir_id`, grouping tail items that are smaller
/// than `min_fraction` (e.g. `0.005` = 0.5%) or beyond `max_visible_tiles` into a single synthetic
/// `"Altri N elementi"` bucket so that directories with thousands of files remain fast and legible.
pub fn build_treemap_view(
    scan: &ScanResult,
    dir_id: EntryId,
    metric: SizeMetric,
    max_visible_tiles: usize,
    min_fraction: f64,
) -> Vec<TreemapTile> {
    let sort_field = match metric {
        SizeMetric::Allocated => SortField::AllocatedBytes,
        SizeMetric::Apparent => SortField::ApparentBytes,
    };
    let children = children_sorted(scan, dir_id, sort_field, true);
    if children.is_empty() {
        return Vec::new();
    }

    let total_bytes: u64 = children.iter().map(|c| c.effective_bytes(metric)).sum();
    let max_tiles = max_visible_tiles.max(1);

    let mut tiles: Vec<TreemapTile> = Vec::new();
    let mut tail_ids: Vec<EntryId> = Vec::new();
    let mut tail_alloc: u64 = 0;
    let mut tail_app: u64 = 0;
    let mut tail_errors: u64 = 0;

    for (idx, child) in children.iter().enumerate() {
        let child_eff = child.effective_bytes(metric);
        let fraction = if total_bytes > 0 {
            (child_eff as f64) / (total_bytes as f64)
        } else {
            0.0
        };

        // Always keep entries with access errors visible so they are never hidden in a tail bucket!
        let has_error = child.access_status.is_error() || child.subtree_error_count > 0;
        let should_bucket = !has_error
            && (idx >= max_tiles || (idx > 0 && fraction < min_fraction && children.len() > 8));

        if should_bucket {
            tail_ids.push(child.id);
            tail_alloc = tail_alloc.saturating_add(child.effective_bytes(SizeMetric::Allocated));
            tail_app = tail_app.saturating_add(child.effective_bytes(SizeMetric::Apparent));
            tail_errors = tail_errors.saturating_add(child.subtree_error_count);
        } else {
            let children_preview = if child.entry_type == EntryType::Directory && child_eff > 0 {
                let sub_children = children_sorted(scan, child.id, sort_field, true);
                sub_children
                    .into_iter()
                    .take(6)
                    .filter_map(|sc| {
                        let sc_eff = sc.effective_bytes(metric);
                        if sc_eff == 0 {
                            return None;
                        }
                        Some(TreemapSubTile {
                            entry_id: sc.id,
                            label: sc.name_display.clone(),
                            entry_type: sc.entry_type,
                            allocated_bytes: sc.effective_bytes(SizeMetric::Allocated),
                            apparent_bytes: sc.effective_bytes(SizeMetric::Apparent),
                            fraction_of_dir: (sc_eff as f64) / (child_eff as f64),
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };

            tiles.push(TreemapTile {
                entry_id: Some(child.id),
                label: child.name_display.clone(),
                rel_path_display: child.rel_path_display.clone(),
                entry_type: child.entry_type,
                is_tail_bucket: false,
                bucketed_items_count: 1,
                bucketed_entry_ids: vec![child.id],
                allocated_bytes: child.effective_bytes(SizeMetric::Allocated),
                apparent_bytes: child.effective_bytes(SizeMetric::Apparent),
                fraction_of_parent: fraction,
                access_status: child.access_status,
                subtree_error_count: child.subtree_error_count,
                children_preview,
            });
        }
    }

    if !tail_ids.is_empty() {
        let tail_eff = match metric {
            SizeMetric::Allocated => tail_alloc,
            SizeMetric::Apparent => tail_app,
        };
        let fraction = if total_bytes > 0 {
            (tail_eff as f64) / (total_bytes as f64)
        } else {
            0.0
        };
        let count = tail_ids.len();
        tiles.push(TreemapTile {
            entry_id: None,
            label: format!("Altri {} elementi minori", count),
            rel_path_display: String::new(),
            entry_type: EntryType::Other,
            is_tail_bucket: true,
            bucketed_items_count: count,
            bucketed_entry_ids: tail_ids,
            allocated_bytes: tail_alloc,
            apparent_bytes: tail_app,
            fraction_of_parent: fraction,
            access_status: AccessStatus::Ok,
            subtree_error_count: tail_errors,
            children_preview: Vec::new(),
        });
    }

    tiles
}

/// Generates the [`RecoverReport`] for `Recover` mode, identifying top files, top directories,
/// and recognizable cache/temporary/build-artifact locations without ever classifying personal
/// files or unknown content as disposable.
pub fn build_recover_report(
    scan: &ScanResult,
    top_limit: usize,
    metric: SizeMetric,
) -> RecoverReport {
    let largest_files: Vec<EntrySummary> = top_files(scan, top_limit, metric, false)
        .into_iter()
        .map(EntrySummary::from_node)
        .collect();
    let largest_directories: Vec<EntrySummary> = top_directories(scan, top_limit, metric)
        .into_iter()
        .map(EntrySummary::from_node)
        .collect();

    let mut cache_and_temp_candidates: Vec<RecoverCandidate> = Vec::new();
    let mut covered_roots: Vec<Vec<u8>> = Vec::new();

    let mut indices: Vec<usize> = (1..scan.entries.len()).collect();
    indices.sort_by(|&a, &b| {
        scan.entries[a]
            .depth
            .cmp(&scan.entries[b].depth)
            .then_with(|| {
                scan.entries[b]
                    .effective_bytes(SizeMetric::Allocated)
                    .cmp(&scan.entries[a].effective_bytes(SizeMetric::Allocated))
            })
    });

    for idx in indices {
        let node = &scan.entries[idx];
        if covered_roots
            .iter()
            .any(|root_raw| is_strict_descendant_raw(&node.rel_path_raw, root_raw))
        {
            continue;
        }
        if let Some((code, label, reason)) = classify_recoverable_pattern(node) {
            let qual = compute_candidate_qualification(scan, node, code, label, reason);
            if qual.detected_allocated_bytes > 0 || qual.detected_apparent_bytes > 0 {
                if node.entry_type == EntryType::Directory {
                    covered_roots.push(node.rel_path_raw.clone());
                }
                cache_and_temp_candidates.push(RecoverCandidate {
                    entry: EntrySummary::from_node(node),
                    abs_path_display: resolve_abs_path_display(scan, node),
                    qualification: qual,
                });
            }
        }
    }

    cache_and_temp_candidates.sort_by(|a, b| {
        b.qualification
            .potentially_recoverable_bytes
            .cmp(&a.qualification.potentially_recoverable_bytes)
            .then_with(|| {
                b.qualification
                    .detected_allocated_bytes
                    .cmp(&a.qualification.detected_allocated_bytes)
            })
    });

    let total_potentially_recoverable_bytes = cache_and_temp_candidates
        .iter()
        .map(|c| c.qualification.potentially_recoverable_bytes)
        .sum();
    let total_verified_recoverable_bytes = cache_and_temp_candidates
        .iter()
        .map(|c| c.qualification.verified_recoverable_bytes)
        .sum();

    RecoverReport {
        total_detected_allocated_bytes: scan.total_allocated_bytes,
        total_potentially_recoverable_bytes,
        total_verified_recoverable_bytes,
        cache_and_temp_candidates,
        largest_files,
        largest_directories,
    }
}

/// Computes the 3-tier [`SpaceQualification`] for any single [`EntryNode`].
pub fn qualify_entry_space(scan: &ScanResult, node: &EntryNode) -> SpaceQualification {
    if let Some((code, label, reason)) = classify_recoverable_pattern(node) {
        return compute_candidate_qualification(scan, node, code, label, reason);
    }

    let mut cursor = node.parent_id;
    while let Some(pid) = cursor {
        let Some(pnode) = scan.get(pid) else {
            break;
        };
        if let Some((code, label, reason)) = classify_recoverable_pattern(pnode) {
            return compute_candidate_qualification(scan, node, code, label, reason);
        }
        cursor = pnode.parent_id;
    }

    let det_alloc = node.effective_bytes(SizeMetric::Allocated);
    let det_app = node.effective_bytes(SizeMetric::Apparent);
    SpaceQualification {
        detected_allocated_bytes: det_alloc,
        detected_apparent_bytes: det_app,
        potentially_recoverable_bytes: 0,
        verified_recoverable_bytes: 0,
        category_code: "user_or_project_data".to_string(),
        category_label: "Dati utente / progetto (non classificato come cache)".to_string(),
        explanation: "Contenuto ordinario: DiskGarden non classifica file personali o di progetto come inutili.".to_string(),
    }
}

fn compute_candidate_qualification(
    scan: &ScanResult,
    node: &EntryNode,
    code: &str,
    label: &str,
    reason: &str,
) -> SpaceQualification {
    let det_alloc = node.effective_bytes(SizeMetric::Allocated);
    let det_app = node.effective_bytes(SizeMetric::Apparent);
    let pot_bytes = det_alloc.max(det_app);

    if node.access_status.is_error() || node.subtree_error_count > 0 {
        return SpaceQualification {
            detected_allocated_bytes: det_alloc,
            detected_apparent_bytes: det_app,
            potentially_recoverable_bytes: pot_bytes,
            verified_recoverable_bytes: 0,
            category_code: code.to_string(),
            category_label: label.to_string(),
            explanation: format!(
                "{} — Spazio verificato pari a 0 B perché il sottoalbero contiene {} errori di accesso.",
                reason,
                node.subtree_error_count.max(1)
            ),
        };
    }

    let verified = match node.entry_type {
        EntryType::File => {
            if node.has_external_hardlinks() {
                0
            } else {
                node.allocated_bytes
            }
        }
        EntryType::Directory => compute_verified_subtree_recoverable(scan, node),
        _ => 0,
    };

    let explanation = if verified < det_alloc {
        format!(
            "{} — Parte dei blocchi ({} B) appartiene a file con hard link esterni alla cartella.",
            reason,
            det_alloc.saturating_sub(verified)
        )
    } else {
        format!(
            "{} — Nessun hard link esterno rilevato; spazio allocato interamente verificato.",
            reason
        )
    };

    SpaceQualification {
        detected_allocated_bytes: det_alloc,
        detected_apparent_bytes: det_app,
        potentially_recoverable_bytes: pot_bytes,
        verified_recoverable_bytes: verified,
        category_code: code.to_string(),
        category_label: label.to_string(),
        explanation,
    }
}

fn compute_verified_subtree_recoverable(scan: &ScanResult, dir_node: &EntryNode) -> u64 {
    let prefix = dir_node.rel_path_raw.as_slice();
    let mut inode_links_in_subtree: HashMap<(u64, u64), (u64, u64, u64)> = HashMap::new();
    let mut unshared_alloc: u64 = 0;

    for entry in &scan.entries {
        if entry.entry_type != EntryType::File {
            continue;
        }
        let inside = if dir_node.depth == 0 {
            true
        } else {
            is_strict_descendant_raw(&entry.rel_path_raw, prefix)
        };
        if !inside {
            continue;
        }
        if entry.nlink <= 1 {
            unshared_alloc = unshared_alloc.saturating_add(entry.allocated_bytes);
        } else {
            let rec = inode_links_in_subtree
                .entry((entry.dev, entry.ino))
                .or_insert((0, entry.nlink, entry.allocated_bytes));
            rec.0 += 1;
        }
    }

    for (seen_in_dir, total_nlink, alloc_bytes) in inode_links_in_subtree.values() {
        if *seen_in_dir >= *total_nlink {
            unshared_alloc = unshared_alloc.saturating_add(*alloc_bytes);
        }
    }
    unshared_alloc
}

fn classify_recoverable_pattern(
    node: &EntryNode,
) -> Option<(&'static str, &'static str, &'static str)> {
    let name = node.name_display.as_str();
    let rel = node.rel_path_display.as_str();

    match node.entry_type {
        EntryType::Directory => {
            if name == ".cache" || name == "_cacache" || rel.ends_with("/.cache") {
                Some((
                    "xdg_cache",
                    "Cache Applicativa / Utente (.cache)",
                    "Directory di cache standard rigenerabile dalle applicazioni",
                ))
            } else if name == "__pycache__"
                || name == ".pytest_cache"
                || name == ".mypy_cache"
                || name == ".ruff_cache"
            {
                Some((
                    "python_cache",
                    "Cache Bytecode / Test Python",
                    "Artefatti di cache Python rigenerabili automaticamente",
                ))
            } else if (name == "debug" || name == "release")
                && (rel.starts_with("target/") || rel.contains("/target/"))
            {
                Some((
                    "cargo_build_cache",
                    "Artefatti di Compilazione (target/)",
                    "Output di compilazione intermedia rigenerabile con cargo build",
                ))
            } else if name == ".next" || name == ".nuxt" || name == ".svelte-kit" || name == ".turbo"
            {
                Some((
                    "web_build_cache",
                    "Cache Build Frontend",
                    "Directory di build/cache frontend rigenerabile",
                ))
            } else if name == "tmp" || name == "temp" || name == ".tmp" {
                Some((
                    "temp_dir",
                    "Directory Temporanea",
                    "Contenitore di file temporanei da verificare prima della rimozione",
                ))
            } else {
                None
            }
        }
        EntryType::File => {
            let lower = name.to_lowercase();
            if lower.ends_with(".tmp")
                || lower.ends_with(".temp")
                || lower.ends_with(".bak")
                || lower.ends_with(".old")
                || lower.ends_with(".swp")
            {
                Some((
                    "temp_file",
                    "File Temporaneo / Backup (.tmp/.bak/.old)",
                    "File temporaneo o di salvataggio intermedio",
                ))
            } else if lower.ends_with(".log")
                || lower.contains(".log.")
                || lower.starts_with("core.")
            {
                Some((
                    "log_or_dump",
                    "File di Log / Core Dump",
                    "Traccia diagnostica o crash dump da esaminare",
                ))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn is_strict_descendant_raw(path_raw: &[u8], ancestor_raw: &[u8]) -> bool {
    if ancestor_raw.is_empty() {
        return !path_raw.is_empty();
    }
    path_raw.len() > ancestor_raw.len()
        && path_raw.starts_with(ancestor_raw)
        && path_raw[ancestor_raw.len()] == b'/'
}

fn resolve_abs_path_display(scan: &ScanResult, node: &EntryNode) -> String {
    if node.depth == 0 || node.rel_path_raw.is_empty() {
        scan.root_path_display.clone()
    } else {
        let root_pb = raw_bytes_to_path_buf(&scan.root_path_raw);
        let rel_pb = raw_bytes_to_path_buf(&node.rel_path_raw);
        root_pb.join(rel_pb).to_string_lossy().into_owned()
    }
}

/// Sorts a slice of [`EntryNode`] references in-place with deterministic tie-breaking by `rel_path_raw`.
pub fn sort_entry_refs(entries: &mut [&EntryNode], sort_field: SortField, descending: bool) {
    entries.sort_by(|a, b| {
        let primary = match sort_field {
            SortField::AllocatedBytes => a
                .effective_bytes(SizeMetric::Allocated)
                .cmp(&b.effective_bytes(SizeMetric::Allocated))
                .then_with(|| {
                    a.effective_bytes(SizeMetric::Apparent)
                        .cmp(&b.effective_bytes(SizeMetric::Apparent))
                }),
            SortField::ApparentBytes => a
                .effective_bytes(SizeMetric::Apparent)
                .cmp(&b.effective_bytes(SizeMetric::Apparent))
                .then_with(|| {
                    a.effective_bytes(SizeMetric::Allocated)
                        .cmp(&b.effective_bytes(SizeMetric::Allocated))
                }),
            SortField::Name => a
                .name_display
                .to_lowercase()
                .cmp(&b.name_display.to_lowercase()),
            SortField::FilesCount => a.subtree_files_count.cmp(&b.subtree_files_count),
            SortField::ModifiedTime => a
                .mtime_sec
                .unwrap_or(i64::MIN)
                .cmp(&b.mtime_sec.unwrap_or(i64::MIN))
                .then_with(|| a.mtime_nsec.unwrap_or(0).cmp(&b.mtime_nsec.unwrap_or(0))),
        };

        let ordered = if descending {
            primary.reverse()
        } else {
            primary
        };

        if ordered == Ordering::Equal {
            a.rel_path_raw.cmp(&b.rel_path_raw)
        } else {
            ordered
        }
    });
}

fn extract_extension(name: &str) -> String {
    let trimmed = name.trim_start_matches('.');
    match trimmed.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() && ext.len() <= 16 => ext.to_lowercase(),
        _ => "(nessuna)".to_string(),
    }
}
