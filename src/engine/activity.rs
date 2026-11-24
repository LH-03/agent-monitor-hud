//! Codex `state_5.sqlite` liveness hints (thread id, rollout path, updated
//! time only) through the Windows system SQLite (`winsqlite3.dll`), opened
//! read-only. Any failure degrades to file-based discovery.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::sync::OnceLock;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows::core::{PCSTR, w};

#[derive(Clone, Debug)]
pub struct Activity {
    pub session_id: String,
    pub rollout_path: String,
    pub updated_ms: i64,
}

type Open = unsafe extern "C" fn(*const c_char, *mut *mut c_void, c_int, *const c_char) -> c_int;
type Busy = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type Prepare = unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut *mut c_void, *mut *const c_char) -> c_int;
type Step = unsafe extern "C" fn(*mut c_void) -> c_int;
type ColText = unsafe extern "C" fn(*mut c_void, c_int) -> *const u8;
type ColBlob = unsafe extern "C" fn(*mut c_void, c_int) -> *const u8;
type ColBytes = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type ColInt64 = unsafe extern "C" fn(*mut c_void, c_int) -> i64;
type Finalize = unsafe extern "C" fn(*mut c_void) -> c_int;
type Close = unsafe extern "C" fn(*mut c_void) -> c_int;

struct Api {
    open: Open,
    busy: Busy,
    prepare: Prepare,
    step: Step,
    text: ColText,
    int64: ColInt64,
    blob: ColBlob,
    bytes: ColBytes,
    finalize: Finalize,
    close: Close,
}

fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let module = LoadLibraryExW(w!("winsqlite3.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok()?;
        macro_rules! sym {
            ($name:literal) => {
                std::mem::transmute(GetProcAddress(module, PCSTR(concat!($name, "\0").as_ptr()))?)
            };
        }
        Some(Api {
            open: sym!("sqlite3_open_v2"),
            busy: sym!("sqlite3_busy_timeout"),
            prepare: sym!("sqlite3_prepare_v2"),
            step: sym!("sqlite3_step"),
            text: sym!("sqlite3_column_text"),
            int64: sym!("sqlite3_column_int64"),
            blob: sym!("sqlite3_column_blob"),
            bytes: sym!("sqlite3_column_bytes"),
            finalize: sym!("sqlite3_finalize"),
            close: sym!("sqlite3_close_v2"),
        })
    })
    .as_ref()
}

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;
const OPEN_READONLY: c_int = 0x1;

pub(super) struct Database {
    api: &'static Api,
    handle: *mut c_void,
}

impl Database {
    pub fn open(path: &Path) -> Option<Self> {
        let api = api()?;
        let path = CString::new(path.to_str()?).ok()?;
        let mut db = Self { api, handle: std::ptr::null_mut() };
        unsafe {
            if (api.open)(path.as_ptr(), &mut db.handle, OPEN_READONLY, std::ptr::null()) != SQLITE_OK {
                return None;
            }
            (api.busy)(db.handle, 50);
        }
        Some(db)
    }

    pub fn query<T>(&self, sql: &str, mut read: impl FnMut(&Row<'_>) -> T) -> Option<Vec<T>> {
        let sql = CString::new(sql).ok()?;
        let mut stmt = Statement { db: self, handle: std::ptr::null_mut() };
        unsafe {
            if (self.api.prepare)(self.handle, sql.as_ptr(), -1, &mut stmt.handle, std::ptr::null_mut()) != SQLITE_OK
                || stmt.handle.is_null()
            {
                return None;
            }
            let mut rows = Vec::new();
            loop {
                match (self.api.step)(stmt.handle) {
                    SQLITE_ROW => rows.push(read(&Row(&stmt))),
                    SQLITE_DONE => return Some(rows),
                    _ => return None,
                }
            }
        }
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.api.close)(self.handle) };
        }
    }
}

struct Statement<'a> {
    db: &'a Database,
    handle: *mut c_void,
}

impl Drop for Statement<'_> {
    fn drop(&mut self) {
        unsafe { (self.db.api.finalize)(self.handle) };
    }
}

pub(super) struct Row<'a>(&'a Statement<'a>);

impl Row<'_> {
    pub fn text(&self, col: c_int) -> String {
        let ptr = unsafe { (self.0.db.api.text)(self.0.handle, col) };
        if ptr.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(ptr.cast()) }.to_string_lossy().into_owned()
        }
    }

    pub fn int(&self, col: c_int) -> i64 {
        unsafe { (self.0.db.api.int64)(self.0.handle, col) }
    }

    // Give the slice only to the decoder. SQLite owns it until the next
    // step/conversion, so no borrowed blob can escape this callback.
    pub fn blob<T>(&self, col: c_int, decode: impl FnOnce(&[u8]) -> T) -> T {
        let api = self.0.db.api;
        let stmt = self.0.handle;
        unsafe {
            let ptr = (api.blob)(stmt, col);
            let len = (api.bytes)(stmt, col);
            decode(if ptr.is_null() || len <= 0 { &[] } else { std::slice::from_raw_parts(ptr, len as usize) })
        }
    }
}

pub fn recent_user_sessions(database: &Path, cutoff_ms: i64, limit: usize) -> Vec<Activity> {
    let Some(db) = Database::open(database) else { return Vec::new() };
    let sql = format!(
        "SELECT id, rollout_path, updated_at_ms FROM threads WHERE archived = 0 AND thread_source = 'user' \
         AND updated_at_ms >= {cutoff_ms} ORDER BY updated_at_ms DESC LIMIT {}",
        limit.clamp(1, 256)
    );
    db.query(&sql, |row| Activity { session_id: row.text(0), rollout_path: row.text(1), updated_ms: row.int(2) })
        .unwrap_or_default()
        .into_iter()
        .filter(|r| !r.session_id.is_empty() && !r.rollout_path.is_empty() && r.updated_ms > 0)
        .collect()
}

/// `\\?\C:\...` → `C:\...`, `\\?\UNC\server\share` → `\\server\share`.
pub fn normalize_extended(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path.to_string()
    }
}
