//! Add-ons Rotae downloads on request (analysis tools, FFmpeg, AI models, Whisper): where they
//! come from, how they are checked (pinned SHA-256 or the digest GitHub publishes), where they are
//! kept, and the Settings › Add-ons commands. security.rs and the other engines only ask
//! `tool_path` / `tool_file` whether something is installed.

use super::security::curl;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

// --- The table of add-ons ---------------------------------

/// tool id, GitHub repo, asset name must contain, exe name
pub const TOOLS: &[(&str, &str, &str, &str)] = &[
    // FFmpeg is not shipped inside the app; this downloads the official Windows build on request
    ("ffmpeg", "BtbN/FFmpeg-Builds", "win64-gpl.zip", "ffmpeg.exe"),
    ("capa", "mandiant/capa", "windows", "capa.exe"),
    ("floss", "mandiant/flare-floss", "windows", "floss.exe"),
    ("upx", "upx/upx", "win64", "upx.exe"),
    ("qpdf", "qpdf/qpdf", "msvc64", "qpdf.exe"),
    ("yara", "VirusTotal/yara-x", "windows", "yr.exe"),
    ("whisper", "ggml-org/whisper.cpp", "whisper-bin-x64", "whisper-cli.exe"),
    ("upscale", "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-windows.zip", "", "realesrgan-ncnn-vulkan.exe"),
    ("ai", "https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/onnxruntime-win-x64-1.22.0.zip", "", "onnxruntime.dll"),
];

/// Files some tools need besides the program: (url, file name, smallest plausible size).
fn tool_extras(id: &str) -> &'static [(&'static str, &'static str, u64)] {
    match id {
        "whisper" => &[(WHISPER_MODEL, "ggml-base.bin", 10_000_000)],
        "ai" => &[
            ("https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx", "isnet-general-use.onnx", 10_000_000),
            (
                "https://github.com/onnx/models/raw/4c46cd00fbdb7cd30b6c1c17ab54f2e1f4f7b177/validated/vision/body_analysis/ultraface/models/version-RFB-320.onnx",
                "version-RFB-320.onnx",
                500_000,
            ),
        ],
        _ => &[],
    }
}

/// Linux: add-ons that come from the distribution or another installer instead of a download.
#[cfg(not(windows))]
pub fn linux_hint(id: &str) -> Option<&'static str> {
    Some(match id {
        "ffmpeg" => "sudo apt install ffmpeg",
        "capa" => "pipx install flare-capa",
        "floss" => "pipx install flare-floss",
        "upx" => "sudo apt install upx-ucl",
        "qpdf" => "sudo apt install qpdf",
        "yara" => "cargo install yara-x-cli",
        // the model is still downloaded here once whisper-cli is on PATH
        "whisper" => "build whisper.cpp from github.com/ggml-org/whisper.cpp and put whisper-cli on PATH",
        _ => return None,
    })
}

/// Linux builds of the add-ons that are downloaded (pinned below).
#[cfg(not(windows))]
fn linux_source(id: &str) -> Option<&'static str> {
    match id {
        "upscale" => Some("https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-ubuntu.zip"),
        "ai" => Some("https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/onnxruntime-linux-x64-1.22.0.tgz"),
        _ => None,
    }
}

/// The program (or library) file an add-on provides on this system.
fn exe_name(id: &str) -> Option<&'static str> {
    let win = TOOLS.iter().find(|t| t.0 == id)?.3;
    if cfg!(windows) {
        return Some(win);
    }
    Some(match id {
        "ai" => "libonnxruntime.so.1.22.0",
        _ => win.strip_suffix(".exe").unwrap_or(win),
    })
}

/// "X isn't installed" in the words that fit this system: on Windows the add-on download, on
/// Linux the command that installs it.
pub fn missing(id: &str, windows: &str) -> String {
    #[cfg(not(windows))]
    if let Some(hint) = linux_hint(id) {
        let name = match id {
            "ffmpeg" => "FFmpeg",
            "whisper" => "whisper.cpp",
            other => other,
        };
        return format!("{name} isn't installed: {hint}");
    }
    let _ = id;
    windows.to_string()
}

/// A file that came with a tool (model, library), once installed.
pub fn tool_file(id: &str, name: &str) -> Option<PathBuf> {
    Some(tools_dir().join(id).join(name)).filter(|p| p.is_file())
}

/// whisper.cpp model: multilingual "base" (≈150 MB) handles Vietnamese and English well enough.
const WHISPER_MODEL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.bin";

/// SHA-256 of the files from pinned downloads (anything else is refused and deleted).
/// Tools that follow GitHub's latest release are checked against the digest GitHub publishes.
#[cfg(not(windows))]
const PINNED: &[(&str, &str, &str)] = &[
    ("ai", "libonnxruntime.so.1.22.0", "3da6146e14e7b8aaec625dde11d6114c7457c87a5f93d744897da8781e35c673"),
    ("ai", "isnet-general-use.onnx", "60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a"),
    ("ai", "version-RFB-320.onnx", "34cd7e60aeff28744c657de7a3dc64e872d506741de66987f3426f2b79f88017"),
    ("upscale", "realesrgan-ncnn-vulkan", "22c2296a39b40cc43ac92615c827ab6d4b60b7142e8cee59f718e3f8e2cdbda9"),
    ("upscale", "realesrgan-x4plus.bin", "713ee713b0353afaa27976f0563a64a5043bd70b9bd8936c2e26e25ebcdbcddf"),
    ("upscale", "realesrgan-x4plus.param", "35330ececcea33b6c397a72548e788d5d53becee4734c50b7fada36e89f10a86"),
    ("whisper", "ggml-base.bin", "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"),
];

#[cfg(windows)]
const PINNED: &[(&str, &str, &str)] = &[
    ("ai", "onnxruntime.dll", "579b636403983254346a5c1d80bd28f1519cd1e284cd204f8d4ff41f8d711559"),
    ("ai", "isnet-general-use.onnx", "60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a"),
    ("ai", "version-RFB-320.onnx", "34cd7e60aeff28744c657de7a3dc64e872d506741de66987f3426f2b79f88017"),
    ("upscale", "realesrgan-ncnn-vulkan.exe", "07e49f7cbb4ede01ae4dd4c399d3a7e5846e3d2085c3128eff881e55cb7b1a0c"),
    ("upscale", "realesrgan-x4plus.bin", "713ee713b0353afaa27976f0563a64a5043bd70b9bd8936c2e26e25ebcdbcddf"),
    ("upscale", "realesrgan-x4plus.param", "35330ececcea33b6c397a72548e788d5d53becee4734c50b7fada36e89f10a86"),
    ("whisper", "ggml-base.bin", "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe"),
];

pub fn whisper_model() -> Option<PathBuf> {
    tool_file("whisper", "ggml-base.bin")
}

pub fn tools_dir() -> PathBuf {
    crate::platform::data_dir().join("tools")
}

/// Path of an installed tool: our tools folder first, then PATH.
pub fn tool_path(id: &str) -> Option<PathBuf> {
    let exe = exe_name(id)?;
    let dir = tools_dir().join(id);
    if let Some(p) = find_file(&dir, exe, 3) {
        return Some(p);
    }
    // a DLL found on PATH (Windows ships an older onnxruntime.dll) would be loaded into the app itself
    if id == "ai" {
        return None;
    }
    std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join(exe)).find(|p| p.is_file()))
}

fn find_file(dir: &Path, file: &str, depth: u32) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() && depth > 0 {
            if let Some(f) = find_file(&p, file, depth - 1) {
                return Some(f);
            }
        } else if e.file_name().to_string_lossy().eq_ignore_ascii_case(file) {
            return Some(p);
        }
    }
    None
}

/// Downloads the latest Windows release of a tool from GitHub into the tools folder.
pub fn install_tool(id: &str) -> Result<PathBuf> {
    // a second click while the first download runs would delete its files
    static BUSY: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    {
        let mut busy = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        if busy.iter().any(|b| b == id) {
            bail!("already downloading");
        }
        busy.push(id.to_string());
    }
    let r = install_tool_once(id);
    BUSY.lock().unwrap_or_else(|e| e.into_inner()).retain(|b| b != id);
    r
}

fn install_tool_once(id: &str) -> Result<PathBuf> {
    #[allow(unused_variables)]
    let (_, source, want, _) = *TOOLS.iter().find(|t| t.0 == id).context("unknown tool")?;
    #[cfg(not(windows))]
    if id == "whisper" {
        return install_whisper_model();
    }
    #[cfg(not(windows))]
    if let Some(hint) = linux_hint(id) {
        bail!("{}", missing(id, hint));
    }
    #[cfg(not(windows))]
    let (source, want) = (linux_source(id).context("unknown tool")?, "");
    let mut digest = None;
    let url = if source.starts_with("https://") {
        source.to_string()
    } else {
        let api = super::tools::quiet(&curl())
            .args(["-sL", "--max-time", "30", "-H", "User-Agent: Rotae"])
            .arg(format!("https://api.github.com/repos/{source}/releases/latest"))
            .output()?;
        let v: serde_json::Value = serde_json::from_slice(&api.stdout).context("GitHub didn't answer")?;
        let asset = v["assets"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["name"].as_str().is_some_and(|n| n.to_lowercase().contains(want) && n.ends_with(".zip"))))
            .context("no Windows download found")?;
        digest = Some(
            asset["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).context("GitHub didn't publish a checksum for this download, so it wasn't installed")?.to_lowercase(),
        );
        asset["browser_download_url"].as_str().context("no download link")?.to_string()
    };
    // build the new copy next to the old one; a failed or tampered download leaves the working one alone
    let dir = tools_dir().join(id);
    let new = tools_dir().join(format!("{id}.new"));
    let _ = std::fs::remove_dir_all(&new);
    std::fs::create_dir_all(&new)?;
    let r = (|| -> Result<()> {
        let zip = new.join("download.zip");
        download(&url, &zip, 1000)?;
        if let Some(want) = &digest {
            if &super::files::sha256(&zip)? != want {
                bail!("the download didn't match its published checksum, so it wasn't installed");
            }
        }
        super::files::untar(&zip, &new)?;
        let _ = std::fs::remove_file(&zip);
        for (url, name, min) in tool_extras(id) {
            download(url, &new.join(name), *min)?;
        }
        for (_, name, sum) in PINNED.iter().filter(|p| p.0 == id) {
            let f = find_file(&new, name, 4).with_context(|| format!("the download is missing {name}"))?;
            if super::files::sha256(&f)? != *sum {
                bail!("{name} didn't match its known checksum, so it wasn't installed");
            }
        }
        // zips don't keep the executable bit
        #[cfg(not(windows))]
        if let Some(exe) = exe_name(id).and_then(|n| find_file(&new, n, 4)) {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    })();
    if let Err(e) = r {
        let _ = std::fs::remove_dir_all(&new);
        return Err(e);
    }
    // swap: the old copy moves aside first (fails cleanly while it is running), comes back if
    // the new one can't move in, and is deleted only once the new one is in place
    let old = tools_dir().join(format!("{id}.old"));
    let _ = std::fs::remove_dir_all(&old);
    let had_old = dir.exists();
    if had_old {
        if let Err(e) = std::fs::rename(&dir, &old) {
            let _ = std::fs::remove_dir_all(&new);
            return Err(anyhow::Error::from(e).context("the old version is still in use; close what is using it and try again"));
        }
    }
    if let Err(e) = std::fs::rename(&new, &dir) {
        if had_old {
            let _ = std::fs::rename(&old, &dir);
        }
        let _ = std::fs::remove_dir_all(&new);
        return Err(anyhow::Error::from(e).context("couldn't put the new version in place"));
    }
    let _ = std::fs::remove_dir_all(&old);
    tool_path(id).context("the download didn't contain the program")
}

/// Linux: whisper-cli comes from the user's own build; only the speech model is downloaded.
#[cfg(not(windows))]
fn install_whisper_model() -> Result<PathBuf> {
    let cli = tool_path("whisper").with_context(|| missing("whisper", ""))?;
    let dir = tools_dir().join("whisper");
    std::fs::create_dir_all(&dir)?;
    let model = dir.join("ggml-base.bin");
    let tmp = dir.join("ggml-base.bin.part");
    download(WHISPER_MODEL, &tmp, 10_000_000)?;
    let want = PINNED.iter().find(|p| p.1 == "ggml-base.bin").map(|p| p.2).unwrap_or_default();
    if super::files::sha256(&tmp)? != want {
        let _ = std::fs::remove_file(&tmp);
        bail!("ggml-base.bin didn't match its known checksum, so it wasn't installed");
    }
    std::fs::rename(&tmp, &model)?;
    Ok(cli)
}

/// The largest add-on (FFmpeg, the AI runtime) is a few hundred MB; anything far bigger is not
/// what was asked for and must not fill the disk.
const MAX_DOWNLOAD: &str = "2147483648";

fn download(url: &str, to: &Path, min_size: u64) -> Result<()> {
    let mut child = super::tools::quiet(&curl())
        .args(["-sfL", "--proto", "=https", "--proto-redir", "=https", "--retry", "2", "--max-time", "1800", "--max-filesize", MAX_DOWNLOAD, "-o"])
        .arg(to)
        .arg(url)
        .spawn()?;
    super::tools::adopt(child.id());
    let ok = child.wait()?.success();
    if !ok || std::fs::metadata(to).map(|m| m.len()).unwrap_or(0) < min_size {
        let _ = std::fs::remove_file(to);
        bail!("download failed: {}", url.rsplit('/').next().unwrap_or(url));
    }
    Ok(())
}

#[derive(serde::Serialize)]
pub struct ToolInfo {
    id: &'static str,
    installed: bool,
    /// how to install it instead of a download (Linux distribution packages)
    hint: Option<&'static str>,
}

#[tauri::command]
pub async fn tools_status() -> Vec<ToolInfo> {
    TOOLS
        .iter()
        .map(|t| {
            #[cfg(not(windows))]
            let hint = linux_hint(t.0);
            #[cfg(windows)]
            let hint = None;
            // Linux whisper: the program is the user's, the model is ours; once the program is
            // there, the model is a normal download
            #[cfg(not(windows))]
            let hint = hint.filter(|_| !(t.0 == "whisper" && tool_path("whisper").is_some()));
            ToolInfo { id: t.0, installed: tool_path(t.0).is_some() && tool_extras(t.0).iter().all(|(_, n, _)| tool_file(t.0, n).is_some()), hint }
        })
        .collect()
}

#[tauri::command]
pub async fn tools_install(id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || install_tool(&id).map(|_| ()).map_err(|e| e.to_string())).await.map_err(|e| e.to_string())?
}
