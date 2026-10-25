// Compiles the Windows resource script (icon, version info, manifest) with the
// SDK's rc.exe and links the result. No resource crate is needed.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=resources/app.rc");
    println!("cargo:rerun-if-changed=resources/app.manifest");
    println!("cargo:rerun-if-changed=resources/assets/codex-monitor-hud.ico");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let res = out.join("app.res");
    let rc = find_rc().unwrap_or_else(|| PathBuf::from("rc.exe"));
    let status = Command::new(&rc)
        .args(["/nologo", "/fo"])
        .arg(&res)
        .arg("resources/app.rc")
        .status()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", rc.display()));
    assert!(status.success(), "rc.exe failed");
    println!("cargo:rustc-link-arg-bins={}", res.display());
}

fn find_rc() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("rc.exe");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    let kits = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut versions: Vec<_> = std::fs::read_dir(&kits).ok()?.flatten().map(|e| e.path()).collect();
    versions.sort();
    versions.into_iter().rev().map(|v| v.join("x64").join("rc.exe")).find(|p| p.exists())
}
