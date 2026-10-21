use disk_garden_core::{
    build_treemap_view, children_sorted, extension_distribution, filter_entries,
    raw_bytes_to_path_buf, scan_directory, scan_directory_with_progress, top_directories,
    top_files, AccessStatus, CancelToken, EntryFilter, EntryType, ScanConfig, ScanHandle,
    ScanStatus, SizeMetric, SortField,
};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{ symlink, PermissionsExt};
use std::path::PathBuf;
use tempfile::TempDir;

/// RAII helper to restore directory permissions on drop so `TempDir` cleanup always succeeds.
struct ChmodGuard {
    path: PathBuf,
    restore_mode: u32,
}

impl Drop for ChmodGuard {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.restore_mode));
    }
}

#[test]
fn test_empty_directory() {
    let tmp = TempDir::new().expect("create tempdir");
    let config = ScanConfig::new(tmp.path());
    let token = CancelToken::new();

    let res = scan_directory(&config, &token);

    assert_eq!(res.status, ScanStatus::Completed);
    assert_eq!(res.total_files_count, 0);
    assert_eq!(res.total_dirs_count, 0);
    assert_eq!(res.total_symlinks_count, 0);
    assert_eq!(res.total_apparent_bytes, 0);
    assert_eq!(res.total_allocated_bytes, 0);
    assert_eq!(res.entries.len(), 1);

    let root = res.root().expect("root node present");
    assert_eq!(root.entry_type, EntryType::Directory);
    assert_eq!(root.access_status, AccessStatus::Ok);
    assert_eq!(root.depth, 0);
}

#[test]
fn test_nested_hierarchy_totals_and_sorting() {
    let tmp = TempDir::new().expect("create tempdir");
    let root = tmp.path();

    fs::create_dir_all(root.join("alpha/sub1")).unwrap();
    fs::create_dir_all(root.join("beta")).unwrap();
    fs::create_dir_all(root.join("empty_sub")).unwrap();

    // Create files with known logical sizes
    write_sized_file(&root.join("root_small.txt"), 512);
    write_sized_file(&root.join("alpha/a_medium.bin"), 10_000);
    write_sized_file(&root.join("alpha/sub1/a_deep_large.iso"), 1_200_000);
    write_sized_file(&root.join("beta/b_data.log"), 250_000);
    write_sized_file(&root.join("beta/zero.empty"), 0);

    let config = ScanConfig::new(root);
    let res = scan_directory(&config, &CancelToken::new());

    assert_eq!(res.status, ScanStatus::Completed);
    assert_eq!(res.total_files_count, 5);
    assert_eq!(res.total_dirs_count, 4); // alpha, alpha/sub1, beta, empty_sub
    assert_eq!(res.total_apparent_bytes, 512 + 10_000 + 1_200_000 + 250_000);

    // Check top files by apparent size
    let top_f = top_files(&res, 3, SizeMetric::Apparent, false);
    assert_eq!(top_f.len(), 3);
    assert_eq!(top_f[0].rel_path_display, "alpha/sub1/a_deep_large.iso");
    assert_eq!(top_f[0].apparent_bytes, 1_200_000);
    assert_eq!(top_f[1].rel_path_display, "beta/b_data.log");
    assert_eq!(top_f[1].apparent_bytes, 250_000);
    assert_eq!(top_f[2].rel_path_display, "alpha/a_medium.bin");
    assert_eq!(top_f[2].apparent_bytes, 10_000);

    // Check top directories by apparent subtree size
    let top_d = top_directories(&res, 4, SizeMetric::Apparent);
    assert_eq!(top_d.len(), 4);
    assert_eq!(top_d[0].rel_path_display, "alpha");
    assert_eq!(top_d[0].subtree_apparent_bytes, 1_210_000);
    assert_eq!(top_d[0].direct_files_apparent_bytes, 10_000);
    assert_eq!(top_d[0].subtree_files_count, 2);
    assert_eq!(top_d[0].subtree_dirs_count, 1);

    assert_eq!(top_d[1].rel_path_display, "alpha/sub1");
    assert_eq!(top_d[1].subtree_apparent_bytes, 1_200_000);

    assert_eq!(top_d[2].rel_path_display, "beta");
    assert_eq!(top_d[2].subtree_apparent_bytes, 250_000);

    assert_eq!(top_d[3].rel_path_display, "empty_sub");
    assert_eq!(top_d[3].subtree_apparent_bytes, 0);

    // Check children sorting of root
    let root_id = res.root_id.unwrap();
    let root_children_by_name = children_sorted(&res, root_id, SortField::Name, false);
    let names: Vec<&str> = root_children_by_name
        .iter()
        .map(|n| n.name_display.as_str())
        .collect();
    assert_eq!(names, vec!["alpha", "beta", "empty_sub", "root_small.txt"]);
}

#[test]
fn test_unicode_special_chars_and_invalid_utf8_paths() {
    let tmp = TempDir::new().expect("create tempdir");
    let root = tmp.path();

    let unicode_name = "giardino_🌱_日本語_Ω.dat";
    let shell_chars_name = "file with spaces; rm -rf -- $(echo unsafe) `tick` 'single' \"dbl\".txt";
    let invalid_utf8_bytes = vec![
        b'r', b'a', b'w', b'_', 0xFF, 0xFE, 0x80, b'_', b'l', b'i', b'n', b'u', b'x', b'.', b'b',
        b'i', b'n',
    ];
    let invalid_utf8_os = OsString::from_vec(invalid_utf8_bytes.clone());

    write_sized_file(&root.join(unicode_name), 1024);
    write_sized_file(&root.join(shell_chars_name), 2048);
    write_sized_file(&root.join(&invalid_utf8_os), 4096);

    let res = scan_directory(&ScanConfig::new(root), &CancelToken::new());
    assert_eq!(res.status, ScanStatus::Completed);
    assert_eq!(res.total_files_count, 3);
    assert_eq!(res.total_apparent_bytes, 1024 + 2048 + 4096);

    let non_utf8_node = res
        .entries
        .iter()
        .find(|e| !e.is_valid_utf8)
        .expect("should find non-UTF-8 entry");
    assert_eq!(non_utf8_node.name_raw, invalid_utf8_bytes);
    assert_eq!(non_utf8_node.rel_path_raw, invalid_utf8_bytes);
    assert_eq!(non_utf8_node.apparent_bytes, 4096);

    // Verify lossless path reconstruction from raw POSIX bytes
    let reconstructed = root.join(raw_bytes_to_path_buf(&non_utf8_node.rel_path_raw));
    assert!(reconstructed.exists());
}

#[test]
fn test_symlinks_and_mutual_symlink_cycles() {
    let tmp = TempDir::new().expect("create tempdir");
    let root = tmp.path();

    fs::create_dir_all(root.join("dir_a")).unwrap();
    fs::create_dir_all(root.join("dir_b")).unwrap();
    write_sized_file(&root.join("dir_a/real_file.bin"), 8192);

    // Create mutual directory symlink cycle: dir_a/to_b -> ../dir_b and dir_b/to_a -> ../dir_a
    symlink("../dir_b", root.join("dir_a/to_b")).unwrap();
    symlink("../dir_a", root.join("dir_b/to_a")).unwrap();
    // Self cycle: dir_a/self_loop -> .
    symlink(".", root.join("dir_a/self_loop")).unwrap();
    // Symlink to file and broken symlink
    symlink("real_file.bin", root.join("dir_a/file_link")).unwrap();
    symlink("/nonexistent/ disk-garden-missing", root.join("dir_b/broken_link")).unwrap();

    // 1. Default scan (`follow_symlinks = false`): symlinks are recorded but never traversed
    let cfg_default = ScanConfig::new(root);
    let res_default = scan_directory(&cfg_default, &CancelToken::new());
    assert_eq!(res_default.status, ScanStatus::Completed);
    assert_eq!(res_default.total_files_count, 1);
    assert_eq!(res_default.total_symlinks_count, 5);
    assert_eq!(res_default.total_apparent_bytes, 8192);
    assert_eq!(res_default.symlink_cycles_count, 0);

    // 2. Scan with `follow_symlinks = true`: must detect cycles and terminate without infinite recursion
    let mut cfg_follow = ScanConfig::new(root);
    cfg_follow.follow_symlinks = true;
    let res_follow = scan_directory(&cfg_follow, &CancelToken::new());
    assert_eq!(res_follow.status, ScanStatus::Completed);
    assert!(
        res_follow.symlink_cycles_count >= 2,
        "expected cycle detection to trigger at least twice, got {}",
        res_follow.symlink_cycles_count
    );
    assert!(res_follow
        .entries
        .iter()
        .any(|e| e.access_status == AccessStatus::SymlinkCycleSkipped));
}

#[test]
fn test_permission_denied_is_partial_not_empty_directory() {
    let tmp = TempDir::new().expect("create tempdir");
    let root = tmp.path();

    let accessible_empty = root.join("accessible_empty");
    let restricted_dir = root.join("restricted_vault");
    fs::create_dir_all(&accessible_empty).unwrap();
    fs::create_dir_all(&restricted_dir).unwrap();
    write_sized_file(&restricted_dir.join("hidden.dat"), 4096);
    write_sized_file(&root.join("visible.dat"), 2048);

    // Remove all permissions from `restricted_vault`
    fs::set_permissions(&restricted_dir, fs::Permissions::from_mode(0o000)).unwrap();
    let _guard = ChmodGuard {
        path: restricted_dir.clone(),
        restore_mode: 0o755,
    };

    // Only assert PermissionDenied behavior when running as non-root user
    if unsafe { libc::geteuid() } != 0 {
        let res = scan_directory(&ScanConfig::new(root), &CancelToken::new());
        assert_eq!(
            res.status,
            ScanStatus::Partial,
            "scan with unreadable directory must be marked Partial, never Completed"
        );
        assert_eq!(res.permission_denied_count, 1);
        assert_eq!(res.unreadable_dirs_count, 1);
        assert_eq!(res.total_apparent_bytes, 2048);

        let empty_node = res
            .entries
            .iter()
            .find(|e| e.name_display == "accessible_empty")
            .unwrap();
        assert_eq!(empty_node.access_status, AccessStatus::Ok);
        assert_eq!(empty_node.subtree_error_count, 0);

        let restricted_node = res
            .entries
            .iter()
            .find(|e| e.name_display == "restricted_vault")
            .unwrap();
        assert_eq!(restricted_node.access_status, AccessStatus::PermissionDenied);
        assert_eq!(restricted_node.subtree_error_count, 1);

        // Root must propagate `subtree_error_count`
        assert_eq!(res.root().unwrap().subtree_error_count, 1);

        // Filtering by `only_errors` must surface the restricted directory
        let err_entries = filter_entries(
            &res,
            &EntryFilter {
                only_errors: true,
                ..Default::default()
            },
            SortField::Name,
            false,
        );
        assert_eq!(err_entries.len(), 1);
        assert_eq!(err_entries[0].name_display, "restricted_vault");
    }
}

#[test]
fn test_sparse_file_apparent_vs_allocated() {
    let tmp = TempDir::new().expect("create tempdir");
    let root = tmp.path();
    let sparse_path = root.join("sparse_disk.img");

    let f = File::create(&sparse_path).unwrap();
    let logical_len = 16 * 1024 * 1024; // 16 MiB logical size, 0 bytes written
    f.set_len(logical_len).unwrap();
    drop(f);

    let res = scan_directory(&ScanConfig::new(root), &CancelToken::new());
    assert_eq!(res.status, ScanStatus::Completed);
    assert_eq!(res.total_apparent_bytes, logical_len);
    assert!(
        res.total_allocated_bytes < logical_len,
        "expected sparse file allocated_bytes ({}) < apparent_bytes ({})",
        res.total_allocated_bytes,
        logical_len
    );

    let sparse_entries = filter_entries(
        &res,
        &EntryFilter {
            only_sparse: true,
            ..Default::default()
        },
        SortField::ApparentBytes,
        true,
    );
    assert_eq!(sparse_entries.len(), 1);
    assert!(sparse_entries[0].is_sparse());
}

#[test]
fn test_hardlinks_deduplication_and_external_link_detection() {
    let tmp = TempDir::new().expect("create tempdir");
    let scan_root = tmp.path().join("workspace");
    fs::create_dir_all(scan_root.join("dir1")).unwrap();
    fs::create_dir_all(scan_root.join("dir2")).unwrap();

    let orig = scan_root.join("dir1/original.bin");
    write_sized_file(&orig, 65_536);
    let link_in_dir1 = scan_root.join("dir1/hardlink_a.bin");
    let link_in_dir2 = scan_root.join("dir2/hardlink_b.bin");
    let external_link = tmp.path().join("outside_workspace_link.bin");

    fs::hard_link(&orig, &link_in_dir1).unwrap();
    fs::hard_link(&orig, &link_in_dir2).unwrap();
    fs::hard_link(&orig, &external_link).unwrap();

    // Scan `workspace`: 3 paths inside `workspace`, 1 path outside (`st_nlink == 4`).
    let res = scan_directory(&ScanConfig::new(&scan_root), &CancelToken::new());
    assert_eq!(res.status, ScanStatus::Completed);
    assert_eq!(res.total_files_count, 3);
    assert_eq!(res.hardlink_duplicate_files_count, 2);
    // Physical/deduplicated total must count the 65,536-byte inode ONLY ONCE
    assert_eq!(res.total_apparent_bytes, 65_536);
    assert_eq!(res.hardlink_saved_apparent_bytes, 2 * 65_536);

    let hl_nodes = filter_entries(
        &res,
        &EntryFilter {
            only_hardlinks: true,
            ..Default::default()
        },
        SortField::Name,
        false,
    );
    assert_eq!(hl_nodes.len(), 3);
    for node in &hl_nodes {
        assert_eq!(node.nlink, 4);
        assert_eq!(node.shared_link_paths_in_scan, 3);
        assert!(
            node.has_external_hardlinks(),
            "should detect that 1 of the 4 hard links resides outside scan_root"
        );
    }

    // Remove the external hard link and rescan: now all 3 links are inside `workspace`
    fs::remove_file(&external_link).unwrap();
    let res_internal = scan_directory(&ScanConfig::new(&scan_root), &CancelToken::new());
    for node in res_internal
        .entries
        .iter()
        .filter(|e| e.entry_type == EntryType::File)
    {
        assert_eq!(node.nlink, 3);
        assert_eq!(node.shared_link_paths_in_scan, 3);
        assert!(
            !node.has_external_hardlinks(),
            "all hard links are now accounted for within scan_root"
        );
    }
}

#[test]
fn test_cancellation_and_background_scan_handle() {
    let tmp = TempDir::new().expect("create tempdir");
    let root = tmp.path();

    // Create 20 subdirectories with 40 files each = 800 files
    for d in 0..20 {
        let sub = root.join(format!("pkg_{:02}", d));
        fs::create_dir_all(&sub).unwrap();
        for f in 0..40 {
            write_sized_file(&sub.join(format!("item_{:03}.dat", f)), 256);
        }
    }

    // 1. Mid-scan cancellation test
    let mut cfg = ScanConfig::new(root);
    cfg.progress_interval_ms = 0; // emit progress on every directory
    let token = CancelToken::new();
    let token_cb = token.clone();
    let mut callbacks = 0usize;

    let res_cancelled = scan_directory_with_progress(&cfg, &token, |_prog| {
        callbacks += 1;
        if callbacks >= 5 {
            token_cb.cancel();
        }
    });

    assert_eq!(
        res_cancelled.status,
        ScanStatus::Cancelled,
        "cancelled scan must never be marked Completed or Partial"
    );
    assert!(
        res_cancelled.total_files_count < 800,
        "cancelled scan should stop early (scanned {} / 800 files)",
        res_cancelled.total_files_count
    );

    // 2. Full background non-blocking ScanHandle execution + Treemap tail bucketing + Extension stats
    let handle = ScanHandle::spawn(cfg);
    let mut progress_events = 0usize;
    let res_full = handle
        .wait_with_progress(|_p| {
            progress_events += 1;
        })
        .expect("background scan finished");

    assert_eq!(res_full.status, ScanStatus::Completed);
    assert_eq!(res_full.total_files_count, 800);
    assert_eq!(res_full.total_dirs_count, 20);
    assert_eq!(res_full.total_apparent_bytes, 800 * 256);
    assert!(progress_events >= 20);

    // Verify treemap tail bucketing caps visible tiles
    let root_id = res_full.root_id.unwrap();
    let tiles = build_treemap_view(&res_full, root_id, SizeMetric::Apparent, 8, 0.01);
    assert_eq!(tiles.len(), 9); // 8 top directories + 1 tail bucket for the remaining 12 directories
    assert!(tiles.last().unwrap().is_tail_bucket);
    assert_eq!(tiles.last().unwrap().bucketed_items_count, 12);
    let sum_tile_apparent: u64 = tiles.iter().map(|t| t.apparent_bytes).sum();
    assert_eq!(sum_tile_apparent, res_full.total_apparent_bytes);

    // Verify extension distribution
    let ext_stats = extension_distribution(&res_full);
    assert_eq!(ext_stats.len(), 1);
    assert_eq!(ext_stats[0].extension, "dat");
    assert_eq!(ext_stats[0].files_count, 800);
    assert_eq!(ext_stats[0].apparent_bytes, 800 * 256);
}

#[test]
fn test_vanished_directory_during_scan_is_handled_gracefully() {
    let tmp = TempDir::new().expect("create tempdir");
    let root = tmp.path();

    let stable_dir = root.join("01_stable");
    let vanishing_dir = root.join("02_vanishing");
    fs::create_dir_all(&stable_dir).unwrap();
    fs::create_dir_all(&vanishing_dir).unwrap();
    write_sized_file(&stable_dir.join("keep.bin"), 4096);
    write_sized_file(&vanishing_dir.join("gone.bin"), 8192);

    let mut cfg = ScanConfig::new(root);
    cfg.progress_interval_ms = 0;
    let vanish_target = vanishing_dir.clone();

    let res = scan_directory_with_progress(&cfg, &CancelToken::new(), |prog| {
        if prog.current_rel_path == "02_vanishing" && vanish_target.exists() {
            let _ = fs::remove_dir_all(&vanish_target);
        }
    });

    assert_eq!(res.status, ScanStatus::Completed);
    assert_eq!(res.vanished_count, 1);
    assert_eq!(res.total_apparent_bytes, 4096);
    let vanished_node = res
        .entries
        .iter()
        .find(|e| e.name_display == "02_vanishing")
        .expect("vanished node recorded");
    assert_eq!(vanished_node.access_status, AccessStatus::Vanished);
}

#[test]
fn test_invalid_root_paths_fail_gracefully() {
    let tmp = TempDir::new().expect("create tempdir");
    let missing = tmp.path().join("does_not_exist");
    let res_missing = scan_directory(&ScanConfig::new(&missing), &CancelToken::new());
    assert_eq!(res_missing.status, ScanStatus::Failed);
    assert!(res_missing.root_error_message.is_some());

    let file_path = tmp.path().join("not_a_dir.txt");
    write_sized_file(&file_path, 128);
    let res_file = scan_directory(&ScanConfig::new(&file_path), &CancelToken::new());
    assert_eq!(res_file.status, ScanStatus::Failed);
    assert!(res_file.root_error_message.is_some());
}

#[test]
fn test_mount_boundary_exclusion() {
    let tmp = TempDir::new().expect("create tempdir");
    let root = tmp.path();
    write_sized_file(&root.join("local.bin"), 1024);

    // Link to `/proc` or `/dev/shm`, which resides on a different filesystem (`st_dev != root_dev`)
    let other_mount = std::path::Path::new("/proc");
    if other_mount.exists() {
        symlink(other_mount, root.join("mounted_proc")).unwrap();

        let mut cfg = ScanConfig::new(root);
        cfg.follow_symlinks = true;
        cfg.cross_mounts = false; // default: do not cross mount boundaries

        let res = scan_directory(&cfg, &CancelToken::new());
        assert_eq!(res.status, ScanStatus::Completed);
        assert_eq!(res.excluded_mounts_count, 1);
        assert_eq!(res.total_apparent_bytes, 1024);

        let mount_node = res
            .entries
            .iter()
            .find(|e| e.name_display == "mounted_proc")
            .expect("mounted_proc entry exists");
        assert_eq!(mount_node.access_status, AccessStatus::ExcludedMount);
    }
}

fn write_sized_file(path: &std::path::Path, size: usize) {
    let mut file = File::create(path).expect("create test file");
    if size > 0 {
        let buf = vec![0x5Au8; size];
        file.write_all(&buf).expect("write test file");
    }
}
