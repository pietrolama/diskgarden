use crate::model::{
    AccessStatus, EntryId, EntryNode, EntryType, ScanConfig, ScanResult, ScanStatus,
};
use crate::scanner::raw_bytes_to_path_buf;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

const SCHEMA_V1_SQL: &str = include_str!("../../../docs/SCHEMA_V1.sql");

/// Errors produced by [`SnapshotStore`] persistence operations.
#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("Libreria di sistema SQLite3 (libsqlite3.so.0) non disponibile: {0}")]
    SqliteLibraryUnavailable(String),
    #[error("Errore SQLite ({code}): {message}")]
    Sqlite { code: i32, message: String },
    #[error("Errore I/O filesystem per il database snapshot: {0}")]
    Io(#[from] std::io::Error),
    #[error("Impossibile salvare come snapshot una scansione incompleta o annullata (stato: {0:?})")]
    IncompleteScan(ScanStatus),
    #[error("Snapshot con ID {0} non trovato nel database locale")]
    NotFound(i64),
}

/// Metadata header of a saved snapshot in the local SQLite store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotHeader {
    pub id: i64,
    pub schema_version: i32,
    #[serde(skip)]
    pub root_path_raw: Vec<u8>,
    pub root_path_display: String,
    pub label: Option<String>,
    pub created_at_unix_ms: i64,
    pub scan_duration_ms: u64,
    pub status: ScanStatus,
    pub root_dev: u64,
    pub cross_mounts: bool,
    pub follow_symlinks: bool,
    pub total_apparent_bytes: u64,
    pub total_allocated_bytes: u64,
    pub total_files_count: u64,
    pub total_dirs_count: u64,
    pub total_symlinks_count: u64,
    pub hardlink_saved_bytes: u64,
    pub unreadable_dirs_count: u64,
    pub io_errors_count: u64,
}

/// A full snapshot loaded from SQLite, containing its header and reconstructed [`ScanResult`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSnapshot {
    pub header: SnapshotHeader,
    pub scan: ScanResult,
}

// ============================================================================
// Dynamic SQLite3 C API Wrapper (`libsqlite3.so.0`)
// ============================================================================

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;
const SQLITE_TRANSIENT: isize = -1;

type Sqlite3Db = c_void;
type Sqlite3Stmt = c_void;

struct SqliteApi {
    open: unsafe extern "C" fn(*const c_char, *mut *mut Sqlite3Db) -> c_int,
    close: unsafe extern "C" fn(*mut Sqlite3Db) -> c_int,
    errmsg: unsafe extern "C" fn(*mut Sqlite3Db) -> *const c_char,
    exec: unsafe extern "C" fn(
        *mut Sqlite3Db,
        *const c_char,
        Option<unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_char, *mut *mut c_char) -> c_int>,
        *mut c_void,
        *mut *mut c_char,
    ) -> c_int,
    prepare_v2: unsafe extern "C" fn(
        *mut Sqlite3Db,
        *const c_char,
        c_int,
        *mut *mut Sqlite3Stmt,
        *mut *const c_char,
    ) -> c_int,
    step: unsafe extern "C" fn(*mut Sqlite3Stmt) -> c_int,
    reset: unsafe extern "C" fn(*mut Sqlite3Stmt) -> c_int,
    clear_bindings: unsafe extern "C" fn(*mut Sqlite3Stmt) -> c_int,
    finalize: unsafe extern "C" fn(*mut Sqlite3Stmt) -> c_int,
    last_insert_rowid: unsafe extern "C" fn(*mut Sqlite3Db) -> i64,
    changes: unsafe extern "C" fn(*mut Sqlite3Db) -> c_int,
    bind_int64: unsafe extern "C" fn(*mut Sqlite3Stmt, c_int, i64) -> c_int,
    bind_null: unsafe extern "C" fn(*mut Sqlite3Stmt, c_int) -> c_int,
    bind_text:
        unsafe extern "C" fn(*mut Sqlite3Stmt, c_int, *const c_char, c_int, isize) -> c_int,
    bind_blob:
        unsafe extern "C" fn(*mut Sqlite3Stmt, c_int, *const c_void, c_int, isize) -> c_int,
    column_int64: unsafe extern "C" fn(*mut Sqlite3Stmt, c_int) -> i64,
    column_text: unsafe extern "C" fn(*mut Sqlite3Stmt, c_int) -> *const u8,
    column_blob: unsafe extern "C" fn(*mut Sqlite3Stmt, c_int) -> *const c_void,
    column_bytes: unsafe extern "C" fn(*mut Sqlite3Stmt, c_int) -> c_int,
    column_type: unsafe extern "C" fn(*mut Sqlite3Stmt, c_int) -> c_int,
}

unsafe impl Send for SqliteApi {}
unsafe impl Sync for SqliteApi {}

static SQLITE_API: OnceLock<Result<SqliteApi, String>> = OnceLock::new();

#[allow(clippy::missing_transmute_annotations)]
fn get_sqlite_api() -> Result<&'static SqliteApi, SnapshotError> {
    let res = SQLITE_API.get_or_init(|| unsafe {
        let candidates = [
            "libsqlite3.so.0\0",
            "libsqlite3.so\0",
            "/usr/lib/x86_64-linux-gnu/libsqlite3.so.0\0",
            "/usr/lib64/libsqlite3.so.0\0",
        ];
        let mut handle: *mut c_void = ptr::null_mut();
        for cand in candidates {
            handle = libc::dlopen(
                cand.as_ptr() as *const c_char,
                libc::RTLD_NOW | libc::RTLD_LOCAL,
            );
            if !handle.is_null() {
                break;
            }
        }
        if handle.is_null() {
            return Err("Impossibile caricare libsqlite3.so.0 tramite dlopen".to_string());
        }

        macro_rules! load_sym {
            ($name:literal) => {{
                let sym = libc::dlsym(handle, concat!($name, "\0").as_ptr() as *const c_char);
                if sym.is_null() {
                    return Err(format!("Simbolo SQLite mancante: {}", $name));
                }
                std::mem::transmute(sym)
            }};
        }

        Ok(SqliteApi {
            open: load_sym!("sqlite3_open"),
            close: load_sym!("sqlite3_close"),
            errmsg: load_sym!("sqlite3_errmsg"),
            exec: load_sym!("sqlite3_exec"),
            prepare_v2: load_sym!("sqlite3_prepare_v2"),
            step: load_sym!("sqlite3_step"),
            reset: load_sym!("sqlite3_reset"),
            clear_bindings: load_sym!("sqlite3_clear_bindings"),
            finalize: load_sym!("sqlite3_finalize"),
            last_insert_rowid: load_sym!("sqlite3_last_insert_rowid"),
            changes: load_sym!("sqlite3_changes"),
            bind_int64: load_sym!("sqlite3_bind_int64"),
            bind_null: load_sym!("sqlite3_bind_null"),
            bind_text: load_sym!("sqlite3_bind_text"),
            bind_blob: load_sym!("sqlite3_bind_blob"),
            column_int64: load_sym!("sqlite3_column_int64"),
            column_text: load_sym!("sqlite3_column_text"),
            column_blob: load_sym!("sqlite3_column_blob"),
            column_bytes: load_sym!("sqlite3_column_bytes"),
            column_type: load_sym!("sqlite3_column_type"),
        })
    });

    match res {
        Ok(api) => Ok(api),
        Err(msg) => Err(SnapshotError::SqliteLibraryUnavailable(msg.clone())),
    }
}

struct SqliteConn {
    db: *mut Sqlite3Db,
    api: &'static SqliteApi,
}

impl SqliteConn {
    fn open(path: &Path) -> Result<Self, SnapshotError> {
        let api = get_sqlite_api()?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            SnapshotError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Il percorso del database contiene byte NUL",
            ))
        })?;
        let mut db: *mut Sqlite3Db = ptr::null_mut();
        let rc = unsafe { (api.open)(c_path.as_ptr(), &mut db) };
        if rc != SQLITE_OK || db.is_null() {
            let msg = if !db.is_null() {
                let s = unsafe { CStr::from_ptr((api.errmsg)(db)).to_string_lossy().into_owned() };
                unsafe { (api.close)(db) };
                s
            } else {
                "Impossibile allocare connessione SQLite".to_string()
            };
            return Err(SnapshotError::Sqlite {
                code: rc,
                message: msg,
            });
        }
        Ok(Self { db, api })
    }

    fn last_error(&self, code: c_int) -> SnapshotError {
        let msg = unsafe {
            let ptr = (self.api.errmsg)(self.db);
            if ptr.is_null() {
                "Errore SQLite sconosciuto".to_string()
            } else {
                CStr::from_ptr(ptr).to_string_lossy().into_owned()
            }
        };
        SnapshotError::Sqlite { code, message: msg }
    }

    fn exec_batch(&self, sql: &str) -> Result<(), SnapshotError> {
        let c_sql = CString::new(sql).unwrap();
        let rc = unsafe {
            (self.api.exec)(
                self.db,
                c_sql.as_ptr(),
                None,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if rc != SQLITE_OK {
            return Err(self.last_error(rc));
        }
        Ok(())
    }

    fn prepare<'a>(&'a self, sql: &str) -> Result<SqliteStmt<'a>, SnapshotError> {
        let c_sql = CString::new(sql).unwrap();
        let mut stmt: *mut Sqlite3Stmt = ptr::null_mut();
        let rc = unsafe {
            (self.api.prepare_v2)(
                self.db,
                c_sql.as_ptr(),
                c_sql.as_bytes().len() as c_int,
                &mut stmt,
                ptr::null_mut(),
            )
        };
        if rc != SQLITE_OK || stmt.is_null() {
            return Err(self.last_error(rc));
        }
        Ok(SqliteStmt { conn: self, stmt })
    }

    fn last_insert_rowid(&self) -> i64 {
        unsafe { (self.api.last_insert_rowid)(self.db) }
    }

    fn changes(&self) -> usize {
        unsafe { (self.api.changes)(self.db).max(0) as usize }
    }
}

impl Drop for SqliteConn {
    fn drop(&mut self) {
        if !self.db.is_null() {
            unsafe {
                (self.api.close)(self.db);
            }
        }
    }
}

struct SqliteStmt<'a> {
    conn: &'a SqliteConn,
    stmt: *mut Sqlite3Stmt,
}

impl<'a> SqliteStmt<'a> {
    fn reset(&mut self) {
        unsafe {
            (self.conn.api.reset)(self.stmt);
            (self.conn.api.clear_bindings)(self.stmt);
        }
    }

    fn bind_i64(&mut self, idx: c_int, val: i64) -> Result<(), SnapshotError> {
        let rc = unsafe { (self.conn.api.bind_int64)(self.stmt, idx, val) };
        if rc != SQLITE_OK {
            return Err(self.conn.last_error(rc));
        }
        Ok(())
    }

    fn bind_opt_i64(&mut self, idx: c_int, val: Option<i64>) -> Result<(), SnapshotError> {
        match val {
            Some(v) => self.bind_i64(idx, v),
            None => self.bind_null(idx),
        }
    }

    fn bind_null(&mut self, idx: c_int) -> Result<(), SnapshotError> {
        let rc = unsafe { (self.conn.api.bind_null)(self.stmt, idx) };
        if rc != SQLITE_OK {
            return Err(self.conn.last_error(rc));
        }
        Ok(())
    }

    fn bind_text(&mut self, idx: c_int, val: &str) -> Result<(), SnapshotError> {
        let rc = unsafe {
            (self.conn.api.bind_text)(
                self.stmt,
                idx,
                val.as_ptr() as *const c_char,
                val.len() as c_int,
                SQLITE_TRANSIENT,
            )
        };
        if rc != SQLITE_OK {
            return Err(self.conn.last_error(rc));
        }
        Ok(())
    }

    fn bind_opt_text(&mut self, idx: c_int, val: Option<&str>) -> Result<(), SnapshotError> {
        match val {
            Some(s) => self.bind_text(idx, s),
            None => self.bind_null(idx),
        }
    }

    fn bind_blob(&mut self, idx: c_int, bytes: &[u8]) -> Result<(), SnapshotError> {
        // Note: SQLite treats a NULL pointer in sqlite3_bind_blob with len 0 as SQL NULL!
        // To bind an empty Zero-Length BLOB (X''), we must pass a non-null pointer with len 0.
        let non_null_ptr = if bytes.is_empty() {
            b"" as *const u8 as *const c_void
        } else {
            bytes.as_ptr() as *const c_void
        };
        let rc = unsafe {
            (self.conn.api.bind_blob)(
                self.stmt,
                idx,
                non_null_ptr,
                bytes.len() as c_int,
                SQLITE_TRANSIENT,
            )
        };
        if rc != SQLITE_OK {
            return Err(self.conn.last_error(rc));
        }
        Ok(())
    }

    fn bind_opt_blob(&mut self, idx: c_int, bytes: Option<&[u8]>) -> Result<(), SnapshotError> {
        match bytes {
            Some(b) => self.bind_blob(idx, b),
            None => self.bind_null(idx),
        }
    }

    fn step(&mut self) -> Result<bool, SnapshotError> {
        let rc = unsafe { (self.conn.api.step)(self.stmt) };
        if rc == SQLITE_ROW {
            Ok(true)
        } else if rc == SQLITE_DONE {
            Ok(false)
        } else {
            Err(self.conn.last_error(rc))
        }
    }

    fn col_i64(&self, col: c_int) -> i64 {
        unsafe { (self.conn.api.column_int64)(self.stmt, col) }
    }

    fn col_opt_i64(&self, col: c_int) -> Option<i64> {
        const SQLITE_NULL: c_int = 5;
        let t = unsafe { (self.conn.api.column_type)(self.stmt, col) };
        if t == SQLITE_NULL {
            None
        } else {
            Some(self.col_i64(col))
        }
    }

    fn col_text(&self, col: c_int) -> String {
        unsafe {
            let ptr = (self.conn.api.column_text)(self.stmt, col);
            let len = (self.conn.api.column_bytes)(self.stmt, col);
            if ptr.is_null() || len <= 0 {
                String::new()
            } else {
                let slice = std::slice::from_raw_parts(ptr, len as usize);
                String::from_utf8_lossy(slice).into_owned()
            }
        }
    }

    fn col_opt_text(&self, col: c_int) -> Option<String> {
        const SQLITE_NULL: c_int = 5;
        let t = unsafe { (self.conn.api.column_type)(self.stmt, col) };
        if t == SQLITE_NULL {
            None
        } else {
            Some(self.col_text(col))
        }
    }

    fn col_blob(&self, col: c_int) -> Vec<u8> {
        unsafe {
            let ptr = (self.conn.api.column_blob)(self.stmt, col);
            let len = (self.conn.api.column_bytes)(self.stmt, col);
            if ptr.is_null() || len <= 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(ptr as *const u8, len as usize).to_vec()
            }
        }
    }
}

impl<'a> Drop for SqliteStmt<'a> {
    fn drop(&mut self) {
        if !self.stmt.is_null() {
            unsafe {
                (self.conn.api.finalize)(self.stmt);
            }
        }
    }
}

// ============================================================================
// SnapshotStore Public API
// ============================================================================

/// Local SQLite-backed persistence store for DiskGarden scans (`SCHEMA_V1.sql`).
#[derive(Debug, Clone)]
pub struct SnapshotStore {
    db_path: PathBuf,
}

impl SnapshotStore {
    /// Opens or creates a SQLite snapshot database at `db_path` and initializes Schema v1.
    pub fn open<P: AsRef<Path>>(db_path: P) -> Result<Self, SnapshotError> {
        let db_path = db_path.as_ref().to_path_buf();
        let conn = SqliteConn::open(&db_path)?;
        conn.exec_batch(SCHEMA_V1_SQL)?;
        conn.exec_batch(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (1, datetime('now'));
             UPDATE snapshots SET status = 'interrupted' WHERE status = 'saving';",
        )?;
        Ok(Self { db_path })
    }

    /// Opens the default user-local snapshot store under `$XDG_DATA_HOME/disk-garden/snapshots.db`
    /// (or `~/.local/share/disk-garden/snapshots.db`).
    pub fn open_default() -> Result<Self, SnapshotError> {
        let base = if let Some(xdg) = env::var_os("XDG_DATA_HOME") {
            PathBuf::from(xdg)
        } else if let Some(home) = env::var_os("HOME") {
            PathBuf::from(home).join(".local/share")
        } else {
            env::temp_dir()
        };
        let path = base.join("disk-garden").join("snapshots.db");
        Self::open(path)
    }

    /// Returns the filesystem path of the SQLite database.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Saves a completed or partial [`ScanResult`] as a local snapshot.
    ///
    /// By specification (Section 4.2), cancelled or failed scans cannot be saved as valid baseline
    /// snapshots (`SnapshotError::IncompleteScan`).
    pub fn save_snapshot(
        &self,
        scan: &ScanResult,
        label: Option<&str>,
        cross_mounts: bool,
        follow_symlinks: bool,
    ) -> Result<SnapshotHeader, SnapshotError> {
        if scan.status == ScanStatus::Cancelled || scan.status == ScanStatus::Failed {
            return Err(SnapshotError::IncompleteScan(scan.status));
        }

        let conn = SqliteConn::open(&self.db_path)?;
        conn.exec_batch("PRAGMA foreign_keys = ON; BEGIN IMMEDIATE TRANSACTION;")?;

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(scan.started_at_unix_ms);

        let root_dev = scan
            .root_id
            .and_then(|id| scan.get(id))
            .map(|n| n.dev)
            .unwrap_or(0);

        let final_status_str = match scan.status {
            ScanStatus::Completed => "completed",
            ScanStatus::Partial => "partial",
            ScanStatus::Cancelled => "cancelled",
            ScanStatus::Failed => "interrupted",
        };

        // 1. Insert snapshot row in 'saving' state first
        {
            let mut ins_snap = conn.prepare(
                "INSERT INTO snapshots (
                    schema_version, root_path_raw, root_path_display, label,
                    created_at_unix_ms, scan_duration_ms, status, root_dev,
                    cross_mounts, follow_symlinks, total_apparent_bytes, total_allocated_bytes,
                    total_files_count, total_dirs_count, total_symlinks_count,
                    hardlink_saved_bytes, unreadable_dirs_count, io_errors_count
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'saving', ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            )?;
            ins_snap.bind_i64(1, 1)?;
            ins_snap.bind_blob(2, &scan.root_path_raw)?;
            ins_snap.bind_text(3, &scan.root_path_display)?;
            ins_snap.bind_opt_text(4, label.map(str::trim).filter(|s| !s.is_empty()))?;
            ins_snap.bind_i64(5, now_ms)?;
            ins_snap.bind_i64(6, scan.duration_ms as i64)?;
            ins_snap.bind_i64(7, root_dev as i64)?;
            ins_snap.bind_i64(8, i64::from(cross_mounts))?;
            ins_snap.bind_i64(9, i64::from(follow_symlinks))?;
            ins_snap.bind_i64(10, scan.total_apparent_bytes as i64)?;
            ins_snap.bind_i64(11, scan.total_allocated_bytes as i64)?;
            ins_snap.bind_i64(12, scan.total_files_count as i64)?;
            ins_snap.bind_i64(13, scan.total_dirs_count as i64)?;
            ins_snap.bind_i64(14, scan.total_symlinks_count as i64)?;
            ins_snap.bind_i64(15, scan.hardlink_saved_allocated_bytes as i64)?;
            ins_snap.bind_i64(16, scan.unreadable_dirs_count as i64)?;
            ins_snap.bind_i64(17, scan.io_errors_count as i64)?;
            ins_snap.step()?;
        }

        let snapshot_id = conn.last_insert_rowid();

        // 2. Insert all entries with a reused prepared statement
        {
            let mut ins_entry = conn.prepare(
                "INSERT INTO snapshot_entries (
                    snapshot_id, rel_path_raw, rel_path_display, parent_rel_path_raw,
                    name_display, is_valid_utf8, depth, entry_type,
                    apparent_bytes, allocated_bytes, subtree_apparent_bytes, subtree_allocated_bytes,
                    subtree_files_count, subtree_dirs_count, subtree_error_count,
                    dev, ino, nlink, is_hardlink_duplicate, mtime_sec, mtime_nsec,
                    access_status, error_message
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                    ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                    ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23
                )",
            )?;

            for node in &scan.entries {
                ins_entry.reset();
                let parent_rel_raw = node
                    .parent_id
                    .and_then(|pid| scan.get(pid))
                    .map(|p| p.rel_path_raw.as_slice());

                ins_entry.bind_i64(1, snapshot_id)?;
                ins_entry.bind_blob(2, &node.rel_path_raw)?;
                ins_entry.bind_text(3, &node.rel_path_display)?;
                ins_entry.bind_opt_blob(4, parent_rel_raw)?;
                ins_entry.bind_text(5, &node.name_display)?;
                ins_entry.bind_i64(6, i64::from(node.is_valid_utf8))?;
                ins_entry.bind_i64(7, i64::from(node.depth))?;
                ins_entry.bind_text(8, entry_type_to_db(node.entry_type))?;
                ins_entry.bind_i64(9, node.apparent_bytes as i64)?;
                ins_entry.bind_i64(10, node.allocated_bytes as i64)?;
                ins_entry.bind_i64(11, node.subtree_apparent_bytes as i64)?;
                ins_entry.bind_i64(12, node.subtree_allocated_bytes as i64)?;
                ins_entry.bind_i64(13, node.subtree_files_count as i64)?;
                ins_entry.bind_i64(14, node.subtree_dirs_count as i64)?;
                ins_entry.bind_i64(15, node.subtree_error_count as i64)?;
                ins_entry.bind_i64(16, node.dev as i64)?;
                ins_entry.bind_i64(17, node.ino as i64)?;
                ins_entry.bind_i64(18, node.nlink as i64)?;
                ins_entry.bind_i64(19, i64::from(node.is_hardlink_duplicate))?;
                ins_entry.bind_opt_i64(20, node.mtime_sec)?;
                ins_entry.bind_opt_i64(21, node.mtime_nsec.map(i64::from))?;
                ins_entry.bind_text(22, access_status_to_db(node.access_status))?;
                ins_entry.bind_opt_text(23, node.error_message.as_deref())?;
                ins_entry.step()?;
            }
        }

        // 3. Transition status from 'saving' to final status and commit
        {
            let mut upd = conn.prepare("UPDATE snapshots SET status = ?1 WHERE id = ?2")?;
            upd.bind_text(1, final_status_str)?;
            upd.bind_i64(2, snapshot_id)?;
            upd.step()?;
        }

        conn.exec_batch("COMMIT;")?;

        Ok(SnapshotHeader {
            id: snapshot_id,
            schema_version: 1,
            root_path_raw: scan.root_path_raw.clone(),
            root_path_display: scan.root_path_display.clone(),
            label: label
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from),
            created_at_unix_ms: now_ms,
            scan_duration_ms: scan.duration_ms,
            status: scan.status,
            root_dev,
            cross_mounts,
            follow_symlinks,
            total_apparent_bytes: scan.total_apparent_bytes,
            total_allocated_bytes: scan.total_allocated_bytes,
            total_files_count: scan.total_files_count,
            total_dirs_count: scan.total_dirs_count,
            total_symlinks_count: scan.total_symlinks_count,
            hardlink_saved_bytes: scan.hardlink_saved_allocated_bytes,
            unreadable_dirs_count: scan.unreadable_dirs_count,
            io_errors_count: scan.io_errors_count,
        })
    }

    /// Lists saved snapshots ordered from newest to oldest (`created_at_unix_ms DESC`).
    /// If `root_path_filter` is provided, filters to snapshots matching that root directory.
    pub fn list_snapshots(
        &self,
        root_path_filter: Option<&Path>,
    ) -> Result<Vec<SnapshotHeader>, SnapshotError> {
        let conn = SqliteConn::open(&self.db_path)?;
        let filter_raw = root_path_filter.map(|p| {
            let canon = fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            canon.as_os_str().as_bytes().to_vec()
        });

        let sql = if filter_raw.is_some() {
            "SELECT id, schema_version, root_path_raw, root_path_display, label,
                    created_at_unix_ms, scan_duration_ms, status, root_dev,
                    cross_mounts, follow_symlinks, total_apparent_bytes, total_allocated_bytes,
                    total_files_count, total_dirs_count, total_symlinks_count,
                    hardlink_saved_bytes, unreadable_dirs_count, io_errors_count
             FROM snapshots
             WHERE status IN ('completed', 'partial') AND root_path_raw = ?1
             ORDER BY created_at_unix_ms DESC, id DESC"
        } else {
            "SELECT id, schema_version, root_path_raw, root_path_display, label,
                    created_at_unix_ms, scan_duration_ms, status, root_dev,
                    cross_mounts, follow_symlinks, total_apparent_bytes, total_allocated_bytes,
                    total_files_count, total_dirs_count, total_symlinks_count,
                    hardlink_saved_bytes, unreadable_dirs_count, io_errors_count
             FROM snapshots
             WHERE status IN ('completed', 'partial')
             ORDER BY created_at_unix_ms DESC, id DESC"
        };

        let mut stmt = conn.prepare(sql)?;
        if let Some(ref raw) = filter_raw {
            stmt.bind_blob(1, raw)?;
        }

        let mut list = Vec::new();
        while stmt.step()? {
            list.push(read_snapshot_header_row(&stmt));
        }
        Ok(list)
    }

    /// Loads a saved snapshot header and reconstructs its full [`ScanResult`] tree from SQLite.
    pub fn load_snapshot(&self, snapshot_id: i64) -> Result<StoredSnapshot, SnapshotError> {
        let conn = SqliteConn::open(&self.db_path)?;

        let header = {
            let mut stmt = conn.prepare(
                "SELECT id, schema_version, root_path_raw, root_path_display, label,
                        created_at_unix_ms, scan_duration_ms, status, root_dev,
                        cross_mounts, follow_symlinks, total_apparent_bytes, total_allocated_bytes,
                        total_files_count, total_dirs_count, total_symlinks_count,
                        hardlink_saved_bytes, unreadable_dirs_count, io_errors_count
                 FROM snapshots
                 WHERE id = ?1",
            )?;
            stmt.bind_i64(1, snapshot_id)?;
            if !stmt.step()? {
                return Err(SnapshotError::NotFound(snapshot_id));
            }
            read_snapshot_header_row(&stmt)
        };

        let mut entries = Vec::new();
        let mut parent_raws: Vec<Option<Vec<u8>>> = Vec::new();
        let mut path_to_id: HashMap<Vec<u8>, EntryId> = HashMap::new();

        {
            let mut stmt = conn.prepare(
                "SELECT rel_path_raw, rel_path_display, parent_rel_path_raw,
                        name_display, is_valid_utf8, depth, entry_type,
                        apparent_bytes, allocated_bytes, subtree_apparent_bytes, subtree_allocated_bytes,
                        subtree_files_count, subtree_dirs_count, subtree_error_count,
                        dev, ino, nlink, is_hardlink_duplicate, mtime_sec, mtime_nsec,
                        access_status, error_message
                 FROM snapshot_entries
                 WHERE snapshot_id = ?1
                 ORDER BY depth ASC, id ASC",
            )?;
            stmt.bind_i64(1, snapshot_id)?;

            while stmt.step()? {
                let id = EntryId(entries.len());
                let rel_path_raw = stmt.col_blob(0);
                let rel_path_display = stmt.col_text(1);
                let parent_raw = {
                    const SQLITE_NULL: c_int = 5;
                    let t = unsafe { (conn.api.column_type)(stmt.stmt, 2) };
                    if t == SQLITE_NULL {
                        None
                    } else {
                        Some(stmt.col_blob(2))
                    }
                };
                let name_display = stmt.col_text(3);
                let is_valid_utf8 = stmt.col_i64(4) != 0;
                let depth = stmt.col_i64(5).max(0) as u32;
                let entry_type = db_to_entry_type(&stmt.col_text(6));
                let apparent_bytes = stmt.col_i64(7).max(0) as u64;
                let allocated_bytes = stmt.col_i64(8).max(0) as u64;
                let subtree_apparent_bytes = stmt.col_i64(9).max(0) as u64;
                let subtree_allocated_bytes = stmt.col_i64(10).max(0) as u64;
                let subtree_files_count = stmt.col_i64(11).max(0) as u64;
                let subtree_dirs_count = stmt.col_i64(12).max(0) as u64;
                let subtree_error_count = stmt.col_i64(13).max(0) as u64;
                let dev = stmt.col_i64(14) as u64;
                let ino = stmt.col_i64(15) as u64;
                let nlink = stmt.col_i64(16).max(1) as u64;
                let is_hardlink_duplicate = stmt.col_i64(17) != 0;
                let mtime_sec = stmt.col_opt_i64(18);
                let mtime_nsec = stmt.col_opt_i64(19).map(|v| v.max(0) as u32);
                let access_status = db_to_access_status(&stmt.col_text(20));
                let error_message = stmt.col_opt_text(21);

                path_to_id.insert(rel_path_raw.clone(), id);
                parent_raws.push(parent_raw);

                entries.push(EntryNode {
                    id,
                    parent_id: None,
                    children_ids: Vec::new(),
                    name_raw: Vec::new(),
                    name_display,
                    rel_path_raw,
                    rel_path_display,
                    is_valid_utf8,
                    depth,
                    entry_type,
                    apparent_bytes,
                    allocated_bytes,
                    subtree_apparent_bytes,
                    subtree_allocated_bytes,
                    direct_files_apparent_bytes: 0,
                    direct_files_allocated_bytes: 0,
                    subtree_files_count,
                    subtree_dirs_count,
                    subtree_symlinks_count: 0,
                    subtree_error_count,
                    dev,
                    ino,
                    nlink,
                    is_hardlink_duplicate,
                    primary_hardlink_id: None,
                    shared_link_paths_in_scan: 1,
                    mtime_sec,
                    mtime_nsec,
                    access_status,
                    error_message,
                });
            }
        }

        // Reconnect parent_id and children_ids
        let mut root_id = None;
        for idx in 0..entries.len() {
            if entries[idx].depth == 0 && entries[idx].rel_path_raw.is_empty() {
                root_id = Some(EntryId(idx));
            }
            if let Some(ref p_raw) = parent_raws[idx] {
                if let Some(&pid) = path_to_id.get(p_raw) {
                    entries[idx].parent_id = Some(pid);
                    entries[pid.0].children_ids.push(EntryId(idx));
                }
            }
        }

        let permission_denied_count = entries
            .iter()
            .filter(|e| e.access_status == AccessStatus::PermissionDenied)
            .count() as u64;
        let vanished_count = entries
            .iter()
            .filter(|e| e.access_status == AccessStatus::Vanished)
            .count() as u64;
        let excluded_mounts_count = entries
            .iter()
            .filter(|e| e.access_status == AccessStatus::ExcludedMount)
            .count() as u64;
        let hardlink_duplicate_files_count =
            entries.iter().filter(|e| e.is_hardlink_duplicate).count() as u64;

        let mut cfg = ScanConfig::new(raw_bytes_to_path_buf(&header.root_path_raw));
        cfg.cross_mounts = header.cross_mounts;
        cfg.follow_symlinks = header.follow_symlinks;

        let scan = ScanResult {
            root_path_raw: header.root_path_raw.clone(),
            root_path_display: header.root_path_display.clone(),
            root_dev: header.root_dev,
            config: cfg,
            status: header.status,
            started_at_unix_ms: header.created_at_unix_ms,
            duration_ms: header.scan_duration_ms,
            entries,
            root_id,
            total_apparent_bytes: header.total_apparent_bytes,
            total_allocated_bytes: header.total_allocated_bytes,
            total_files_count: header.total_files_count,
            total_dirs_count: header.total_dirs_count,
            total_symlinks_count: header.total_symlinks_count,
            hardlink_duplicate_files_count,
            hardlink_saved_allocated_bytes: header.hardlink_saved_bytes,
            hardlink_saved_apparent_bytes: 0,
            unreadable_dirs_count: header.unreadable_dirs_count,
            permission_denied_count,
            io_errors_count: header.io_errors_count,
            vanished_count,
            excluded_mounts_count,
            symlink_cycles_count: 0,
            root_error_message: None,
        };

        Ok(StoredSnapshot { header, scan })
    }

    /// Deletes a snapshot and all its associated entries (`ON DELETE CASCADE`).
    pub fn delete_snapshot(&self, snapshot_id: i64) -> Result<bool, SnapshotError> {
        let conn = SqliteConn::open(&self.db_path)?;
        conn.exec_batch("PRAGMA foreign_keys = ON;")?;
        let mut stmt = conn.prepare("DELETE FROM snapshots WHERE id = ?1")?;
        stmt.bind_i64(1, snapshot_id)?;
        stmt.step()?;
        Ok(conn.changes() > 0)
    }
}

fn read_snapshot_header_row(stmt: &SqliteStmt<'_>) -> SnapshotHeader {
    SnapshotHeader {
        id: stmt.col_i64(0),
        schema_version: stmt.col_i64(1) as i32,
        root_path_raw: stmt.col_blob(2),
        root_path_display: stmt.col_text(3),
        label: stmt.col_opt_text(4),
        created_at_unix_ms: stmt.col_i64(5),
        scan_duration_ms: stmt.col_i64(6).max(0) as u64,
        status: db_to_scan_status(&stmt.col_text(7)),
        root_dev: stmt.col_i64(8) as u64,
        cross_mounts: stmt.col_i64(9) != 0,
        follow_symlinks: stmt.col_i64(10) != 0,
        total_apparent_bytes: stmt.col_i64(11).max(0) as u64,
        total_allocated_bytes: stmt.col_i64(12).max(0) as u64,
        total_files_count: stmt.col_i64(13).max(0) as u64,
        total_dirs_count: stmt.col_i64(14).max(0) as u64,
        total_symlinks_count: stmt.col_i64(15).max(0) as u64,
        hardlink_saved_bytes: stmt.col_i64(16).max(0) as u64,
        unreadable_dirs_count: stmt.col_i64(17).max(0) as u64,
        io_errors_count: stmt.col_i64(18).max(0) as u64,
    }
}

fn entry_type_to_db(t: EntryType) -> &'static str {
    match t {
        EntryType::Directory => "dir",
        EntryType::File => "file",
        EntryType::Symlink => "symlink",
        EntryType::Other => "other",
    }
}

fn db_to_entry_type(s: &str) -> EntryType {
    match s {
        "dir" => EntryType::Directory,
        "file" => EntryType::File,
        "symlink" => EntryType::Symlink,
        _ => EntryType::Other,
    }
}

fn access_status_to_db(a: AccessStatus) -> &'static str {
    match a {
        AccessStatus::Ok => "ok",
        AccessStatus::PermissionDenied => "permission_denied",
        AccessStatus::IoError => "io_error",
        AccessStatus::Vanished => "vanished",
        AccessStatus::ExcludedMount => "excluded_mount",
        AccessStatus::SymlinkSkipped | AccessStatus::SymlinkCycleSkipped => "symlink_skipped",
    }
}

fn db_to_access_status(s: &str) -> AccessStatus {
    match s {
        "ok" => AccessStatus::Ok,
        "permission_denied" => AccessStatus::PermissionDenied,
        "io_error" => AccessStatus::IoError,
        "vanished" => AccessStatus::Vanished,
        "excluded_mount" => AccessStatus::ExcludedMount,
        "symlink_skipped" => AccessStatus::SymlinkSkipped,
        _ => AccessStatus::Ok,
    }
}

fn db_to_scan_status(s: &str) -> ScanStatus {
    match s {
        "completed" => ScanStatus::Completed,
        "partial" => ScanStatus::Partial,
        "cancelled" => ScanStatus::Cancelled,
        _ => ScanStatus::Failed,
    }
}
