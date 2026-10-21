-- DiskGarden — Initial Data Model (SQLite Schema v1)
-- Milestone 0 Deliverable: Bozza del modello dati

PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY NOT NULL,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    schema_version INTEGER NOT NULL DEFAULT 1,
    -- Canonical absolute path of the scanned root directory (raw POSIX bytes + UTF-8 lossy display)
    root_path_raw BLOB NOT NULL,
    root_path_display TEXT NOT NULL,
    label TEXT,
    created_at_unix_ms INTEGER NOT NULL,
    scan_duration_ms INTEGER NOT NULL DEFAULT 0,
    -- Scan lifecycle status: 'saving' | 'completed' | 'partial' | 'cancelled' | 'interrupted'
    status TEXT NOT NULL CHECK (status IN ('saving', 'completed', 'partial', 'cancelled', 'interrupted')),
    -- Scan compatibility parameters
    root_dev INTEGER NOT NULL,
    cross_mounts INTEGER NOT NULL DEFAULT 0,
    follow_symlinks INTEGER NOT NULL DEFAULT 0,
    -- Root aggregate metrics (hard-link deduplicated for allocated_bytes)
    total_apparent_bytes INTEGER NOT NULL DEFAULT 0,
    total_allocated_bytes INTEGER NOT NULL DEFAULT 0,
    total_files_count INTEGER NOT NULL DEFAULT 0,
    total_dirs_count INTEGER NOT NULL DEFAULT 0,
    total_symlinks_count INTEGER NOT NULL DEFAULT 0,
    hardlink_saved_bytes INTEGER NOT NULL DEFAULT 0,
    unreadable_dirs_count INTEGER NOT NULL DEFAULT 0,
    io_errors_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_snapshots_root_status
    ON snapshots(root_path_raw, status, created_at_unix_ms DESC);

CREATE TABLE IF NOT EXISTS snapshot_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    -- Relative path from root_path_raw (empty BLOB X'' for the root entry itself)
    rel_path_raw BLOB NOT NULL,
    rel_path_display TEXT NOT NULL,
    parent_rel_path_raw BLOB,
    name_display TEXT NOT NULL,
    is_valid_utf8 INTEGER NOT NULL DEFAULT 1,
    depth INTEGER NOT NULL,
    -- Entry type: 'dir' | 'file' | 'symlink' | 'other'
    entry_type TEXT NOT NULL CHECK (entry_type IN ('dir', 'file', 'symlink', 'other')),
    -- Self metrics (for files/symlinks) and subtree aggregates (for directories)
    apparent_bytes INTEGER NOT NULL DEFAULT 0,
    allocated_bytes INTEGER NOT NULL DEFAULT 0,
    subtree_apparent_bytes INTEGER NOT NULL DEFAULT 0,
    subtree_allocated_bytes INTEGER NOT NULL DEFAULT 0,
    -- Subtree counts (for directories)
    subtree_files_count INTEGER NOT NULL DEFAULT 0,
    subtree_dirs_count INTEGER NOT NULL DEFAULT 0,
    subtree_error_count INTEGER NOT NULL DEFAULT 0,
    -- POSIX metadata for temporal diffing and hard-link awareness
    dev INTEGER,
    ino INTEGER,
    nlink INTEGER NOT NULL DEFAULT 1,
    is_hardlink_duplicate INTEGER NOT NULL DEFAULT 0,
    mtime_sec INTEGER,
    mtime_nsec INTEGER,
    -- Access and validity status
    access_status TEXT NOT NULL CHECK (
        access_status IN ('ok', 'permission_denied', 'io_error', 'vanished', 'excluded_mount', 'symlink_skipped')
    ),
    error_message TEXT,
    UNIQUE(snapshot_id, rel_path_raw)
);

CREATE INDEX IF NOT EXISTS idx_entries_snapshot_parent
    ON snapshot_entries(snapshot_id, parent_rel_path_raw);

CREATE INDEX IF NOT EXISTS idx_entries_snapshot_type_size
    ON snapshot_entries(snapshot_id, entry_type, subtree_allocated_bytes DESC);
