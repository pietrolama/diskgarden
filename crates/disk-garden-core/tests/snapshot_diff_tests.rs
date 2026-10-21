use disk_garden_core::{
    compare_scans, scan_directory, CancelToken, DiffError, DiffOptions, DiffSortField,
    DirectoryChangeKind, ScanConfig, ScanStatus, SizeMetric, SnapshotError, SnapshotStore,
    UnreliableReason,
};
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::TempDir;

fn write_bytes(path: &Path, len: usize, fill: u8) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut f = File::create(path).unwrap();
    if len > 0 {
        f.write_all(&vec![fill; len]).unwrap();
    }
}

#[test]
fn test_snapshot_store_save_list_load_and_delete() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("snapshots.db");
    let root = tmp.path().join("workspace");

    write_bytes(&root.join("alpha/one.bin"), 10_000, b'A');
    write_bytes(&root.join("beta/two.bin"), 25_000, b'B');

    let store = SnapshotStore::open(&db_path).expect("open sqlite store");
    let scan = scan_directory(&ScanConfig::new(&root), &CancelToken::new());
    assert_eq!(scan.status, ScanStatus::Completed);

    let header = store
        .save_snapshot(&scan, Some("Baseline v1"), false, false)
        .expect("save snapshot");
    assert!(header.id > 0);
    assert_eq!(header.label.as_deref(), Some("Baseline v1"));
    assert_eq!(header.total_apparent_bytes, 35_000);
    assert_eq!(header.total_files_count, 2);

    // List snapshots filtered by root path
    let listed = store
        .list_snapshots(Some(&root))
        .expect("list snapshots for root");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, header.id);

    // Load full snapshot back and verify reconstructed tree
    let loaded = store.load_snapshot(header.id).expect("load snapshot");
    assert_eq!(loaded.header.id, header.id);
    assert_eq!(loaded.scan.total_apparent_bytes, 35_000);
    assert_eq!(loaded.scan.entries.len(), scan.entries.len());
    assert!(loaded.scan.root_id.is_some());

    // Delete snapshot and verify it is removed
    let deleted = store.delete_snapshot(header.id).expect("delete snapshot");
    assert!(deleted);
    assert!(store.list_snapshots(None).unwrap().is_empty());
    assert!(matches!(
        store.load_snapshot(header.id),
        Err(SnapshotError::NotFound(_))
    ));
}

#[test]
fn test_incomplete_or_mismatched_scans_rejected() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("snapshots.db");
    let root_a = tmp.path().join("root_a");
    let root_b = tmp.path().join("root_b");
    write_bytes(&root_a.join("file.txt"), 1024, b'x');
    write_bytes(&root_b.join("file.txt"), 1024, b'y');

    let store = SnapshotStore::open(&db_path).unwrap();
    let mut cancelled_scan = scan_directory(&ScanConfig::new(&root_a), &CancelToken::new());
    cancelled_scan.status = ScanStatus::Cancelled;

    // 1. Saving a cancelled scan as a snapshot must fail
    let err = store
        .save_snapshot(&cancelled_scan, Some("Incomplete"), false, false)
        .unwrap_err();
    assert!(matches!(
        err,
        SnapshotError::IncompleteScan(ScanStatus::Cancelled)
    ));

    // 2. Comparing a cancelled scan against a completed scan must fail
    let valid_scan_a = scan_directory(&ScanConfig::new(&root_a), &CancelToken::new());
    let diff_err = compare_scans(&cancelled_scan, &valid_scan_a, &DiffOptions::default())
        .expect_err("cancelled scan must not be compared as complete");
    assert_eq!(
        diff_err,
        DiffError::IncompleteScanCannotBeCompared {
            side: "base",
            status: ScanStatus::Cancelled
        }
    );

    // 3. Comparing two different roots must fail with RootPathMismatch
    let valid_scan_b = scan_directory(&ScanConfig::new(&root_b), &CancelToken::new());
    let mismatch_err = compare_scans(&valid_scan_a, &valid_scan_b, &DiffOptions::default())
        .expect_err("different roots must fail");
    assert!(matches!(mismatch_err, DiffError::RootPathMismatch { .. }));
}

#[test]
fn test_temporal_diff_added_removed_resized_metadata_only_and_unreliable_paths() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("snapshots.db");
    let root = tmp.path().join("project");

    // Initial state (T0):
    // - logs/app.log (20_000 B) -> will grow to 80_000 B in T1 (+60_000 B)
    // - cache/old.tmp (15_000 B) -> will be removed in T1 (-15_000 B)
    // - config/settings.json (4_096 B) -> will be rewritten via atomic rename with SAME size 4_096 B in T1 (delta = 0, inode changed!)
    // - morph_item (file of 2_000 B) -> will become a directory in T1 (EntryTypeChanged -> UnreliablePath)
    // - secret_vault/inside.dat (12_000 B) -> secret_vault will become chmod 000 in T1 (PermissionDenied -> UnreliablePath, and inside.dat must NOT be reported as removed!)
    write_bytes(&root.join("logs/app.log"), 20_000, b'L');
    write_bytes(&root.join("cache/old.tmp"), 15_000, b'C');
    write_bytes(&root.join("config/settings.json"), 4_096, b'S');
    write_bytes(&root.join("morph_item"), 2_000, b'M');
    write_bytes(&root.join("secret_vault/inside.dat"), 12_000, b'V');

    let store = SnapshotStore::open(&db_path).unwrap();
    let scan_t0 = scan_directory(&ScanConfig::new(&root), &CancelToken::new());
    assert_eq!(scan_t0.status, ScanStatus::Completed);
    let snap_t0 = store
        .save_snapshot(&scan_t0, Some("T0 Baseline"), false, false)
        .unwrap();

    // Apply controlled synthetic mutations for T1:
    // 1. Grow logs/app.log from 20_000 B to 80_000 B
    write_bytes(&root.join("logs/app.log"), 80_000, b'L');
    // 2. Remove cache/old.tmp
    fs::remove_file(root.join("cache/old.tmp")).unwrap();
    // 3. Add build/artifact.tar (50_000 B)
    write_bytes(&root.join("build/artifact.tar"), 50_000, b'B');
    // 4. Atomic replace config/settings.json with exact same size (4_096 B) -> new inode, delta = 0
    let tmp_cfg = root.join("config/settings.json.new");
    write_bytes(&tmp_cfg, 4_096, b'N');
    fs::rename(&tmp_cfg, root.join("config/settings.json")).unwrap();
    // 5. Replace file `morph_item` with a directory `morph_item`
    fs::remove_file(root.join("morph_item")).unwrap();
    fs::create_dir_all(root.join("morph_item")).unwrap();
    // 6. Make `secret_vault` unreadable (chmod 000) if not running as root
    let is_root = unsafe { libc::geteuid() == 0 };
    if !is_root {
        fs::set_permissions(
            root.join("secret_vault"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
    }

    let scan_t1 = scan_directory(&ScanConfig::new(&root), &CancelToken::new());
    let snap_t1 = store
        .save_snapshot(&scan_t1, Some("T1 After Build"), false, false)
        .unwrap();

    // Restore permissions on secret_vault so TempDir cleanup succeeds
    if !is_root {
        let _ = fs::set_permissions(
            root.join("secret_vault"),
            fs::Permissions::from_mode(0o755),
        );
    }

    let loaded_t0 = store.load_snapshot(snap_t0.id).unwrap();
    let loaded_t1 = store.load_snapshot(snap_t1.id).unwrap();

    let mut opts = DiffOptions {
        sort_field: DiffSortField::SignedDelta,
        descending: true,
        metric: SizeMetric::Apparent,
        max_items_per_category: 100,
        base_snapshot_id: Some(snap_t0.id),
        target_snapshot_id: Some(snap_t1.id),
        base_label: snap_t0.label.clone(),
        target_label: snap_t1.label.clone(),
    };

    let report = compare_scans(&loaded_t0.scan, &loaded_t1.scan, &opts).expect("compare t0 and t1");

    // Verify added files: `build/artifact.tar` (50_000 B)
    assert_eq!(report.added_files.len(), 1);
    assert_eq!(report.added_files[0].rel_path_display, "build/artifact.tar");
    assert_eq!(report.added_files[0].apparent_bytes, 50_000);

    // Verify removed files: ONLY `cache/old.tmp` (15_000 B).
    // `secret_vault/inside.dat` MUST NOT be falsely reported as removed when `secret_vault` is PermissionDenied!
    if !is_root {
        assert_eq!(report.removed_files.len(), 1);
        assert_eq!(report.removed_files[0].rel_path_display, "cache/old.tmp");
        assert_eq!(report.removed_files[0].apparent_bytes, 15_000);
    }

    // Verify resized files: ONLY `logs/app.log` (+60_000 B).
    // `config/settings.json` (same 4_096 B, replaced inode) MUST NOT be in `resized_files`!
    assert_eq!(report.resized_files.len(), 1);
    assert_eq!(report.resized_files[0].rel_path_display, "logs/app.log");
    assert_eq!(report.resized_files[0].base_apparent_bytes, 20_000);
    assert_eq!(report.resized_files[0].target_apparent_bytes, 80_000);
    assert_eq!(report.resized_files[0].delta_apparent_bytes, 60_000);

    // Verify metadata-only modified files: `config/settings.json` (4_096 B, replaced_inode = true)
    assert_eq!(report.metadata_only_modified_files.len(), 1);
    assert_eq!(
        report.metadata_only_modified_files[0].rel_path_display,
        "config/settings.json"
    );
    assert_eq!(report.metadata_only_modified_files[0].apparent_bytes, 4_096);
    assert!(report.metadata_only_modified_files[0].replaced_inode);

    // Verify unreliable paths: `morph_item` (EntryTypeChanged) and `secret_vault` (TargetAccessIncomplete)
    assert!(report
        .unreliable_paths
        .iter()
        .any(|u| u.rel_path_display == "morph_item"
            && matches!(u.reason, UnreliableReason::EntryTypeChanged { .. })));
    if !is_root {
        assert!(report
            .unreliable_paths
            .iter()
            .any(|u| u.rel_path_display == "secret_vault"
                && matches!(u.reason, UnreliableReason::TargetAccessIncomplete(_))));
        assert!(!report.compatibility_warnings.is_empty());
    }

    // Verify directory growth sorting: `logs` (+60_000 B), `build` (+50_000 B, Added), `cache` (-15_000 B, Shrunk)
    let logs_dir = report
        .directory_deltas
        .iter()
        .find(|d| d.rel_path_display == "logs")
        .expect("logs dir delta");
    assert_eq!(logs_dir.delta_apparent_bytes, 60_000);
    assert_eq!(logs_dir.change_kind, DirectoryChangeKind::Grown);

    let build_dir = report
        .directory_deltas
        .iter()
        .find(|d| d.rel_path_display == "build")
        .expect("build dir delta");
    assert_eq!(build_dir.delta_apparent_bytes, 50_000);
    assert_eq!(build_dir.change_kind, DirectoryChangeKind::Added);

    let cache_dir = report
        .directory_deltas
        .iter()
        .find(|d| d.rel_path_display == "cache")
        .expect("cache dir delta");
    assert_eq!(cache_dir.delta_apparent_bytes, -15_000);
    assert_eq!(cache_dir.change_kind, DirectoryChangeKind::Shrunk);

    // Verify sorting by RelativePath ascending
    opts.sort_field = DiffSortField::RelativePath;
    opts.descending = false;
    let report_by_path = compare_scans(&loaded_t0.scan, &loaded_t1.scan, &opts).unwrap();
    let paths: Vec<&str> = report_by_path
        .directory_deltas
        .iter()
        .map(|d| d.rel_path_display.as_str())
        .collect();
    let mut sorted_paths = paths.clone();
    sorted_paths.sort_unstable();
    assert_eq!(paths, sorted_paths);
}
