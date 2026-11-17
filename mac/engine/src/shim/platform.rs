//! The parts of src-tauri/src/platform.rs the engines use, for macOS (that file needs Tauri).

use std::path::PathBuf;

/// ~/Library/Application Support/Rotae, the same folder the app keeps its state in.
pub fn data_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Library/Application Support/Rotae")
}

/// A program on PATH.
pub fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(name)).find(|p| p.is_file()))
}

/// Arial ships with every macOS and covers Vietnamese; Helvetica is a .ttc, which not every
/// renderer loads.
pub fn system_font(bold: bool) -> Option<PathBuf> {
    let dir = PathBuf::from("/System/Library/Fonts/Supplemental");
    let names: &[&str] = if bold { &["Arial Bold.ttf", "Arial.ttf"] } else { &["Arial.ttf"] };
    names.iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

/// Only shell.rs's Linux file-manager menus ask for this; the Mac app adds Finder's menu itself.
#[allow(dead_code)]
pub fn launcher_path() -> std::io::Result<String> {
    Err(std::io::Error::other("file manager menus are added by the Mac app"))
}
