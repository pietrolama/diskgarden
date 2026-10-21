use disk_garden_core::{
    build_directory_navigation_view, build_entry_detail_view, build_recover_report, compare_scans,
    extension_distribution, filter_entries, list_system_volumes, raw_bytes_to_path_buf,
    top_directories, top_files, CancelToken, DiffOptions, DiffSortField, DiskVolumeInfo,
    EntryFilter, EntryId, EntrySummary, EntryType, ScanConfig, ScanEvent, ScanHandle, ScanProgress,
    ScanResult, ScanStatus, SizeMetric, SnapshotStore, SortField,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const UI_INDEX_HTML: &str = include_str!("../ui/index.html");

/// Summary of a completed, partial, cancelled, or failed scan exposed to the UI header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanOverview {
    pub root_path_display: String,
    pub status: ScanStatus,
    pub started_at_unix_ms: i64,
    pub duration_ms: u64,
    pub root_id: Option<EntryId>,
    pub total_apparent_bytes: u64,
    pub total_allocated_bytes: u64,
    pub total_files_count: u64,
    pub total_dirs_count: u64,
    pub total_symlinks_count: u64,
    pub hardlink_duplicate_files_count: u64,
    pub hardlink_saved_allocated_bytes: u64,
    pub unreadable_dirs_count: u64,
    pub permission_denied_count: u64,
    pub io_errors_count: u64,
    pub vanished_count: u64,
    pub excluded_mounts_count: u64,
    pub root_error_message: Option<String>,
}

impl ScanOverview {
    pub fn from_result(res: &ScanResult) -> Self {
        Self {
            root_path_display: res.root_path_display.clone(),
            status: res.status,
            started_at_unix_ms: res.started_at_unix_ms,
            duration_ms: res.duration_ms,
            root_id: res.root_id,
            total_apparent_bytes: res.total_apparent_bytes,
            total_allocated_bytes: res.total_allocated_bytes,
            total_files_count: res.total_files_count,
            total_dirs_count: res.total_dirs_count,
            total_symlinks_count: res.total_symlinks_count,
            hardlink_duplicate_files_count: res.hardlink_duplicate_files_count,
            hardlink_saved_allocated_bytes: res.hardlink_saved_allocated_bytes,
            unreadable_dirs_count: res.unreadable_dirs_count,
            permission_denied_count: res.permission_denied_count,
            io_errors_count: res.io_errors_count,
            vanished_count: res.vanished_count,
            excluded_mounts_count: res.excluded_mounts_count,
            root_error_message: res.root_error_message.clone(),
        }
    }
}

/// Top-level application state response polled by the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStateResponse {
    pub is_scanning: bool,
    pub active_scan_root: Option<String>,
    pub progress: Option<ScanProgress>,
    pub scan_overview: Option<ScanOverview>,
    pub default_cwd: String,
    pub volumes: Vec<DiskVolumeInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartScanRequest {
    pub root_path: String,
    #[serde(default)]
    pub follow_symlinks: bool,
    #[serde(default)]
    pub cross_mounts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub entry_type: Option<EntryType>,
    #[serde(default)]
    pub min_bytes: Option<u64>,
    #[serde(default)]
    pub metric: Option<SizeMetric>,
    #[serde(default)]
    pub only_errors: bool,
    #[serde(default)]
    pub only_sparse: bool,
    #[serde(default)]
    pub only_hardlinks: bool,
    #[serde(default)]
    pub ancestor_id: Option<EntryId>,
    #[serde(default)]
    pub top_files_only: bool,
    #[serde(default)]
    pub top_dirs_only: bool,
    pub sort_field: Option<SortField>,
    pub descending: Option<bool>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub total_matched: usize,
    pub items: Vec<EntrySummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketItemsRequest {
    pub entry_ids: Vec<EntryId>,
    pub sort_field: Option<SortField>,
    pub descending: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsDirListing {
    pub current_path: String,
    pub parent_path: Option<String>,
    pub directories: Vec<FsDirEntry>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsDirEntry {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveSnapshotRequest {
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteSnapshotRequest {
    pub snapshot_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadSnapshotRequest {
    pub snapshot_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryDiffRequest {
    pub base_snapshot_id: i64,
    #[serde(default)]
    pub target_snapshot_id: Option<i64>,
    #[serde(default)]
    pub sort_field: Option<DiffSortField>,
    #[serde(default)]
    pub descending: Option<bool>,
    #[serde(default)]
    pub metric: Option<SizeMetric>,
    #[serde(default)]
    pub limit: Option<usize>,
}

struct ActiveJob {
    job_id: u64,
    root_path: String,
    cancel_token: CancelToken,
    latest_progress: Option<ScanProgress>,
}

struct InnerState {
    next_job_id: u64,
    active_job: Option<ActiveJob>,
    current_scan: Option<Arc<ScanResult>>,
    default_cwd: String,
    snapshot_store: Option<SnapshotStore>,
}

/// Thread-safe application state coordinator bridging [`disk_garden_core`] and the UI.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Mutex<InnerState>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        let store = SnapshotStore::open_default().ok();
        Self {
            inner: Arc::new(Mutex::new(InnerState {
                next_job_id: 1,
                active_job: None,
                current_scan: None,
                default_cwd: "/".to_string(),
                snapshot_store: store,
            })),
        }
    }

    /// Creates an [`AppState`] backed by an explicit [`SnapshotStore`] (e.g. for isolated tests).
    pub fn with_snapshot_store(store: SnapshotStore) -> Self {
        Self {
            inner: Arc::new(Mutex::new(InnerState {
                next_job_id: 1,
                active_job: None,
                current_scan: None,
                default_cwd: "/".to_string(),
                snapshot_store: Some(store),
            })),
        }
    }

    pub fn snapshot_store(&self) -> Option<SnapshotStore> {
        self.inner.lock().unwrap().snapshot_store.clone()
    }

    pub fn set_current_scan(&self, scan: ScanResult) {
        let mut guard = self.inner.lock().unwrap();
        guard.current_scan = Some(Arc::new(scan));
    }

    /// Starts a new background scan on `config`, cancelling any previous scan in flight.
    pub fn start_scan(&self, config: ScanConfig) -> u64 {
        let mut guard = self.inner.lock().unwrap();
        if let Some(prev) = guard.active_job.take() {
            prev.cancel_token.cancel();
        }

        let job_id = guard.next_job_id;
        guard.next_job_id += 1;

        let root_str = config.root_path.to_string_lossy().into_owned();
        let handle = ScanHandle::spawn(config);
        let cancel_token = handle.cancel_token();

        guard.active_job = Some(ActiveJob {
            job_id,
            root_path: root_str,
            cancel_token,
            latest_progress: Some(ScanProgress {
                dirs_scanned: 0,
                files_scanned: 0,
                symlinks_scanned: 0,
                apparent_bytes_so_far: 0,
                allocated_bytes_so_far: 0,
                errors_count: 0,
                current_rel_path: String::new(),
                elapsed_ms: 0,
            }),
        });
        drop(guard);

        let state_clone = self.clone();
        thread::spawn(move || {
            while let Ok(event) = handle.receiver().recv() {
                match event {
                    ScanEvent::Progress(prog) => {
                        let mut g = state_clone.inner.lock().unwrap();
                        if let Some(ref mut job) = g.active_job {
                            if job.job_id == job_id {
                                job.latest_progress = Some(prog);
                            }
                        }
                    }
                    ScanEvent::Finished(result) => {
                        let mut g = state_clone.inner.lock().unwrap();
                        let still_current = g
                            .active_job
                            .as_ref()
                            .map(|j| j.job_id == job_id)
                            .unwrap_or(false);
                        if still_current {
                            g.active_job = None;
                            g.current_scan = Some(Arc::new(*result));
                        }
                        break;
                    }
                }
            }
        });

        job_id
    }

    /// Cancels the currently running scan, if any.
    pub fn cancel_scan(&self) -> bool {
        let guard = self.inner.lock().unwrap();
        if let Some(ref job) = guard.active_job {
            job.cancel_token.cancel();
            true
        } else {
            false
        }
    }

    /// Returns a snapshot of the current [`AppStateResponse`].
    pub fn state_snapshot(&self) -> AppStateResponse {
        let guard = self.inner.lock().unwrap();
        AppStateResponse {
            is_scanning: guard.active_job.is_some(),
            active_scan_root: guard.active_job.as_ref().map(|j| j.root_path.clone()),
            progress: guard
                .active_job
                .as_ref()
                .and_then(|j| j.latest_progress.clone()),
            scan_overview: guard
                .current_scan
                .as_ref()
                .map(|s| ScanOverview::from_result(s)),
            default_cwd: guard.default_cwd.clone(),
            volumes: list_system_volumes(),
        }
    }

    /// Clones the `Arc<ScanResult>` of the latest completed/partial/cancelled scan.
    pub fn current_scan(&self) -> Option<Arc<ScanResult>> {
        self.inner.lock().unwrap().current_scan.clone()
    }
}

/// Embedded local HTTP server for the DiskGarden desktop interface.
pub struct AppServer {
    addr: SocketAddr,
    state: AppState,
}

impl AppServer {
    /// Binds to `127.0.0.1:port` (`0` for an ephemeral OS-assigned port) and spawns request workers.
    pub fn start(port: u16, state: AppState) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let addr = listener.local_addr()?;
        let state_clone = state.clone();

        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let st = state_clone.clone();
                thread::spawn(move || {
                    let _ = handle_connection(stream, &st);
                });
            }
        });

        Ok(Self { addr, state })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.addr.port())
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }
}

fn handle_connection(mut stream: TcpStream, state: &AppState) -> std::io::Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream.try_clone()?);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }
    let method = parts[0];
    let raw_target = parts[1];

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    let (path, query_str) = match raw_target.split_once('?') {
        Some((p, q)) => (p, q),
        None => (raw_target, ""),
    };
    let query = parse_query(query_str);

    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => {
            write_response(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                UI_INDEX_HTML.as_bytes(),
            )
        }
        ("GET", "/api/state") => {
            let snap = state.state_snapshot();
            write_json(&mut stream, 200, &snap)
        }
        ("GET", "/api/disks") => {
            let vols = list_system_volumes();
            write_json(&mut stream, 200, &vols)
        }
        ("POST", "/api/scan/start") => {
            let req: StartScanRequest = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    return write_json(
                        &mut stream,
                        400,
                        &serde_json::json!({ "error": format!("Richiesta non valida: {}", e) }),
                    )
                }
            };
            let trimmed = req.root_path.trim();
            if trimmed.is_empty() {
                return write_json(
                    &mut stream,
                    400,
                    &serde_json::json!({ "error": "Il percorso della directory non può essere vuoto." }),
                );
            }
            let mut cfg = ScanConfig::new(PathBuf::from(trimmed));
            cfg.follow_symlinks = req.follow_symlinks;
            cfg.cross_mounts = req.cross_mounts;
            cfg.progress_interval_ms = 50;
            let job_id = state.start_scan(cfg);
            write_json(
                &mut stream,
                200,
                &serde_json::json!({ "started": true, "job_id": job_id }),
            )
        }
        ("POST", "/api/scan/cancel") => {
            let cancelled = state.cancel_scan();
            write_json(&mut stream, 200, &serde_json::json!({ "cancelled": cancelled }))
        }
        ("GET", "/api/explore/dir") => {
            let Some(scan) = state.current_scan() else {
                return write_json(
                    &mut stream,
                    404,
                    &serde_json::json!({ "error": "Nessuna scansione disponibile." }),
                );
            };
            let target_id = query
                .get("id")
                .and_then(|s| s.parse::<usize>().ok())
                .map(EntryId)
                .or(scan.root_id)
                .unwrap_or(EntryId(0));
            let sort_field = parse_sort_field(query.get("sort").map(|s| s.as_str()));
            let descending = query
                .get("desc")
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true);
            let metric = parse_metric(query.get("metric").map(|s| s.as_str()));
            let max_tiles = query
                .get("max_tiles")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(48);
            let max_children = query
                .get("limit")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(500);

            match build_directory_navigation_view(
                &scan,
                target_id,
                sort_field,
                descending,
                metric,
                max_tiles,
                max_children,
            ) {
                Some(view) => write_json(&mut stream, 200, &view),
                None => write_json(
                    &mut stream,
                    404,
                    &serde_json::json!({ "error": "Directory non trovata nella scansione." }),
                ),
            }
        }
        ("POST", "/api/explore/bucket") => {
            let Some(scan) = state.current_scan() else {
                return write_json(
                    &mut stream,
                    404,
                    &serde_json::json!({ "error": "Nessuna scansione disponibile." }),
                );
            };
            let req: BucketItemsRequest = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    return write_json(
                        &mut stream,
                        400,
                        &serde_json::json!({ "error": e.to_string() }),
                    )
                }
            };
            let sort_field = req.sort_field.unwrap_or(SortField::AllocatedBytes);
            let descending = req.descending.unwrap_or(true);
            let mut refs: Vec<_> = req
                .entry_ids
                .iter()
                .filter_map(|&id| scan.get(id))
                .collect();
            disk_garden_core::sort_entry_refs(&mut refs, sort_field, descending);
            let items: Vec<EntrySummary> = refs
                .into_iter()
                .take(500)
                .map(EntrySummary::from_node)
                .collect();
            write_json(&mut stream, 200, &serde_json::json!({ "items": items }))
        }
        ("GET", "/api/entry") => {
            let Some(scan) = state.current_scan() else {
                return write_json(
                    &mut stream,
                    404,
                    &serde_json::json!({ "error": "Nessuna scansione disponibile." }),
                );
            };
            let id = query
                .get("id")
                .and_then(|s| s.parse::<usize>().ok())
                .map(EntryId)
                .or(scan.root_id)
                .unwrap_or(EntryId(0));
            match build_entry_detail_view(&scan, id) {
                Some(detail) => write_json(&mut stream, 200, &detail),
                None => write_json(
                    &mut stream,
                    404,
                    &serde_json::json!({ "error": "Elemento non trovato." }),
                ),
            }
        }
        ("POST", "/api/search") => {
            let Some(scan) = state.current_scan() else {
                return write_json(
                    &mut stream,
                    404,
                    &serde_json::json!({ "error": "Nessuna scansione disponibile." }),
                );
            };
            let req: SearchRequest = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    return write_json(
                        &mut stream,
                        400,
                        &serde_json::json!({ "error": e.to_string() }),
                    )
                }
            };
            let limit = req.limit.unwrap_or(300).max(1);
            let metric = req.metric.unwrap_or(SizeMetric::Allocated);

            if req.top_files_only {
                let top = top_files(&scan, limit, metric, false);
                let total_matched = top.len();
                let items = top.into_iter().map(EntrySummary::from_node).collect();
                return write_json(&mut stream, 200, &SearchResponse { total_matched, items });
            }
            if req.top_dirs_only {
                let top = top_directories(&scan, limit, metric);
                let total_matched = top.len();
                let items = top.into_iter().map(EntrySummary::from_node).collect();
                return write_json(&mut stream, 200, &SearchResponse { total_matched, items });
            }

            let filter = EntryFilter {
                query: req.query,
                entry_type: req.entry_type,
                min_bytes: req.min_bytes,
                metric: Some(metric),
                only_errors: req.only_errors,
                only_sparse: req.only_sparse,
                only_hardlinks: req.only_hardlinks,
                ancestor_id: req.ancestor_id,
            };
            let sort_field = req.sort_field.unwrap_or(match metric {
                SizeMetric::Allocated => SortField::AllocatedBytes,
                SizeMetric::Apparent => SortField::ApparentBytes,
            });
            let descending = req.descending.unwrap_or(true);
            let matched = filter_entries(&scan, &filter, sort_field, descending);
            let total_matched = matched.len();
            let items: Vec<EntrySummary> = matched
                .into_iter()
                .take(limit)
                .map(EntrySummary::from_node)
                .collect();
            write_json(&mut stream, 200, &SearchResponse { total_matched, items })
        }
        ("GET", "/api/recover") => {
            let Some(scan) = state.current_scan() else {
                return write_json(
                    &mut stream,
                    404,
                    &serde_json::json!({ "error": "Nessuna scansione disponibile." }),
                );
            };
            let metric = parse_metric(query.get("metric").map(|s| s.as_str()));
            let limit = query
                .get("limit")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(25);
            let report = build_recover_report(&scan, limit, metric);
            write_json(&mut stream, 200, &report)
        }
        ("GET", "/api/extensions") => {
            let Some(scan) = state.current_scan() else {
                return write_json(
                    &mut stream,
                    404,
                    &serde_json::json!({ "error": "Nessuna scansione disponibile." }),
                );
            };
            let stats = extension_distribution(&scan);
            write_json(&mut stream, 200, &stats)
        }
        ("GET", "/api/fs/list-dirs") => {
            let requested = query
                .get("path")
                .cloned()
                .unwrap_or_else(|| state.state_snapshot().default_cwd);
            let listing = list_local_directories(Path::new(&requested));
            write_json(&mut stream, 200, &listing)
        }
        ("POST", "/api/fs/pick-native") => {
            let start_dir = state.state_snapshot().default_cwd;
            let picked = try_native_directory_picker(&start_dir);
            write_json(&mut stream, 200, &serde_json::json!({ "picked_path": picked }))
        }
        ("POST", "/api/fs/reveal") => {
            let Some(scan) = state.current_scan() else {
                return write_json(
                    &mut stream,
                    404,
                    &serde_json::json!({ "error": "Nessuna scansione disponibile." }),
                );
            };
            #[derive(Deserialize)]
            struct RevealReq {
                entry_id: EntryId,
            }
            let req: RevealReq = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    return write_json(
                        &mut stream,
                        400,
                        &serde_json::json!({ "error": e.to_string() }),
                    )
                }
            };
            let Some(node) = scan.get(req.entry_id) else {
                return write_json(
                    &mut stream,
                    404,
                    &serde_json::json!({ "error": "Elemento non trovato." }),
                );
            };
            let root_pb = raw_bytes_to_path_buf(&scan.root_path_raw);
            let target_pb = if node.depth == 0 {
                root_pb
            } else {
                root_pb.join(raw_bytes_to_path_buf(&node.rel_path_raw))
            };
            let dir_to_open = if node.entry_type == EntryType::Directory {
                target_pb
            } else {
                target_pb
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or(target_pb)
            };
            // Safe process invocation without any shell interpolation
            let ok = Command::new("xdg-open").arg(&dir_to_open).spawn().is_ok();
            write_json(
                &mut stream,
                200,
                &serde_json::json!({
                    "opened": ok,
                    "directory": dir_to_open.to_string_lossy()
                }),
            )
        }
        ("GET", "/api/snapshots") => {
            let Some(store) = state.snapshot_store() else {
                return write_json(
                    &mut stream,
                    500,
                    &serde_json::json!({ "error": "SnapshotStore SQLite non disponibile." }),
                );
            };
            let root_filter = query
                .get("root")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            match store.list_snapshots(root_filter.as_deref()) {
                Ok(list) => write_json(&mut stream, 200, &list),
                Err(e) => write_json(
                    &mut stream,
                    500,
                    &serde_json::json!({ "error": e.to_string() }),
                ),
            }
        }
        ("POST", "/api/snapshots/save") => {
            let Some(store) = state.snapshot_store() else {
                return write_json(
                    &mut stream,
                    500,
                    &serde_json::json!({ "error": "SnapshotStore SQLite non disponibile." }),
                );
            };
            let Some(scan) = state.current_scan() else {
                return write_json(
                    &mut stream,
                    404,
                    &serde_json::json!({ "error": "Nessuna scansione attiva da salvare come snapshot." }),
                );
            };
            let req: SaveSnapshotRequest = serde_json::from_slice(&body)
                .unwrap_or(SaveSnapshotRequest { label: None });
            match store.save_snapshot(
                &scan,
                req.label.as_deref(),
                scan.config.cross_mounts,
                scan.config.follow_symlinks,
            ) {
                Ok(header) => write_json(&mut stream, 200, &header),
                Err(e) => write_json(
                    &mut stream,
                    400,
                    &serde_json::json!({ "error": e.to_string() }),
                ),
            }
        }
        ("POST", "/api/snapshots/delete") => {
            let Some(store) = state.snapshot_store() else {
                return write_json(
                    &mut stream,
                    500,
                    &serde_json::json!({ "error": "SnapshotStore SQLite non disponibile." }),
                );
            };
            let req: DeleteSnapshotRequest = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    return write_json(
                        &mut stream,
                        400,
                        &serde_json::json!({ "error": e.to_string() }),
                    )
                }
            };
            match store.delete_snapshot(req.snapshot_id) {
                Ok(deleted) => write_json(
                    &mut stream,
                    200,
                    &serde_json::json!({ "deleted": deleted }),
                ),
                Err(e) => write_json(
                    &mut stream,
                    500,
                    &serde_json::json!({ "error": e.to_string() }),
                ),
            }
        }
        ("POST", "/api/snapshots/load") => {
            let Some(store) = state.snapshot_store() else {
                return write_json(
                    &mut stream,
                    500,
                    &serde_json::json!({ "error": "SnapshotStore SQLite non disponibile." }),
                );
            };
            let req: LoadSnapshotRequest = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    return write_json(
                        &mut stream,
                        400,
                        &serde_json::json!({ "error": e.to_string() }),
                    )
                }
            };
            match store.load_snapshot(req.snapshot_id) {
                Ok(stored) => {
                    let ov = ScanOverview::from_result(&stored.scan);
                    state.set_current_scan(stored.scan);
                    write_json(&mut stream, 200, &ov)
                }
                Err(e) => write_json(
                    &mut stream,
                    404,
                    &serde_json::json!({ "error": e.to_string() }),
                ),
            }
        }
        ("POST", "/api/history/diff") => {
            let Some(store) = state.snapshot_store() else {
                return write_json(
                    &mut stream,
                    500,
                    &serde_json::json!({ "error": "SnapshotStore SQLite non disponibile." }),
                );
            };
            let req: HistoryDiffRequest = match serde_json::from_slice(&body) {
                Ok(r) => r,
                Err(e) => {
                    return write_json(
                        &mut stream,
                        400,
                        &serde_json::json!({ "error": e.to_string() }),
                    )
                }
            };
            let base_stored = match store.load_snapshot(req.base_snapshot_id) {
                Ok(s) => s,
                Err(e) => {
                    return write_json(
                        &mut stream,
                        404,
                        &serde_json::json!({ "error": format!("Snapshot base non trovato: {}", e) }),
                    )
                }
            };

            let (target_scan, target_id, target_label): (Arc<ScanResult>, Option<i64>, Option<String>) =
                if let Some(tid) = req.target_snapshot_id {
                    match store.load_snapshot(tid) {
                        Ok(s) => (Arc::new(s.scan), Some(s.header.id), s.header.label),
                        Err(e) => {
                            return write_json(
                                &mut stream,
                                404,
                                &serde_json::json!({ "error": format!("Snapshot target non trovato: {}", e) }),
                            )
                        }
                    }
                } else if let Some(cur) = state.current_scan() {
                    (
                        cur,
                        None,
                        Some("Scansione Corrente (Live)".to_string()),
                    )
                } else {
                    return write_json(
                        &mut stream,
                        400,
                        &serde_json::json!({ "error": "Seleziona uno snapshot target oppure esegui prima una scansione corrente." }),
                    );
                };

            let opts = DiffOptions {
                sort_field: req.sort_field.unwrap_or(DiffSortField::SignedDelta),
                descending: req.descending.unwrap_or(true),
                metric: req.metric.unwrap_or(SizeMetric::Allocated),
                max_items_per_category: req.limit.unwrap_or(300),
                base_snapshot_id: Some(base_stored.header.id),
                target_snapshot_id: target_id,
                base_label: base_stored.header.label,
                target_label,
            };

            match compare_scans(&base_stored.scan, &target_scan, &opts) {
                Ok(report) => write_json(&mut stream, 200, &report),
                Err(e) => write_json(
                    &mut stream,
                    400,
                    &serde_json::json!({ "error": e.to_string() }),
                ),
            }
        }
        _ => write_json(
            &mut stream,
            404,
            &serde_json::json!({ "error": "Endpoint non trovato" }),
        ),
    }
}

fn list_local_directories(path: &Path) -> FsDirListing {
    let target = if path.exists() && path.is_dir() {
        path.to_path_buf()
    } else if let Some(parent) = path.parent() {
        if parent.exists() && parent.is_dir() {
            parent.to_path_buf()
        } else {
            PathBuf::from("/")
        }
    } else {
        PathBuf::from("/")
    };

    let canonical = fs::canonicalize(&target).unwrap_or(target);
    let parent_path = canonical
        .parent()
        .map(|p| p.to_string_lossy().into_owned());

    match fs::read_dir(&canonical) {
        Ok(rd) => {
            let mut dirs = Vec::new();
            for entry in rd.flatten() {
                if let Ok(ft) = entry.file_type() {
                    if ft.is_dir() {
                        let p = entry.path();
                        let name = entry.file_name().to_string_lossy().into_owned();
                        dirs.push(FsDirEntry {
                            name,
                            path: p.to_string_lossy().into_owned(),
                        });
                    }
                }
            }
            dirs.sort_by_key(|a| a.name.to_lowercase());
            FsDirListing {
                current_path: canonical.to_string_lossy().into_owned(),
                parent_path,
                directories: dirs,
                error: None,
            }
        }
        Err(err) => FsDirListing {
            current_path: canonical.to_string_lossy().into_owned(),
            parent_path,
            directories: Vec::new(),
            error: Some(err.to_string()),
        },
    }
}

fn try_native_directory_picker(start_dir: &str) -> Option<String> {
    // 1. Try kdialog (KDE / Qt) without shell
    if let Ok(out) = Command::new("kdialog")
        .arg("--getexistingdirectory")
        .arg(start_dir)
        .arg("--title")
        .arg("DiskGarden — Seleziona Directory da Analizzare")
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        return None;
    }

    // 2. Try zenity (GTK) without shell
    if let Ok(out) = Command::new("zenity")
        .arg("--file-selection")
        .arg("--directory")
        .arg("--title=DiskGarden — Seleziona Directory da Analizzare")
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

fn parse_sort_field(s: Option<&str>) -> SortField {
    match s {
        Some("apparent") | Some("apparent_bytes") => SortField::ApparentBytes,
        Some("name") => SortField::Name,
        Some("files") | Some("files_count") => SortField::FilesCount,
        Some("mtime") | Some("modified_time") => SortField::ModifiedTime,
        _ => SortField::AllocatedBytes,
    }
}

fn parse_metric(s: Option<&str>) -> SizeMetric {
    match s {
        Some("apparent") => SizeMetric::Apparent,
        _ => SizeMetric::Allocated,
    }
}

fn parse_query(q: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        map.insert(url_decode(k), url_decode(v));
    }
    map
}

fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(h), Some(l)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                    out.push((h << 4) | l);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn write_json<T: Serialize>(stream: &mut TcpStream, status: u16, payload: &T) -> std::io::Result<()> {
    let body = serde_json::to_vec(payload).unwrap_or_else(|_| b"{}".to_vec());
    write_response(stream, status, "application/json; charset=utf-8", &body)
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status,
        status_text,
        content_type,
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}
