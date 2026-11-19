//! Installed tools the menu can hand a file to (editors, debuggers, disassemblers...).
//! Found once through Start menu and desktop shortcuts, so nothing has to be configured.

use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

#[derive(Serialize, Clone)]
pub struct App {
    pub id: &'static str,
    pub label: &'static str,
    /// File kinds the menu offers it for: text, exe, archive, pcap, any
    pub kinds: &'static [&'static str],
    #[serde(skip)]
    pub target: PathBuf,
}

/// id, label, kinds, shortcut-name prefixes (lowercase)
#[cfg(windows)]
const KNOWN: &[(&str, &str, &[&str], &[&str])] = &[
    ("vscode", "VS Code", &["text", "any"], &["visual studio code"]),
    ("cursor", "Cursor", &["text", "any"], &["cursor"]),
    ("sublime", "Sublime", &["text"], &["sublime text"]),
    ("notepadpp", "Notepad++", &["text"], &["notepad++"]),
    ("x64dbg", "x64dbg", &["exe"], &["x64dbg"]),
    ("x32dbg", "x32dbg", &[], &["x32dbg"]),
    ("ida", "IDA", &["exe"], &["ida pro", "ida free", "ida home", "ida64", "ida "]),
    ("ghidra", "Ghidra", &["exe"], &["ghidra"]),
    ("binja", "Binary Ninja", &["exe"], &["binary ninja"]),
    ("die", "DIE", &["exe"], &["die.exe", "detect it easy", "die "]),
    ("pebear", "PE-bear", &["exe"], &["pe-bear"]),
    ("dnspy", "dnSpy", &["exe"], &["dnspy"]),
    ("ilspy", "ILSpy", &["exe"], &["ilspy"]),
    ("hxd", "HxD", &["exe", "any"], &["hxd"]),
    ("wireshark", "Wireshark", &["pcap"], &["wireshark"]),
];

#[cfg(windows)]
const SKIP: &[&str] = &["uninstall", "help", "tutorial", "installer", "readme", "manual", "documentation", "website", "release notes"];

static APPS: OnceLock<Vec<App>> = OnceLock::new();

pub fn all() -> &'static [App] {
    APPS.get_or_init(scan)
}

#[cfg(windows)]
fn shortcut_dirs() -> Vec<PathBuf> {
    let env = |v: &str| std::env::var_os(v).map(PathBuf::from);
    let mut dirs = Vec::new();
    if let Some(p) = env("ProgramData") {
        dirs.push(p.join("Microsoft\\Windows\\Start Menu\\Programs"));
    }
    if let Some(p) = env("APPDATA") {
        dirs.push(p.join("Microsoft\\Windows\\Start Menu\\Programs"));
    }
    if let Some(p) = env("USERPROFILE") {
        dirs.push(p.join("Desktop"));
        dirs.push(p.join("OneDrive\\Desktop"));
    }
    if let Some(p) = env("PUBLIC") {
        dirs.push(p.join("Desktop"));
    }
    dirs
}

#[cfg(windows)]
fn walk(dir: &Path, depth: u32, found: &mut Vec<PathBuf>, wanted: &dyn Fn(&Path) -> bool) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                walk(&p, depth - 1, found, wanted);
            }
        } else if wanted(&p) {
            found.push(p);
        }
    }
}
/// Linux: id, label, kinds, program names on PATH (first found wins).
#[cfg(not(windows))]
const KNOWN_LINUX: &[(&str, &str, &[&str], &[&str])] = &[
    ("vscode", "VS Code", &["text", "any"], &["code", "codium"]),
    ("cursor", "Cursor", &["text", "any"], &["cursor"]),
    ("sublime", "Sublime", &["text"], &["subl"]),
    ("kate", "Kate", &["text"], &["kate"]),
    ("gedit", "Text Editor", &["text"], &["gnome-text-editor", "gedit"]),
    ("mousepad", "Mousepad", &["text"], &["mousepad"]),
    ("edb", "edb", &["exe"], &["edb"]),
    ("ida", "IDA", &["exe"], &["ida64", "ida"]),
    ("ghidra", "Ghidra", &["exe"], &["ghidra", "ghidraRun"]),
    ("binja", "Binary Ninja", &["exe"], &["binaryninja"]),
    ("cutter", "Cutter", &["exe"], &["cutter", "Cutter"]),
    ("iaito", "iaito", &["exe"], &["iaito"]),
    ("die", "DIE", &["exe"], &["die"]),
    ("imhex", "ImHex", &["exe", "any"], &["imhex"]),
    ("ghex", "GHex", &["exe", "any"], &["ghex"]),
    ("jadx", "jadx", &["apk"], &["jadx-gui"]),
    ("wireshark", "Wireshark", &["pcap"], &["wireshark"]),
];

#[cfg(not(windows))]
fn scan() -> Vec<App> {
    KNOWN_LINUX.iter().filter_map(|&(id, label, kinds, bins)| bins.iter().find_map(|b| crate::platform::which(b)).map(|target| App { id, label, kinds, target })).collect()
}

#[cfg(windows)]
fn scan() -> Vec<App> {
    let mut links = Vec::new();
    for d in shortcut_dirs() {
        walk(&d, 4, &mut links, &|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")));
    }
    let names: Vec<(String, PathBuf)> =
        links.into_iter().filter_map(|p| Some((p.file_stem()?.to_string_lossy().to_lowercase(), p))).filter(|(n, _)| !SKIP.iter().any(|s| n.contains(s))).collect();
    let mut apps = Vec::new();
    for &(id, label, kinds, prefixes) in KNOWN {
        let hit = names.iter().find(|(n, _)| prefixes.iter().any(|pre| n == pre.trim() || n.starts_with(pre)));
        let target = hit.map(|(_, p)| p.clone()).or_else(|| if id == "ghidra" { find_ghidra() } else { None });
        if let Some(target) = target {
            apps.push(App { id, label, kinds, target });
        }
    }
    apps
}

/// Ghidra ships as a zip without shortcuts; look for ghidraRun.bat in the usual places.
#[cfg(windows)]
fn find_ghidra() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from)?;
    let roots = [PathBuf::from("C:\\"), PathBuf::from("D:\\"), PathBuf::from("C:\\Tools"), home.clone(), home.join("Downloads"), home.join("Desktop"), home.join("tools")];
    for r in roots {
        let Ok(rd) = std::fs::read_dir(&r) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && e.file_name().to_string_lossy().to_lowercase().starts_with("ghidra") {
                let bat = p.join("ghidraRun.bat");
                if bat.is_file() {
                    return Some(bat);
                }
            }
        }
    }
    None
}

#[tauri::command]
pub async fn get_apps() -> Vec<App> {
    all().to_vec()
}

/// Opens the files in an installed tool. x64dbg switches to x32dbg for 32-bit programs.
#[tauri::command]
pub async fn open_with(app: String, paths: Vec<String>) -> Result<(), String> {
    let apps = all();
    let mut chosen = apps.iter().find(|a| a.id == app).ok_or("that app isn't installed")?;
    if app == "x64dbg" && paths.first().and_then(|p| crate::engines::files::pe_is_64(Path::new(p))) == Some(false) {
        if let Some(x32) = apps.iter().find(|a| a.id == "x32dbg") {
            chosen = x32;
        }
    }
    // a real file name never holds a quote; one here could add the app's own options
    if paths.iter().any(|p| p.contains('"') || p.chars().any(char::is_control) || !Path::new(p).exists()) {
        return Err("not a file".into());
    }
    // a trailing backslash (a drive root like C:\) would escape the closing quote: double it
    let args = paths.iter().map(|p| if p.ends_with('\\') { format!("\"{p}\\\"") } else { format!("\"{p}\"") }).collect::<Vec<_>>().join(" ");
    launch(&chosen.target, &args)
}

/// Starts an installed app with raw arguments.
pub fn launch_id(id: &str, args: &str) -> Result<(), String> {
    let app = all().iter().find(|a| a.id == id).ok_or("that app isn't installed")?;
    launch(&app.target, args)
}

#[cfg(windows)]
fn launch(target: &Path, args: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let w = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, file, params) = (w("open"), w(&target.display().to_string()), w(args));
    // ShellExecute resolves .lnk shortcuts and passes the arguments to their target
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), params.as_ptr(), std::ptr::null(), SW_SHOWNORMAL) };
    if (r as isize) <= 32 {
        return Err(format!("couldn't start {}", target.display()));
    }
    Ok(())
}

/// Linux: the arguments are split on spaces outside double quotes (paths never hold a quote,
/// see `open_with`) and passed straight to the program, never through a shell. The app keeps
/// running after Rotae quits.
#[cfg(not(windows))]
fn launch(target: &Path, args: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let mut argv = Vec::new();
    let (mut cur, mut quoted, mut any) = (String::new(), false, false);
    for c in args.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            ' ' if !quoted => {
                if any || !cur.is_empty() {
                    argv.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        argv.push(cur);
    }
    Command::new(target)
        .args(&argv)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't start {}: {e}", target.display()))
}
