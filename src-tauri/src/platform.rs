//! Small differences between operating systems, kept in one place.

use std::path::{Path, PathBuf};

/// Per-user data folder: %LOCALAPPDATA%\Rotae on Windows, $XDG_DATA_HOME/rotae
/// (usually ~/.local/share/rotae) elsewhere.
pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Rotae")
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
            .unwrap_or_else(std::env::temp_dir)
            .join("rotae")
    }
}

/// A program on PATH.
#[cfg_attr(windows, allow(dead_code))]
pub fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(name)).find(|p| p.is_file()))
}

/// `file://` URI with everything except unreserved characters and slashes percent-encoded
/// (commas too, so the URI is safe inside a dbus-send array argument).
#[cfg(not(windows))]
pub fn file_uri(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let enc: String =
        p.as_os_str().as_bytes().iter().map(|&b| if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("file://{enc}")
}

/// Opens a folder, file or URL with the desktop's default handler.
#[cfg(not(windows))]
pub fn xdg_open(target: impl AsRef<std::ffi::OsStr>) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    Command::new("xdg-open").arg(target).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map(|_| ())
}

/// Shows the file selected in the file manager (freedesktop FileManager1, which Nautilus,
/// Dolphin, Nemo and Thunar answer); otherwise opens its folder.
#[cfg(not(windows))]
pub fn reveal(p: &Path) -> std::io::Result<()> {
    let shown = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.FileManager1",
            "--type=method_call",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
        ])
        .arg(format!("array:string:{}", file_uri(p)))
        .arg("string:")
        .output()
        .is_ok_and(|o| o.status.success());
    if !shown {
        xdg_open(p.parent().unwrap_or(p))?;
    }
    Ok(())
}

/// One line naming the OS, for bug reports.
pub fn os_name() -> String {
    #[cfg(windows)]
    {
        crate::engines::tools::quiet(Path::new("cmd")).args(["/C", "ver"]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        let pretty = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| s.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())))
            .unwrap_or_else(|| "Linux".into());
        let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        format!("{pretty}, kernel {}, {desktop} {session}", kernel.trim())
    }
}

/// A sans-serif font file that covers Vietnamese: Segoe UI or Arial on Windows; on Linux what
/// fontconfig picks, else DejaVu, Noto or Liberation from the usual folders.
pub fn system_font(bold: bool) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let dir = Path::new(&std::env::var_os("WINDIR")?).join("Fonts");
        let names: &[&str] = if bold { &["segoeuib.ttf", "arialbd.ttf", "segoeui.ttf", "arial.ttf"] } else { &["segoeui.ttf", "arial.ttf"] };
        names.iter().map(|n| dir.join(n)).find(|p| p.is_file())
    }
    #[cfg(not(windows))]
    {
        let pattern = if bold { "sans:bold:lang=vi" } else { "sans:lang=vi" };
        let picked = std::process::Command::new("fc-match")
            .args(["-f", "%{file}", pattern])
            .output()
            .ok()
            .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
            // TrueType/OpenType only: collections (.ttc) don't load everywhere
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf")));
        picked.or_else(|| {
            let style = if bold { "-Bold" } else { "" };
            [
                format!("/usr/share/fonts/truetype/dejavu/DejaVuSans{style}.ttf"),
                format!("/usr/share/fonts/dejavu/DejaVuSans{style}.ttf"),
                format!("/usr/share/fonts/TTF/DejaVuSans{style}.ttf"),
                format!("/usr/share/fonts/truetype/noto/NotoSans{}.ttf", if bold { "-Bold" } else { "-Regular" }),
                format!("/usr/share/fonts/noto/NotoSans{}.ttf", if bold { "-Bold" } else { "-Regular" }),
                format!("/usr/share/fonts/truetype/liberation/LiberationSans{}.ttf", if bold { "-Bold" } else { "-Regular" }),
            ]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
        })
    }
}

/// The program to start from menus and login: the AppImage itself when running from one (the
/// running binary lives in a temporary mount that changes every start). Refused if its path
/// can't be written safely into a desktop file or shell script.
#[cfg(not(windows))]
pub fn launcher_path() -> std::io::Result<String> {
    let p = std::env::var_os("APPIMAGE").map(PathBuf::from).map(Ok).unwrap_or_else(std::env::current_exe)?;
    let s = p.to_string_lossy().into_owned();
    if s.contains(['"', '\'', '`', '$', '\\', '\n', '\r', '%']) {
        return Err(std::io::Error::other("Rotae is installed in a folder whose name can't be used in a menu entry; move it"));
    }
    Ok(s)
}

#[cfg(not(windows))]
fn autostart_file() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_default()
        .join("autostart")
        .join("rotae.desktop")
}

/// Starts with the session: Windows through the autostart plugin, Linux through an XDG autostart
/// entry that points at the AppImage or installed binary.
pub fn autostart_enabled(app: &tauri::AppHandle) -> bool {
    #[cfg(windows)]
    {
        use tauri_plugin_autostart::ManagerExt;
        app.autolaunch().is_enabled().unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        autostart_file().is_file()
    }
}

pub fn set_autostart(app: &tauri::AppHandle, on: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use tauri_plugin_autostart::ManagerExt;
        let al = app.autolaunch();
        if on { al.enable() } else { al.disable() }.map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        let f = autostart_file();
        if !on {
            let _ = std::fs::remove_file(&f);
            return Ok(());
        }
        let exe = launcher_path().map_err(|e| e.to_string())?;
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=Rotae\nComment=Drag a file to convert, compress, check and share it\nExec=\"{exe}\"\nIcon=rotae\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
        );
        std::fs::write(&f, entry).map_err(|e| e.to_string())
    }
}

/// Linux display set-up, before any window exists. On Wayland the app runs through XWayland:
/// only there can the drop strip sit at the screen edge and stay on top (Wayland lets no app
/// place its own windows). With NVIDIA's driver, WebKit's DMA-BUF renderer shows blank windows.
pub fn prepare_display() {
    #[cfg(target_os = "linux")]
    {
        let unset = |k: &str| std::env::var_os(k).is_none();
        if unset("GDK_BACKEND") && !unset("WAYLAND_DISPLAY") && !unset("DISPLAY") {
            std::env::set_var("GDK_BACKEND", "x11");
        }
        if unset("WEBKIT_DISABLE_DMABUF_RENDERER") && Path::new("/proc/driver/nvidia").exists() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }
}
