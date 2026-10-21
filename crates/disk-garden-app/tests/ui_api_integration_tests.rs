use disk_garden_app::{
    AppServer, AppState, AppStateResponse, BucketItemsRequest, DeleteSnapshotRequest, FsDirListing,
    HistoryDiffRequest, LoadSnapshotRequest, SaveSnapshotRequest, SearchRequest, SearchResponse,
    StartScanRequest,
};
use disk_garden_core::{
    DiffReport, DiffSortField, DirectoryNavigationView, EntryDetailView, EntryType, RecoverReport,
    ScanStatus, SizeMetric, SnapshotHeader, SnapshotStore, SortField,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn test_http_hardening_rejects_missing_or_wrong_token_and_invalid_origins_and_hosts() {
    let state = AppState::new();
    let server = AppServer::start(0, state).expect("start local app server");
    let addr = server.addr();
    let port = addr.port();
    let token = server.session_token();
    assert_eq!(token.len(), 64, "expected 256-bit hex session token");

    // 1. Missing token on `/` and `/api/state` -> 401 Unauthorized
    let (status_line, _) = raw_http_request(
        addr,
        &format!(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            port
        ),
    );
    assert!(
        status_line.starts_with("HTTP/1.1 401"),
        "expected 401 without token on /, got: {}",
        status_line
    );

    let (status_line, _) = raw_http_request(
        addr,
        &format!(
            "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            port
        ),
    );
    assert!(
        status_line.starts_with("HTTP/1.1 401"),
        "expected 401 without token on /api/state, got: {}",
        status_line
    );

    // 2. Wrong token -> 401 Unauthorized
    let (status_line, _) = raw_http_request(
        addr,
        &format!(
            "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-DiskGarden-Token: 0000000000000000000000000000000000000000000000000000000000000000\r\nConnection: close\r\n\r\n",
            port
        ),
    );
    assert!(
        status_line.starts_with("HTTP/1.1 401"),
        "expected 401 with wrong token, got: {}",
        status_line
    );

    // 3. Valid token but foreign Origin (`Origin: https://evil.example`) -> 403 Forbidden
    let (status_line, _) = raw_http_request(
        addr,
        &format!(
            "POST /api/scan/cancel HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: https://evil.example\r\nX-DiskGarden-Token: {}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
            port, token
        ),
    );
    assert!(
        status_line.starts_with("HTTP/1.1 403"),
        "expected 403 with foreign Origin, got: {}",
        status_line
    );

    // 4. Valid token but foreign Referer (`Referer: http://evil.local/page`) -> 403 Forbidden
    let (status_line, _) = raw_http_request(
        addr,
        &format!(
            "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nReferer: http://evil.local/page\r\nX-DiskGarden-Token: {}\r\nConnection: close\r\n\r\n",
            port, token
        ),
    );
    assert!(
        status_line.starts_with("HTTP/1.1 403"),
        "expected 403 with foreign Referer, got: {}",
        status_line
    );

    // 5. Valid token but invalid Host header (DNS rebinding attempt `Host: attacker.com`) -> 403 Forbidden
    let (status_line, _) = raw_http_request(
        addr,
        &format!(
            "GET /api/state HTTP/1.1\r\nHost: attacker.com\r\nX-DiskGarden-Token: {}\r\nConnection: close\r\n\r\n",
            token
        ),
    );
    assert!(
        status_line.starts_with("HTTP/1.1 403"),
        "expected 403 with DNS-rebound Host header, got: {}",
        status_line
    );

    // 6. Valid token + valid local Origin + valid Host -> 200 OK
    let (status_line, body) = raw_http_request(
        addr,
        &format!(
            "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: http://127.0.0.1:{}\r\nX-DiskGarden-Token: {}\r\nConnection: close\r\n\r\n",
            port, port, token
        ),
    );
    assert!(
        status_line.starts_with("HTTP/1.1 200"),
        "expected 200 with valid token and origin, got: {}",
        status_line
    );
    assert!(body.contains("default_cwd"));
}

#[test]
fn test_milestone_2_progressive_navigation_treemap_bucketing_search_and_recover() {
    let tmp = TempDir::new().expect("create tempdir");
    let root = tmp.path().join("work_root");
    fs::create_dir_all(root.join("projects/app/src")).unwrap();
    fs::create_dir_all(root.join("projects/app/target/debug")).unwrap();
    fs::create_dir_all(root.join(".cache/pip")).unwrap();
    fs::create_dir_all(root.join("many_small_files")).unwrap();

    // Known files
    write_file(&root.join("projects/app/src/main.rs"), 12_000);
    write_file(&root.join("projects/app/target/debug/binary.bin"), 400_000);
    write_file(&root.join(".cache/pip/wheel_a.whl"), 90_000);

    // Create a file in .cache that is hard-linked outside `work_root`
    let shared_cache_file = root.join(".cache/pip/shared_ext.whl");
    write_file(&shared_cache_file, 128_000);
    let outside_link = tmp.path().join("external_hardlink.whl");
    fs::hard_link(&shared_cache_file, &outside_link).unwrap();

    // Populate `many_small_files` with 120 small files to verify Treemap tail bucketing
    for i in 0..120 {
        write_file(
            &root
                .join("many_small_files")
                .join(format!("small_{:03}.txt", i)),
            500 + (i % 10),
        );
    }

    let state = AppState::new();
    let server = AppServer::start(0, state).expect("start local app server");

    // 1. Verify UI HTML is served at `/?token=...` and injects session token
    let html = http_get_raw(&server, "/");
    assert!(html.contains("DiskGarden"));
    assert!(html.contains("Treemap Squarified"));
    assert!(html.contains(server.session_token()));

    // 2. Test directory browser API (`/api/fs/list-dirs`)
    let dir_listing: FsDirListing = http_get_json(
        &server,
        &format!("/api/fs/list-dirs?path={}", root.display()),
    );
    assert!(dir_listing.directories.iter().any(|d| d.name == "projects"));

    // 3. Start scan via `POST /api/scan/start`
    let start_res: serde_json::Value = http_post_json(
        &server,
        "/api/scan/start",
        &StartScanRequest {
            root_path: root.to_string_lossy().into_owned(),
            follow_symlinks: false,
            cross_mounts: false,
        },
    );
    assert_eq!(start_res["started"], true);

    // Poll `/api/state` until scan finishes
    let mut final_state: Option<AppStateResponse> = None;
    for _ in 0..100 {
        let st: AppStateResponse = http_get_json(&server, "/api/state");
        if !st.is_scanning && st.scan_overview.is_some() {
            final_state = Some(st);
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let st = final_state.expect("scan should finish promptly");
    assert_eq!(st.default_cwd, "/");
    assert!(
        st.volumes
            .iter()
            .any(|v| v.mount_point == "/" && v.total_bytes > 0),
        "expected root volume '/' in volumes list: {:?}",
        st.volumes
    );
    let ov = st.scan_overview.expect("scan overview present");
    assert_eq!(ov.status, ScanStatus::Completed);
    assert_eq!(ov.total_files_count, 124);

    // 4. Progressive navigation from Root -> `projects` -> `projects/app` -> `projects/app/src` -> `main.rs`
    let root_id = ov.root_id.unwrap();
    let root_view: DirectoryNavigationView = http_get_json(
        &server,
        &format!("/api/explore/dir?id={}&metric=apparent", root_id.0),
    );
    assert_eq!(root_view.breadcrumbs.len(), 1);
    assert!(root_view.treemap_tiles.len() <= 4);

    let projects_summary = root_view
        .children
        .iter()
        .find(|c| c.name_display == "projects")
        .expect("projects dir in root children");
    assert_eq!(projects_summary.subtree_apparent_bytes, 412_000);

    let projects_view: DirectoryNavigationView = http_get_json(
        &server,
        &format!(
            "/api/explore/dir?id={}&metric=apparent",
            projects_summary.id.0
        ),
    );
    assert_eq!(projects_view.breadcrumbs.len(), 2);
    assert_eq!(projects_view.breadcrumbs[1].name_display, "projects");

    let app_summary = &projects_view.children[0];
    assert_eq!(app_summary.name_display, "app");

    let app_view: DirectoryNavigationView = http_get_json(
        &server,
        &format!("/api/explore/dir?id={}&metric=apparent", app_summary.id.0),
    );
    assert_eq!(app_view.breadcrumbs.len(), 3);
    let src_summary = app_view
        .children
        .iter()
        .find(|c| c.name_display == "src")
        .unwrap();

    let src_view: DirectoryNavigationView = http_get_json(
        &server,
        &format!("/api/explore/dir?id={}&metric=apparent", src_summary.id.0),
    );
    assert_eq!(src_view.breadcrumbs.len(), 4);
    assert_eq!(src_view.children.len(), 1);
    let main_rs = &src_view.children[0];
    assert_eq!(main_rs.name_display, "main.rs");
    assert_eq!(main_rs.entry_type, EntryType::File);
    assert_eq!(main_rs.apparent_bytes, 12_000);

    // Inspect individual file via `/api/entry?id=...`
    let main_detail: EntryDetailView =
        http_get_json(&server, &format!("/api/entry?id={}", main_rs.id.0));
    assert_eq!(main_detail.summary.apparent_bytes, 12_000);
    assert!(main_detail
        .abs_path_display
        .ends_with("projects/app/src/main.rs"));

    // 5. Verify Treemap tail bucketing on `many_small_files` (120 files -> bounded to <= 21 tiles)
    let many_dir = root_view
        .children
        .iter()
        .find(|c| c.name_display == "many_small_files")
        .unwrap();
    let many_view: DirectoryNavigationView = http_get_json(
        &server,
        &format!(
            "/api/explore/dir?id={}&metric=apparent&max_tiles=20",
            many_dir.id.0
        ),
    );
    assert_eq!(many_view.total_children_count, 120);
    assert_eq!(many_view.treemap_tiles.len(), 21); // 20 individual + 1 tail bucket
    let tail_tile = many_view.treemap_tiles.last().unwrap();
    assert!(tail_tile.is_tail_bucket);
    assert_eq!(tail_tile.bucketed_items_count, 100);

    // Fetch bucketed items via `POST /api/explore/bucket`
    let bucket_res: serde_json::Value = http_post_json(
        &server,
        "/api/explore/bucket",
        &BucketItemsRequest {
            entry_ids: tail_tile.bucketed_entry_ids.clone(),
            sort_field: Some(SortField::Name),
            descending: Some(false),
        },
    );
    assert_eq!(bucket_res["items"].as_array().unwrap().len(), 100);

    // 6. Verify Search & Filter API (`POST /api/search`)
    let search_res: SearchResponse = http_post_json(
        &server,
        "/api/search",
        &SearchRequest {
            query: Some("binary.bin".to_string()),
            entry_type: None,
            min_bytes: None,
            metric: None,
            only_errors: false,
            only_sparse: false,
            only_hardlinks: false,
            ancestor_id: None,
            top_files_only: false,
            top_dirs_only: false,
            sort_field: Some(SortField::ApparentBytes),
            descending: Some(true),
            limit: Some(20),
        },
    );
    assert_eq!(search_res.total_matched, 1);
    assert_eq!(
        search_res.items[0].rel_path_display,
        "projects/app/target/debug/binary.bin"
    );

    // 7. Verify `Recover` mode 3-tier qualification (`GET /api/recover`)
    let recover: RecoverReport = http_get_json(&server, "/api/recover?metric=allocated&limit=10");
    assert!(recover.cache_and_temp_candidates.len() >= 2);

    let cargo_cand = recover
        .cache_and_temp_candidates
        .iter()
        .find(|c| c.entry.rel_path_display == "projects/app/target/debug")
        .expect("cargo target/debug candidate detected");
    assert!(cargo_cand.qualification.detected_allocated_bytes > 0);
    assert_eq!(
        cargo_cand.qualification.verified_recoverable_bytes,
        cargo_cand.qualification.detected_allocated_bytes
    );

    let cache_cand = recover
        .cache_and_temp_candidates
        .iter()
        .find(|c| c.entry.rel_path_display == ".cache")
        .expect(".cache candidate detected");
    assert!(
        cache_cand.qualification.verified_recoverable_bytes
            < cache_cand.qualification.detected_allocated_bytes,
        "expected verified ({}) < detected ({}) due to external hard link in .cache",
        cache_cand.qualification.verified_recoverable_bytes,
        cache_cand.qualification.detected_allocated_bytes
    );
    assert!(cache_cand.qualification.verified_recoverable_bytes > 0);
}

#[test]
fn test_milestone_3_history_snapshots_and_diff_api() {
    let tmp = TempDir::new().expect("create tempdir");
    let db_path = tmp.path().join("ui_snapshots.db");
    let store = SnapshotStore::open(&db_path).expect("open sqlite store");

    let root = tmp.path().join("monitored_root");
    fs::create_dir_all(root.join("logs")).unwrap();
    fs::create_dir_all(root.join("data")).unwrap();
    write_file(&root.join("logs/server.log"), 10_000);
    write_file(&root.join("data/old.db"), 30_000);

    let state = AppState::with_snapshot_store(store);
    let server = AppServer::start(0, state).expect("start local app server");

    // 1. Run baseline scan T0
    let _: serde_json::Value = http_post_json(
        &server,
        "/api/scan/start",
        &StartScanRequest {
            root_path: root.to_string_lossy().into_owned(),
            follow_symlinks: false,
            cross_mounts: false,
        },
    );
    for _ in 0..100 {
        let st: AppStateResponse = http_get_json(&server, "/api/state");
        if !st.is_scanning && st.scan_overview.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    // 2. Save baseline snapshot via `POST /api/snapshots/save`
    let snap_t0: SnapshotHeader = http_post_json(
        &server,
        "/api/snapshots/save",
        &SaveSnapshotRequest {
            label: Some("Baseline T0".to_string()),
        },
    );
    assert_eq!(snap_t0.label.as_deref(), Some("Baseline T0"));
    assert_eq!(snap_t0.total_apparent_bytes, 40_000);

    // 3. Mutate directory: grow `logs/server.log` (+50_000 B), remove `data/old.db` (-30_000 B), add `data/new.parquet` (+70_000 B)
    write_file(&root.join("logs/server.log"), 60_000);
    fs::remove_file(root.join("data/old.db")).unwrap();
    write_file(&root.join("data/new.parquet"), 70_000);

    // 4. Run scan T1
    let _: serde_json::Value = http_post_json(
        &server,
        "/api/scan/start",
        &StartScanRequest {
            root_path: root.to_string_lossy().into_owned(),
            follow_symlinks: false,
            cross_mounts: false,
        },
    );
    for _ in 0..100 {
        let st: AppStateResponse = http_get_json(&server, "/api/state");
        if !st.is_scanning
            && st
                .scan_overview
                .as_ref()
                .map(|o| o.total_apparent_bytes == 130_000)
                .unwrap_or(false)
        {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    // 5. Compare saved `snap_t0` against live current scan (`target_snapshot_id: None`)
    let diff: DiffReport = http_post_json(
        &server,
        "/api/history/diff",
        &HistoryDiffRequest {
            base_snapshot_id: snap_t0.id,
            target_snapshot_id: None,
            sort_field: Some(DiffSortField::SignedDelta),
            descending: Some(true),
            metric: Some(SizeMetric::Apparent),
            limit: Some(50),
        },
    );
    assert_eq!(diff.base_total_apparent_bytes, 40_000);
    assert_eq!(diff.target_total_apparent_bytes, 130_000);
    assert_eq!(diff.net_apparent_delta_bytes, 90_000);
    assert_eq!(diff.added_files.len(), 1);
    assert_eq!(diff.added_files[0].rel_path_display, "data/new.parquet");
    assert_eq!(diff.removed_files.len(), 1);
    assert_eq!(diff.removed_files[0].rel_path_display, "data/old.db");
    assert_eq!(diff.resized_files.len(), 1);
    assert_eq!(diff.resized_files[0].rel_path_display, "logs/server.log");
    assert_eq!(diff.resized_files[0].delta_apparent_bytes, 50_000);

    // 6. Save second snapshot T1, list snapshots, load T0 back into Explore, and delete T0
    let snap_t1: SnapshotHeader = http_post_json(
        &server,
        "/api/snapshots/save",
        &SaveSnapshotRequest {
            label: Some("After Growth T1".to_string()),
        },
    );
    let listed: Vec<SnapshotHeader> = http_get_json(&server, "/api/snapshots");
    assert_eq!(listed.len(), 2);

    let loaded_ov: serde_json::Value = http_post_json(
        &server,
        "/api/snapshots/load",
        &LoadSnapshotRequest {
            snapshot_id: snap_t0.id,
        },
    );
    assert_eq!(loaded_ov["total_apparent_bytes"], 40_000);

    let del_res: serde_json::Value = http_post_json(
        &server,
        "/api/snapshots/delete",
        &DeleteSnapshotRequest {
            snapshot_id: snap_t0.id,
        },
    );
    assert_eq!(del_res["deleted"], true);
    let remaining: Vec<SnapshotHeader> = http_get_json(&server, "/api/snapshots");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, snap_t1.id);
}

fn write_file(path: &Path, size: usize) {
    let mut f = File::create(path).unwrap();
    if size > 0 {
        f.write_all(&vec![b'X'; size]).unwrap();
    }
}

fn raw_http_request(addr: std::net::SocketAddr, raw_req: &str) -> (String, String) {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.write_all(raw_req.as_bytes()).unwrap();
    let mut buf = String::new();
    stream.read_to_string(&mut buf).unwrap();
    let (headers, body) = buf.split_once("\r\n\r\n").unwrap_or((&buf, ""));
    let status_line = headers.lines().next().unwrap_or("").to_string();
    (status_line, body.to_string())
}

fn http_get_raw(server: &AppServer, path: &str) -> String {
    let addr = server.addr();
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-DiskGarden-Token: {}\r\nConnection: close\r\n\r\n",
        path,
        addr.port(),
        server.session_token()
    );
    let (status_line, body) = raw_http_request(addr, &req);
    assert!(
        status_line.starts_with("HTTP/1.1 200"),
        "expected 200 OK for GET {}, got: {}\nBody: {}",
        path,
        status_line,
        body
    );
    body
}

fn http_get_json<T: DeserializeOwned>(server: &AppServer, path: &str) -> T {
    let body = http_get_raw(server, path);
    serde_json::from_str(&body)
        .unwrap_or_else(|e| panic!("failed to parse JSON from {}: {}\nBody: {}", path, e, body))
}

fn http_post_json<B: Serialize, T: DeserializeOwned>(
    server: &AppServer,
    path: &str,
    payload: &B,
) -> T {
    let addr = server.addr();
    let json_bytes = serde_json::to_vec(payload).unwrap();
    let mut stream = TcpStream::connect(addr).unwrap();
    let req = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nOrigin: http://127.0.0.1:{}\r\nX-DiskGarden-Token: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        path,
        addr.port(),
        addr.port(),
        server.session_token(),
        json_bytes.len()
    );
    stream.write_all(req.as_bytes()).unwrap();
    stream.write_all(&json_bytes).unwrap();
    let mut buf = String::new();
    stream.read_to_string(&mut buf).unwrap();
    let (_, body) = buf.split_once("\r\n\r\n").unwrap();
    serde_json::from_str(body).unwrap_or_else(|e| {
        panic!(
            "failed to parse JSON from POST {}: {}\nBody: {}",
            path, e, body
        )
    })
}
