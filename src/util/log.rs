//! Tiny bounded diagnostic log (`<state>/hud.log`, rotated at 512 KiB).
//! Only HUD events and errors; never task content.

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

static PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init(path: PathBuf) {
    let _ = PATH.set(path);
}

pub fn write(message: impl AsRef<str>) {
    let Some(path) = PATH.get() else { return };
    if std::fs::metadata(path).is_ok_and(|m| m.len() > 512 * 1024) {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{} {}", super::time::local_iso(super::time::now_ms()), message.as_ref());
    }
}
