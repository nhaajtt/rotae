//! Small file-system helpers for the hub: undo (to the Recycle Bin), files or images from the
//! clipboard, batch rename, and the Explorer right-click entry.

#[cfg(windows)]
use std::path::Path;
use std::path::PathBuf;

/// Moves files to the Recycle Bin, so "Undo" can itself be undone from there.
#[tauri::command]
pub async fn trash(paths: Vec<String>) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::{SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE, SHFILEOPSTRUCTW};
        let mut from: Vec<u16> = Vec::new();
        for p in paths.iter().filter(|p| Path::new(p).exists()) {
            from.extend(p.encode_utf16());
            from.push(0);
        }
        if from.is_empty() {
            return Ok(());
        }
        from.push(0);
        let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
        op.wFunc = FO_DELETE;
        op.pFrom = from.as_ptr();
        op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as u16;
        if unsafe { SHFileOperationW(&mut op) } != 0 {
            return Err("couldn't move to the Recycle Bin".into());
        }
    }
    // Linux: the desktop's trash (gio comes with GTK, trash-put with trash-cli). Never a plain
    // delete: Undo must stay undoable.
    #[cfg(not(windows))]
    {
        let paths: Vec<&String> = paths.iter().filter(|p| std::path::Path::new(p).exists()).collect();
        if paths.is_empty() {
            return Ok(());
        }
        let ok = |prog: &str, args: &[&str]| std::process::Command::new(prog).args(args).args(&paths).output().is_ok_and(|o| o.status.success());
        if !(ok("gio", &["trash", "--"]) || ok("trash-put", &["--"])) {
            return Err("couldn't move to the trash (needs gio or trash-cli)".into());
        }
    }
    Ok(())
}

/// Files copied in Explorer, or a copied image saved to Pictures\Rotae.
#[tauri::command]
pub async fn clipboard_files() -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        if let Some(files) = clip::files() {
            return Ok(files);
        }
        if let Some(png) = clip::image_png() {
            let dir = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Pictures").join("Rotae");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let out = crate::naming::unique(&dir, &format!("Pasted image {stamp}"), "png");
            std::fs::write(&out, png).map_err(|e| e.to_string())?;
            return Ok(vec![out.display().to_string()]);
        }
    }
    Err("the clipboard has no files or image".into())
}

#[cfg(windows)]
mod clip {
    use windows_sys::Win32::{
        System::{
            DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard},
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        },
        UI::Shell::{DragQueryFileW, HDROP},
    };
    const CF_HDROP: u32 = 15;
    const CF_DIB: u32 = 8;

    fn with<T>(f: impl FnOnce() -> Option<T>) -> Option<T> {
        unsafe {
            if OpenClipboard(std::ptr::null_mut()) == 0 {
                return None;
            }
            let r = f();
            CloseClipboard();
            r
        }
    }

    pub fn files() -> Option<Vec<String>> {
        with(|| unsafe {
            if IsClipboardFormatAvailable(CF_HDROP) == 0 {
                return None;
            }
            let h = GetClipboardData(CF_HDROP) as HDROP;
            let n = DragQueryFileW(h, u32::MAX, std::ptr::null_mut(), 0);
            let mut out = Vec::new();
            for i in 0..n {
                let len = DragQueryFileW(h, i, std::ptr::null_mut(), 0) as usize;
                let mut buf = vec![0u16; len + 1];
                DragQueryFileW(h, i, buf.as_mut_ptr(), buf.len() as u32);
                out.push(String::from_utf16_lossy(&buf[..len]));
            }
            (!out.is_empty()).then_some(out)
        })
    }

    /// The clipboard bitmap (CF_DIB) re-encoded as PNG.
    pub fn image_png() -> Option<Vec<u8>> {
        let dib = with(|| unsafe {
            if IsClipboardFormatAvailable(CF_DIB) == 0 {
                return None;
            }
            let h = GetClipboardData(CF_DIB);
            let p = GlobalLock(h) as *const u8;
            if p.is_null() {
                return None;
            }
            let v = std::slice::from_raw_parts(p, GlobalSize(h)).to_vec();
            GlobalUnlock(h);
            Some(v)
        })?;
        // a DIB is a BMP without its 14-byte file header
        let header_len = u32::from_le_bytes(dib.get(0..4)?.try_into().ok()?) as usize;
        let bits = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?) as usize;
        let colors = u32::from_le_bytes(dib.get(32..36)?.try_into().ok()?) as usize;
        let compression = u32::from_le_bytes(dib.get(16..20)?.try_into().ok()?);
        let masks = if compression == 3 && header_len == 40 { 12 } else { 0 };
        let palette = if bits <= 8 { (if colors == 0 { 1 << bits } else { colors }) * 4 } else { 0 };
        let offset = 14 + header_len + masks + palette;
        let mut bmp = Vec::with_capacity(dib.len() + 14);
        bmp.extend(b"BM");
        bmp.extend(((dib.len() + 14) as u32).to_le_bytes());
        bmp.extend([0u8; 4]);
        bmp.extend((offset as u32).to_le_bytes());
        bmp.extend(&dib);
        let img = image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp).ok()?;
        // 32-bit clipboard bitmaps often carry an empty alpha channel
        let img = if img.color().has_alpha() && img.to_rgba8().pixels().all(|p| p[3] == 0) { image::DynamicImage::ImageRgb8(img.to_rgb8()) } else { img };
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).ok()?;
        Some(out.into_inner())
    }
}

/// Renames files (old, new full paths). Goes through temporary names so swaps work.
#[tauri::command]
pub async fn rename_files(pairs: Vec<(String, String)>) -> Result<usize, String> {
    let pairs: Vec<(PathBuf, PathBuf)> = pairs.into_iter().map(|(a, b)| (PathBuf::from(a), PathBuf::from(b))).filter(|(a, b)| a != b).collect();
    let sources: std::collections::HashSet<_> = pairs.iter().map(|(a, _)| a.to_string_lossy().to_lowercase()).collect();
    let mut seen = std::collections::HashSet::new();
    for (_, b) in &pairs {
        let key = b.to_string_lossy().to_lowercase();
        if !seen.insert(key.clone()) {
            return Err(format!("two files would be named {}", b.file_name().unwrap_or_default().to_string_lossy()));
        }
        if b.exists() && !sources.contains(&key) {
            return Err(format!("{} already exists", b.file_name().unwrap_or_default().to_string_lossy()));
        }
    }
    if sources.len() != pairs.len() {
        return Err("the same file is listed twice".into());
    }
    // renaming only: a pattern like "..\x" must not move files into another folder
    if let Some((_, b)) = pairs.iter().find(|(a, b)| a.parent() != b.parent() || b.file_name().is_none()) {
        return Err(format!("{} would move the file to another folder", b.display()));
    }
    if let Some((a, _)) = pairs.iter().find(|(a, _)| !a.exists()) {
        return Err(format!("{} no longer exists", a.display()));
    }
    // two steps (via temporary names) so names can be swapped; any failure puts everything back
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new(); // (now at, originally at)
    let undo = |done: &mut Vec<(PathBuf, PathBuf)>| {
        for (now, was) in done.drain(..).rev() {
            let _ = std::fs::rename(&now, &was);
        }
    };
    let mut temps = Vec::new();
    for (i, (a, _)) in pairs.iter().enumerate() {
        let t = a.with_file_name(format!(".rotae-rename-{}-{i}", std::process::id()));
        if let Err(e) = std::fs::rename(a, &t) {
            undo(&mut done);
            return Err(format!("{}: {e}", a.file_name().unwrap_or_default().to_string_lossy()));
        }
        done.push((t.clone(), a.clone()));
        temps.push(t);
    }
    for ((_, b), t) in pairs.iter().zip(&temps) {
        if let Err(e) = std::fs::rename(t, b) {
            undo(&mut done);
            return Err(format!("{}: {e}", b.file_name().unwrap_or_default().to_string_lossy()));
        }
        // undoing this one means moving it from its new name straight back
        if let Some(d) = done.iter_mut().find(|d| &d.0 == t) {
            d.0 = b.clone();
        }
    }
    Ok(pairs.len())
}

/// "Rotae" in Explorer's right-click menu (per user, no admin rights needed).
#[tauri::command]
pub async fn context_menu(enable: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        for root in [r"HKCU\Software\Classes\*\shell\Rotae", r"HKCU\Software\Classes\Directory\shell\Rotae"] {
            let reg = |args: &[&str]| crate::engines::tools::quiet(Path::new("reg")).args(args).output().map(|o| o.status.success()).unwrap_or(false);
            if enable {
                let cmd = format!("\"{}\" --open \"%1\"", exe.display());
                let icon = format!("{},0", exe.display());
                let ok = reg(&["add", root, "/ve", "/d", "Rotae…", "/f"])
                    && reg(&["add", root, "/v", "Icon", "/d", &icon, "/f"])
                    && reg(&["add", root, "/v", "MultiSelectModel", "/d", "Player", "/f"])
                    && reg(&["add", &format!(r"{root}\command"), "/ve", "/d", &cmd, "/f"]);
                if !ok {
                    return Err("couldn't add the menu entry".into());
                }
            } else {
                reg(&["delete", root, "/f"]);
            }
        }
    }
    #[cfg(not(windows))]
    linux_menu::set(enable).map_err(|e| e.to_string())?;
    let _ = enable;
    Ok(())
}

#[tauri::command]
pub async fn context_menu_on() -> bool {
    #[cfg(windows)]
    return crate::engines::tools::quiet(Path::new("reg")).args(["query", r"HKCU\Software\Classes\*\shell\Rotae\command"]).output().map(|o| o.status.success()).unwrap_or(false);
    #[cfg(not(windows))]
    linux_menu::is_on()
}

/// Linux: "Rotae…" in the file managers' right-click menus, all per user:
/// Nautilus script, Nemo action, Dolphin service menu, Thunar custom action, and an
/// "Open with" entry that every file manager shows.
#[cfg(not(windows))]
mod linux_menu {
    use std::{fs, io, path::PathBuf};

    const THUNAR_ID: &str = "rotae-open";

    fn home() -> PathBuf {
        std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
    }
    fn data() -> PathBuf {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".local").join("share"))
    }
    fn config() -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".config"))
    }

    fn exe() -> io::Result<String> {
        crate::platform::launcher_path()
    }

    fn files() -> [PathBuf; 4] {
        [
            data().join("nautilus").join("scripts").join("Rotae…"),
            data().join("nemo").join("actions").join("rotae.nemo_action"),
            data().join("kio").join("servicemenus").join("rotae.desktop"),
            data().join("applications").join("rotae-open.desktop"),
        ]
    }

    fn write(p: &PathBuf, text: &str, executable: bool) -> io::Result<()> {
        if let Some(d) = p.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(p, text)?;
        if executable {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(p, fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }

    pub fn set(enable: bool) -> io::Result<()> {
        let [nautilus, nemo, dolphin, open_with] = files();
        if !enable {
            for p in files() {
                let _ = fs::remove_file(p);
            }
            return thunar(None);
        }
        let exe = exe()?;
        write(&nautilus, &format!("#!/bin/sh\n# Rotae: actions for the selected files\nset -f\nIFS='\n'\nexec \"{exe}\" --open $NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\n"), true)?;
        write(
            &nemo,
            &format!("[Nemo Action]\nName=Rotae…\nComment=Convert, compress, check and share\nExec=\"{exe}\" --open %F\nIcon-Name=rotae\nSelection=notnone\nExtensions=any;\n"),
            false,
        )?;
        write(&dolphin, &format!("[Desktop Entry]\nType=Service\nMimeType=application/octet-stream;inode/directory;all/allfiles;\nActions=rotae\nX-KDE-Priority=TopLevel\n\n[Desktop Action rotae]\nName=Rotae…\nIcon=rotae\nExec=\"{exe}\" --open %F\n"), true)?;
        write(&open_with, &format!("[Desktop Entry]\nType=Application\nName=Rotae\nComment=Convert, compress, check and share files\nExec=\"{exe}\" --open %F\nIcon=rotae\nTerminal=false\nNoDisplay=true\nMimeType=application/octet-stream;application/pdf;image/png;image/jpeg;image/webp;video/mp4;audio/mpeg;text/plain;\n"), false)?;
        thunar(Some(&exe))
    }

    /// Thunar keeps custom actions in one XML file; only our own entry is added or removed.
    fn thunar(exe: Option<&str>) -> io::Result<()> {
        let p = config().join("Thunar").join("uca.xml");
        let mut xml = fs::read_to_string(&p).unwrap_or_else(|_| "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<actions>\n</actions>\n".into());
        // drop an earlier copy of our entry
        while let Some(id) = xml.find(&format!("<unique-id>{THUNAR_ID}</unique-id>")) {
            let (Some(start), Some(end)) = (xml[..id].rfind("<action>"), xml[id..].find("</action>")) else { break };
            let end = id + end + "</action>".len();
            xml.replace_range(start..end, "");
        }
        if let Some(exe) = exe {
            let esc = exe.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
            let entry = format!("<action>\n\t<icon>rotae</icon>\n\t<name>Rotae…</name>\n\t<unique-id>{THUNAR_ID}</unique-id>\n\t<command>&quot;{esc}&quot; --open %F</command>\n\t<description>Convert, compress, check and share</description>\n\t<patterns>*</patterns>\n\t<directories/>\n\t<audio-files/>\n\t<image-files/>\n\t<other-files/>\n\t<text-files/>\n\t<video-files/>\n</action>\n");
            let Some(at) = xml.rfind("</actions>") else { return Err(io::Error::other("Thunar's actions file looks damaged; left it alone")) };
            xml.insert_str(at, &entry);
        } else if !p.exists() {
            return Ok(());
        }
        if let Some(d) = p.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(&p, xml)
    }

    pub fn is_on() -> bool {
        files().iter().any(|p| p.exists())
    }
}
