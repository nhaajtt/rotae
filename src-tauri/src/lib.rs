mod apps;
mod clipboard;
mod engines;
#[cfg(windows)]
mod explorer;
mod jobs;
#[cfg(test)]
mod lib_tests;
mod naming;
mod platform;
mod settings;
mod share;
mod shell;
mod tray;
mod watch;
#[cfg(windows)]
mod win;
mod windows;
#[cfg(windows)]
mod winrt;
#[cfg(target_os = "linux")]
mod x11drag;

use std::sync::Mutex;
use tauri::{Manager, RunEvent};

/// Where the log lives: the per-user data folder (kept on this PC, never sent anywhere).
pub fn log_dir() -> std::path::PathBuf {
    platform::data_dir().join("logs")
}

/// Appends a timestamped line to the log; the file is rotated at 1 MB.
pub fn log(msg: &str) {
    use std::io::Write;
    static LOCK: Mutex<()> = Mutex::new(());
    let _g = LOCK.lock();
    let dir = log_dir();
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("rotae.log");
    if std::fs::metadata(&file).map(|m| m.len() > 1 << 20).unwrap_or(false) {
        let _ = std::fs::rename(&file, dir.join("rotae.old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let _ = writeln!(f, "{secs} {msg}");
    }
}

/// Opens a help link from a failure's fix. Only these sites, so a message can never open anything else.
#[tauri::command]
async fn open_link(url: String) -> Result<(), String> {
    if !link_allowed(&url) {
        return Err("link not allowed".into());
    }
    engines::security::open_url(&url).map_err(|e| e.to_string())
}

/// Links the windows may open: download pages of LibreOffice and Office, and this app's GitHub
/// pages. The path may not climb out of its prefix ("..", "%2e%2e", backslashes), and only
/// plain URL characters are accepted, so nothing reaches the browser or shell half-escaped.
fn link_allowed(url: &str) -> bool {
    const ALLOWED: [&str; 3] = ["https://www.libreoffice.org/", "https://www.microsoft.com/", "https://github.com/nhaajtt/rotae/"];
    // the repo itself (no slash) is allowed too, but not look-alikes such as …/rotae-evil
    let prefixed = url == "https://github.com/nhaajtt/rotae" || ALLOWED.iter().any(|a| url.starts_with(a));
    let plain = url.len() <= 2048 && url.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~:/?#[]@!&'()*+,;=%".contains(&b));
    let path = url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
    let climbs = path.split('/').any(|s| s == "." || s == "..") || path.contains("%2e") || path.contains("%2f") || path.contains("%5c") || path.contains('@');
    prefixed && plain && !climbs
}

/// Opens the log folder in Explorer.
#[tauri::command]
async fn open_logs() -> Result<(), String> {
    let dir = log_dir();
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(windows)]
    return std::process::Command::new("explorer").arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string());
    #[cfg(not(windows))]
    platform::xdg_open(&dir).map_err(|e| e.to_string())
}

/// Opens a new GitHub issue with the app and Windows versions filled in. Nothing is sent by
/// the app itself; the user reviews the text in the browser before submitting.
#[tauri::command]
async fn report_bug(app: tauri::AppHandle, context: Option<String>) -> Result<(), String> {
    let version = app.package_info().version.to_string();
    let build = platform::os_name();
    let message = context.map(|c| format!("**Message**\n```\n{c}\n```\n\n")).unwrap_or_default();
    let body = format!(
        "**What happened**\n\n\n**What you expected**\n\n\n**Steps**\n1. \n\n{message}**Version** Rotae {version}, {build}\n\n_Settings › Report a problem › Open log folder has a log you can attach (it contains file names)._"
    );
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    let url = format!("https://github.com/nhaajtt/rotae/issues/new?title={}&body={}", enc("Bug: "), enc(&body));
    engines::security::open_url(&url).map_err(|e| e.to_string())
}

#[tauri::command]
fn ui_log(msg: String) {
    // one line, bounded: a page can't flood the log or fake other entries
    let msg: String = msg.chars().take(500).map(|c| if c.is_control() { ' ' } else { c }).collect();
    log(&format!("ui: {msg}"));
}

/// Where pdfium.dll (and optionally ffmpeg.exe / ffprobe.exe) live.
fn bin_dir(app: &tauri::App) -> std::path::PathBuf {
    let bundled = app.path().resource_dir().map(|d| d.join("resources").join("bin")).unwrap_or_default();
    if cfg!(debug_assertions) && !bundled.exists() {
        return std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join("bin");
    }
    bundled
}

#[cfg(windows)]
fn start_drag_watch(app: tauri::AppHandle) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tauri::Emitter;
    use win::HookEvent;

    let (tx, rx) = std::sync::mpsc::channel();
    win::start_hook(tx);
    let open = AtomicBool::new(false);
    std::thread::spawn(move || {
        let mut last_move = std::time::Instant::now();
        for ev in rx {
            match ev {
                HookEvent::ShiftDrag { ox, oy } => {
                    let s = settings::get(&app);
                    if !s.enabled || win::is_own_window_at(ox, oy) || (s.explorer_only && !win::is_explorer_at(ox, oy)) {
                        continue;
                    }
                    // Shift-drag from empty space in Explorer selects files with a box: not a file drag
                    if win::is_explorer_at(ox, oy) && !explorer::on_item(ox, oy) {
                        continue;
                    }
                    let paths = explorer::selection(win::root_handle_at(ox, oy));
                    log(&format!("shift-drag from {ox},{oy}: {paths:?}"));
                    if paths.is_empty() {
                        continue;
                    }
                    open.store(true, Ordering::SeqCst);
                    windows::show_overlay(&app, ox, oy, &paths);
                }
                HookEvent::Moved { x, y } => {
                    // high-rate mice report up to 8000 moves a second; ~120 updates are plenty
                    if open.load(Ordering::SeqCst) && last_move.elapsed() >= std::time::Duration::from_millis(8) {
                        last_move = std::time::Instant::now();
                        windows::overlay_pointer(&app, x, y, false);
                    }
                }
                HookEvent::Peek { root } => {
                    if settings::get(&app).peek {
                        let paths = explorer::selection(root);
                        if let Some(p) = paths.first() {
                            let _ = windows::open_peek_now(app.clone(), p.display().to_string());
                        }
                    }
                }
                HookEvent::Key { n } => {
                    if open.load(Ordering::SeqCst) {
                        let _ = app.emit_to("overlay", "overlay:key", n);
                        if n == 0 {
                            open.store(false, Ordering::SeqCst);
                            windows::hide_overlay_later(&app);
                        }
                    }
                }
                HookEvent::Released { x, y } => {
                    if open.swap(false, Ordering::SeqCst) {
                        windows::overlay_pointer(&app, x, y, true);
                    }
                }
            }
        }
    });
}

/// Linux: drags reach the wheel through its drop strip at the screen edge (see windows.rs), and on
/// X11 also by holding Shift while dragging.
#[cfg(not(windows))]
fn start_drag_watch(app: tauri::AppHandle) {
    #[cfg(target_os = "linux")]
    x11drag::start(app);
    #[cfg(not(target_os = "linux"))]
    let _ = app;
}

fn autostart_on_first_run(app: &tauri::AppHandle) -> Result<(), String> {
    platform::set_autostart(app, true)
}

/// Files passed with `--open` (Explorer's right-click entry). Explorer may start one process per
/// selected file, so paths arriving within a moment are gathered into one hub window.
fn opened_with(app: &tauri::AppHandle, argv: Vec<String>) {
    static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let Some(i) = argv.iter().position(|a| a == "--open") else { return };
    let first = {
        let mut p = PENDING.lock().unwrap();
        let first = p.is_empty();
        p.extend(argv[i + 1..].iter().cloned());
        first
    };
    if first {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(350));
            let paths = std::mem::take(&mut *PENDING.lock().unwrap());
            let _ = windows::open_hub_now(app, paths, "actions".into(), None);
        });
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    platform::prepare_display();
    std::panic::set_hook(Box::new(|p| log(&format!("panic: {p}"))));
    log(&format!("start {}", env!("CARGO_PKG_VERSION")));
    let mut builder = tauri::Builder::default();
    // a test copy with its own data folder runs next to the installed app instead of handing over to it
    if platform::test_dir().is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _| opened_with(app, argv)));
    }
    builder
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .manage(settings::Store(Mutex::new(settings::Settings::default())))
        .invoke_handler(tauri::generate_handler![
            ui_log,
            open_logs,
            report_bug,
            apps::get_apps,
            apps::open_with,
            settings::get_settings,
            settings::update_settings,
            settings::get_autostart,
            settings::set_autostart,
            jobs::run_action,
            jobs::reveal,
            jobs::get_history,
            jobs::clear_history,
            windows::open_editor,
            windows::overlay_hide,
            windows::toast_hide,
            windows::toast_show,
            windows::toast_height,
            open_link,
            windows::open_hub,
            windows::hub_take,
            engines::security::tools_status,
            engines::security::tools_install,
            engines::security::sandbox_ok,
            engines::office::office_ok,
            engines::pdf::pdf_thumbs,
            engines::pentest::pentest_tools,
            jobs::trash,
            share::share_lan,
            share::share_r2,
            watch::folder_exists,
            windows::shelf_add,
            windows::shelf_take,
            windows::drag_icon,
            windows::open_peek,
            windows::read_text,
            windows::file_info,
            shell::clipboard_files,
            shell::rename_files,
            shell::context_menu,
            shell::context_menu_on,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let first = settings::first_run(&handle);
            settings::update(&handle, |s| *s = settings::load(&handle));
            engines::tools::set_bin_dir(bin_dir(app));
            app.manage(jobs::start(handle.clone()));
            windows::create(&handle)?;
            // a desktop without a tray (GNOME without the AppIndicator extension) still gets the
            // wheel, the hub (open Rotae again) and everything else
            if let Err(e) = tray::create(&handle) {
                log(&format!("tray: {e}"));
            }
            #[cfg(windows)]
            // debug aid: ROTAE_DEMO=<file> shows the menu for that file in the middle of the screen
            #[cfg(debug_assertions)]
            if let Ok(f) = std::env::var("ROTAE_DEMO") {
                let h = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    windows::show_overlay(&h, 1000, 600, &[f.into()]);
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    // ROTAE_DEMO_HOVER=dx,dy points somewhere else than the top wedge
                    let (dx, dy) = std::env::var("ROTAE_DEMO_HOVER")
                        .ok()
                        .and_then(|v| v.split_once(',').and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?))))
                        .unwrap_or((0, -100));
                    windows::overlay_pointer(&h, 1000 + dx, 600 + dy, false);
                    if std::env::var_os("ROTAE_DEMO_DROP").is_some() {
                        std::thread::sleep(std::time::Duration::from_secs(4));
                        windows::overlay_pointer(&h, 1000, 600, true);
                    }
                });
            }
            #[cfg(debug_assertions)]
            if let Ok(v) = std::env::var("ROTAE_EDIT") {
                if let Some((tool, path)) = v.split_once('|') {
                    let _ = windows::open_editor_now(handle.clone(), path.into(), tool.into());
                }
            }
            #[cfg(debug_assertions)]
            if let Ok(v) = std::env::var("ROTAE_HUB") {
                let files = std::env::var("ROTAE_HUB_FILES").map(|f| f.split(';').map(String::from).collect()).unwrap_or_default();
                let _ = windows::open_hub_now(handle.clone(), files, v, None);
            }
            watch::start(handle.clone());
            if first {
                // first launch: the app lives in the tray, so say hello and show how it works
                if platform::test_dir().is_none() {
                    let _ = autostart_on_first_run(&handle);
                }
                let _ = windows::open_hub_now(handle.clone(), vec![], "welcome".into(), None);
            }
            opened_with(&handle, std::env::args().collect());
            start_drag_watch(handle);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to start Rotae")
        .run(|_, ev| {
            // keep running in the tray when editor windows close
            if let RunEvent::ExitRequested { code, .. } = &ev {
                log(&format!("exit requested, code {code:?}"));
            }
            if let RunEvent::ExitRequested { api, code: None, .. } = ev {
                api.prevent_exit();
            } else if let RunEvent::Exit = ev {
                // quitting mid-job: stop its programs and don't leave half-written files behind
                engines::tools::end_all();
                naming::discard_running();
            }
        });
}
