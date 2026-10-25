//! File helpers: atomic replacement, shared-read opening and cheap metadata.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use windows::Win32::Storage::FileSystem::{
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileAttributesExW, GetFileExInfoStandard,
    WIN32_FILE_ATTRIBUTE_DATA,
};
use windows::core::HSTRING;

use super::time::filetime_to_ms;

/// Writes `content` to a temporary sibling and renames it over `path`, so
/// readers never observe a half-written document.
pub fn write_atomic(path: &Path, content: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "{}.{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("dat"),
        std::process::id()
    ));
    fs::write(&tmp, content)?;
    let result = fs::rename(&tmp, path);
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Opens a log file for reading while its writer keeps appending, renaming or
/// deleting it (FILE_SHARE_READ | WRITE | DELETE).
pub fn open_shared(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .open(path)
}

pub fn read_text(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileStamp {
    pub size: u64,
    pub write_ms: i64,
    pub create_ms: i64,
}

/// Size and timestamps straight from the file record. Unlike directory
/// enumeration this is current even while another process holds the file open.
pub fn stamp(path: &Path) -> Option<FileStamp> {
    let mut data = WIN32_FILE_ATTRIBUTE_DATA::default();
    unsafe {
        GetFileAttributesExW(&HSTRING::from(path), GetFileExInfoStandard, &mut data as *mut _ as *mut _).ok()?;
    }
    Some(FileStamp {
        size: ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64,
        write_ms: filetime_to_ms(data.ftLastWriteTime.dwLowDateTime, data.ftLastWriteTime.dwHighDateTime),
        create_ms: filetime_to_ms(data.ftCreationTime.dwLowDateTime, data.ftCreationTime.dwHighDateTime),
    })
}

/// True when the writer opened the file without sharing read access (a
/// sharing violation), which the old host treated as a liveness hint.
pub fn is_read_blocked(path: &Path) -> bool {
    match open_shared(path) {
        Ok(_) => false,
        Err(error) => matches!(error.raw_os_error(), Some(32) | Some(33)),
    }
}

pub fn remove_quietly(path: &Path) {
    let _ = fs::remove_file(path);
}

/// Last path component of a Windows or POSIX path, without trailing separators.
pub fn leaf_name(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches(['\\', '/']);
    match trimmed.rfind(['\\', '/']) {
        Some(index) if index + 1 < trimmed.len() => trimmed[index + 1..].to_string(),
        _ => trimmed.to_string(),
    }
}
