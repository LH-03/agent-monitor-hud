//! Directory change notifications (ReadDirectoryChangesW, overlapped) on one
//! background thread. Changes are coalesced and forwarded as path lists; an
//! overflow is reported so the engine can fall back to a bounded rescan.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_CREATION,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_INFORMATION,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadDirectoryChangesW,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects};
use windows::core::HSTRING;

pub enum WatchEvent {
    Paths(Vec<PathBuf>),
    Overflow,
}

struct Watch {
    root: PathBuf,
    recursive: bool,
    handle: HANDLE,
    event: HANDLE,
    overlapped: Box<OVERLAPPED>,
    buffer: Box<[u32; 16 * 1024]>,
    pending: bool,
}

impl Watch {
    fn open(root: &Path, recursive: bool) -> Option<Watch> {
        unsafe {
            let handle = CreateFileW(
                &HSTRING::from(root),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                None,
            )
            .ok()?;
            let event = CreateEventW(None, true, false, None).ok()?;
            let mut overlapped = Box::new(OVERLAPPED::default());
            overlapped.hEvent = event;
            Some(Watch {
                root: root.to_path_buf(),
                recursive,
                handle,
                event,
                overlapped,
                buffer: Box::new([0u32; 16 * 1024]),
                pending: false,
            })
        }
    }

    fn arm(&mut self) -> bool {
        unsafe {
            self.pending = ReadDirectoryChangesW(
                self.handle,
                self.buffer.as_mut_ptr() as *mut _,
                (self.buffer.len() * 4) as u32,
                self.recursive,
                FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_CREATION,
                None,
                Some(&mut *self.overlapped),
                None,
            )
            .is_ok();
            self.pending
        }
    }

    /// Returns `None` on overflow (zero bytes), else the changed paths.
    fn collect(&mut self) -> Option<Vec<PathBuf>> {
        let mut bytes = 0u32;
        unsafe {
            if GetOverlappedResult(self.handle, &*self.overlapped, &mut bytes, false).is_err() || bytes == 0 {
                return None;
            }
        }
        let base = self.buffer.as_ptr() as *const u8;
        let mut offset = 0usize;
        let mut out = Vec::new();
        loop {
            unsafe {
                let info = &*(base.add(offset) as *const FILE_NOTIFY_INFORMATION);
                let len = info.FileNameLength as usize / 2;
                let name = std::slice::from_raw_parts(info.FileName.as_ptr(), len);
                out.push(self.root.join(String::from_utf16_lossy(name)));
                if info.NextEntryOffset == 0 {
                    break;
                }
                offset += info.NextEntryOffset as usize;
            }
        }
        Some(out)
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        unsafe {
            let _ = CancelIoEx(self.handle, Some(&*self.overlapped));
            let mut bytes = 0;
            let _ = GetOverlappedResult(self.handle, &*self.overlapped, &mut bytes, true);
            let _ = CloseHandle(self.handle);
            let _ = CloseHandle(self.event);
        }
    }
}

/// Starts watching `roots` (path, recursive). Missing roots are retried every
/// 30 s so a later first run of Codex/Claude is picked up.
pub fn spawn(roots: Vec<(PathBuf, bool)>, stop: Arc<AtomicBool>, sink: impl Fn(WatchEvent) + Send + 'static) {
    std::thread::Builder::new()
        .name("hud-watch".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            let mut watches: Vec<Watch> = Vec::new();
            let mut missing: Vec<(PathBuf, bool)> = Vec::new();
            for (root, recursive) in roots {
                match Watch::open(&root, recursive) {
                    Some(mut w) => {
                        if w.arm() { watches.push(w) } else { missing.push((root, recursive)) }
                    }
                    None => missing.push((root, recursive)),
                }
            }
            let mut retry_at = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !stop.load(Ordering::Relaxed) {
                if !missing.is_empty() && std::time::Instant::now() >= retry_at {
                    retry_at = std::time::Instant::now() + std::time::Duration::from_secs(30);
                    let mut still = Vec::new();
                    for (root, recursive) in missing.drain(..) {
                        match Watch::open(&root, recursive) {
                            Some(mut w) => {
                                if w.arm() {
                                    watches.push(w);
                                    sink(WatchEvent::Overflow);
                                } else {
                                    still.push((root, recursive));
                                }
                            }
                            None => still.push((root, recursive)),
                        }
                    }
                    missing = still;
                }
                if watches.is_empty() {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    continue;
                }
                let events: Vec<HANDLE> = watches.iter().map(|w| w.event).collect();
                let result = unsafe { WaitForMultipleObjects(&events, false, 1000) };
                if result == WAIT_TIMEOUT {
                    continue;
                }
                let index = result.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
                if index >= watches.len() {
                    continue;
                }
                let mut changed: Vec<PathBuf> = Vec::new();
                let mut overflow = false;
                // Drain every signalled watch, then debounce briefly.
                for w in watches.iter_mut() {
                    if unsafe { windows::Win32::System::Threading::WaitForSingleObject(w.event, 0) } == WAIT_OBJECT_0 {
                        unsafe {
                            let _ = windows::Win32::System::Threading::ResetEvent(w.event);
                        }
                        match w.collect() {
                            Some(paths) => changed.extend(paths),
                            None => overflow = true,
                        }
                        if !w.arm() {
                            overflow = true;
                        }
                    }
                }
                if overflow {
                    sink(WatchEvent::Overflow);
                }
                if !changed.is_empty() {
                    changed.sort();
                    changed.dedup();
                    sink(WatchEvent::Paths(changed));
                }
            }
        })
        .expect("watch thread");
}
