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

pub fn recent_user_sessions(database: &Path, cutoff_ms: i64, limit: usize) -> Vec<Activity> {
    let mut results = Vec::new();
    if !database.exists() {
        return results;
    }
    let Some(api) = api() else { return results };
    let Ok(path) = CString::new(database.to_string_lossy().as_bytes()) else { return results };
    let sql = format!(
        "SELECT id, rollout_path, updated_at_ms FROM threads WHERE archived = 0 AND thread_source = 'user' \
         AND updated_at_ms >= {cutoff_ms} ORDER BY updated_at_ms DESC LIMIT {}",
        limit.clamp(1, 256)
    );
    let Ok(sql) = CString::new(sql) else { return results };
    unsafe {
        let mut db: *mut c_void = std::ptr::null_mut();
        if (api.open)(path.as_ptr(), &mut db, OPEN_READONLY, std::ptr::null()) != SQLITE_OK || db.is_null() {
            if !db.is_null() {
                (api.close)(db);
            }
            return results;
        }
        (api.busy)(db, 75);
        let mut stmt: *mut c_void = std::ptr::null_mut();
        if (api.prepare)(db, sql.as_ptr(), -1, &mut stmt, std::ptr::null_mut()) == SQLITE_OK && !stmt.is_null() {
            loop {
                let step = (api.step)(stmt);
                if step == SQLITE_DONE {
                    break;
                }
                if step != SQLITE_ROW {
                    results.clear();
                    break;
                }
                let read = |col: c_int| -> String {
                    let ptr = (api.text)(stmt, col);
                    if ptr.is_null() { String::new() } else { CStr::from_ptr(ptr as *const c_char).to_string_lossy().into_owned() }
                };
                let session_id = read(0);
                let rollout_path = read(1);
                let updated_ms = (api.int64)(stmt, 2);
                if !session_id.is_empty() && !rollout_path.is_empty() && updated_ms > 0 {
                    results.push(Activity { session_id, rollout_path, updated_ms });
                }
            }
            (api.finalize)(stmt);
        }
        (api.close)(db);
    }
    results
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
