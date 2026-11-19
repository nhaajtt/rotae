//! Rotae's macOS work process. It runs one unit of work and exits, so cancelling is killing
//! its process group and a crash can only ever cost one file.
#![allow(dead_code)]

#[path = "../../../src-tauri/src/engines/mod.rs"]
mod engines;
#[path = "../../../src-tauri/src/naming.rs"]
mod naming;
#[path = "../../../src-tauri/src/share.rs"]
mod share;
#[cfg(test)]
#[allow(unused_imports)]
#[path = "../../../src-tauri/src/shell.rs"]
mod shell;

#[path = "shim/platform.rs"]
mod platform;
#[path = "shim/apps.rs"]
mod apps;
#[path = "shim/clipboard.rs"]
mod clipboard;
#[path = "shim/settings.rs"]
mod settings;
#[path = "shim/winrt.rs"]
mod winrt;

mod desk;
mod devkit;
mod ledger;
mod papers;
mod smart;
mod errors;
mod native;
mod proto;

fn main() {
    std::process::exit(proto::serve());
}

/// What src-tauri's lib.rs offers as `crate::log`: a line in ~/Library/Logs/Rotae/engine.log
/// (stdout carries the protocol, so nothing goes there).
pub fn log(msg: &str) {
    use std::io::Write;
    let dir = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Library/Logs/Rotae");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("engine.log")) {
        let _ = writeln!(f, "{msg}");
    }
}
