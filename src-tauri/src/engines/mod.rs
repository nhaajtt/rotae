pub mod ai;
pub mod dev;
pub mod docx;
pub mod files;
pub mod image;
pub mod imagex;
pub mod magic;
pub mod media;
pub mod office;
pub mod pdf;
pub mod pdfwrite;
pub mod pdfx;
pub mod pentest;
pub mod recon;
pub mod security;
pub mod text;
pub mod tools;
#[cfg(test)]
mod tests;

use crate::naming::{kind, Kind};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Low,
    Medium,
    High,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Action {
    Convert { to: String },
    Compress { level: Level },
    /// Images: percent of the original size. Video: target height in pixels.
    Resize { value: u32 },
    Rotate { deg: u32 },
    StripMetadata,
    ExtractAudio { to: String },
    Mute,
    Merge,
    Split,
    /// Rectangles are fractions (0–1) of the image: [x, y, w, h].
    Crop { rect: [f64; 4] },
    AddBg { color: String, padding: f64, radius: f64, shadow: f64 },
    Redact { boxes: Vec<[f64; 4]> },
    Trim { start: f64, end: f64 },
    FormatJson { pretty: bool },
    Zip,
    Extract,
    Hash,
    CopyPath,
    CopyText,
    Strings,
    PeInfo,
    Identify,
    VerifySignature,
    /// md5 | sha1 | sha256 | sha512 → clipboard; all → report file
    Hashes { algo: String },
    Sandbox,
    Encrypt { password: String },
    Decrypt { password: String },
    Shred,
    Locks,
    VirusTotal,
    /// capa | floss | upx
    Tool { tool: String },
    Base64 { decode: bool },
    /// plain | quoted | wsl | url | markdown
    CopyPathAs { style: String },
    Diff,
    /// Re-encode to fit in this many megabytes (e.g. Discord, Zalo, email limits).
    TargetSize { mb: f64 },
    Speed { factor: f64 },
    /// h | v
    Flip { dir: String },
    /// one PNG every `every` seconds into a folder
    Frames { every: f64 },
    Thumbnail,
    /// images → video, `seconds` per image
    Slideshow { seconds: f64 },
    /// denoise | normalize
    AudioFix { mode: String },
    /// speech → .srt with whisper.cpp; `burn` also draws them into the video
    Transcribe { burn: bool },
    /// clipboard | txt
    Ocr { to: String },
    QrRead,
    /// text file → QR code image
    QrMake,
    /// .ico for apps, or a favicon folder for websites
    IconSet { web: bool },
    Palette,
    Watermark { text: String },
    ContactSheet,
    /// image → SVG
    Vectorize,
    RemoveBg,
    BlurFaces,
    Upscale { scale: u32 },
    /// keep these pages (0-based) in this order, each rotated by `rotate[i]` degrees
    Pages { order: Vec<u32>, #[serde(default)] rotate: Vec<u32> },
    PageNumbers,
    /// a PNG (data URL) placed on `page` at [x, y, w, h] fractions from the top-left
    Stamp { page: u32, rect: [f64; 4], png: String },
    /// text of every page, OCR for scanned ones
    PdfText,
    ExtractImages,
    Protect { password: String },
    Unprotect { password: String },
    /// a recipe: each step runs on what the previous one made
    Chain { steps: Vec<Action> },
    /// the user's own command line; {file} {dir} {name} {stem} {ext} are filled in
    Custom { command: String },
    /// translate | summarize, with the user's Claude API key
    AskClaude { task: String, #[serde(default)] lang: String },
    /// a tool from the pentest manifest, sandboxed (Pentest user mode)
    Pentest {
        tool: String,
        /// the tool's `confirm` question was answered (checked again in the backend)
        #[serde(default)]
        confirmed: bool,
    },
    /// Work done through Apple's frameworks or plain text tools; only exists in the Mac build.
    #[cfg(target_os = "macos")]
    Native { verb: String, #[serde(default)] arg: String },
}

thread_local! {
    static NOTES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A one-line result shown in the toast (for actions that don't create files).
pub fn note(msg: &str) {
    NOTES.with(|n| n.borrow_mut().push(msg.to_string()));
}

/// Notes left by the actions run on this thread since the last call.
pub fn take_notes() -> Vec<String> {
    NOTES.with(|n| std::mem::take(&mut *n.borrow_mut()))
}

/// Actions that treat the whole selection as one unit.
pub fn is_group(a: &Action) -> bool {
    #[cfg(target_os = "macos")]
    if let Action::Native { verb, arg } = a {
        return crate::desk::is_group(verb, arg);
    }
    files::is_group(a) || matches!(a, Action::Merge | Action::Diff | Action::Shred | Action::Slideshow { .. } | Action::ContactSheet) || matches!(a, Action::Hashes { algo } if algo != "all")
}

pub type Progress<'a> = &'a dyn Fn(f32);

/// Runs one action on one file (or on all files for `Merge`). Returns created files.
pub fn run(paths: &[PathBuf], action: &Action, progress: Progress) -> Result<Vec<PathBuf>> {
    if let Action::Merge = action {
        return merge(paths, progress);
    }
    if let Action::Chain { steps } = action {
        return chain(paths, steps, progress);
    }
    if let Action::AskClaude { task, lang } = action {
        return crate::share::ask_claude(&paths[0], task, lang).map(|p| vec![p]);
    }
    if pentest::handles(action) {
        return pentest::run(paths, action, progress);
    }
    if let Action::Custom { command } = action {
        return custom(&paths[0], command);
    }
    if let Action::Slideshow { seconds } = action {
        return media::slideshow(paths, *seconds, progress);
    }
    if files::is_group(action) || matches!(action, Action::Extract | Action::Strings | Action::PeInfo) {
        return files::run(paths, action, progress);
    }
    if security::handles(action) {
        return security::run(paths, action, progress);
    }
    if kind(&paths[0]) == Kind::Pdf && pdfx::handles(action) {
        return pdfx::run(&paths[0], action, progress);
    }
    #[cfg(target_os = "macos")]
    if let Some(r) = crate::native::run(paths, action) {
        return r;
    }
    if ai::handles(action) {
        return ai::run(paths, action, progress);
    }
    if imagex::handles(action) {
        return imagex::run(paths, action, progress);
    }
    if dev::handles(action) {
        return dev::run(paths, action, progress);
    }
    let [src] = paths else { bail!("expected one file") };
    match kind(src) {
        Kind::Text => text::run(src, action, progress),
        Kind::Image => image::run(src, action, progress),
        Kind::Video | Kind::Audio => media::run(src, action, progress),
        Kind::Pdf => pdf::run(src, action, progress),
        Kind::Office => office::run(src, action, progress),
        Kind::Archive | Kind::Exe | Kind::Other => bail!("unsupported file type"),
    }
}

fn chain(paths: &[PathBuf], steps: &[Action], progress: Progress) -> Result<Vec<PathBuf>> {
    let n = steps.len().max(1) as f32;
    let mut cur = paths.to_vec();
    let mut made = Vec::new();
    for (k, step) in steps.iter().enumerate() {
        let units: Vec<Vec<PathBuf>> = if is_group(step) { vec![cur.clone()] } else { cur.iter().map(|p| vec![p.clone()]).collect() };
        let mut next = Vec::new();
        for u in units {
            let r = run(&u, step, &|p| progress((k as f32 + p) / n));
            // "there was nothing to do" must not sink a recipe: the file goes on as it is
            #[cfg(target_os = "macos")]
            let r = match r {
                Err(e) if k > 0 && crate::errors::code(&e.to_string()) == "already_small" => Ok(u.clone()),
                o => o,
            };
            next.extend(r?);
        }
        // steps that only copy or report (hash, copy path…) pass their input on
        if next.is_empty() {
            next = cur.clone();
        }
        made.extend(next.iter().filter(|p| !paths.contains(p)).cloned());
        cur = next;
    }
    // keep only the final results, not the files made along the way
    for p in made.iter().filter(|p| !cur.contains(p)) {
        let _ = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
    }
    Ok(cur.into_iter().filter(|p| !paths.contains(p)).collect())
}

/// Runs a user-defined command on one file, from the file's folder.
fn custom(src: &std::path::Path, command: &str) -> Result<Vec<PathBuf>> {
    use crate::naming::{ext, stem};
    let dir = src.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    // the names are pasted into a cmd line, sometimes outside quotes: a file called
    // "a & del x.txt" must never become a second command
    #[cfg(windows)]
    if src.to_string_lossy().contains(['&', '|', '<', '>', '^', '%', '"']) {
        bail!("this file name has characters (& | < > ^ %) that are unsafe in a command line; rename it first");
    }
    // sh expands $(…), `…` and \ even inside double quotes
    #[cfg(all(unix, not(target_os = "macos")))]
    if src.to_string_lossy().contains(['&', '|', '<', '>', ';', '$', '`', '\\', '"', '\'', '\n', '\r']) {
        bail!("this file name has characters ($ ` \\ ; & | < > or quotes) that are unsafe in a command line; rename it first");
    }
    #[cfg_attr(target_os = "macos", allow(unused))]
    let q = |s: &str| format!("\"{s}\"");
    #[cfg_attr(target_os = "macos", allow(unused))]
    let line = command
        .replace("{file}", &q(&src.display().to_string()))
        .replace("{dir}", &q(&dir.display().to_string()))
        .replace("{name}", &src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
        .replace("{stem}", &stem(src))
        .replace("{ext}", &ext(src));
    let before: std::collections::HashSet<PathBuf> = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    #[cfg(windows)]
    let o = {
        use std::os::windows::process::CommandExt;
        tools::output_within_unowned(tools::quiet(std::path::Path::new("cmd")).arg("/D").arg("/C").raw_arg(&line).current_dir(&dir), std::time::Duration::from_secs(3600))?
    };
    #[cfg(target_os = "macos")]
    let o = {
        // values travel as environment variables, so a file name can never become shell code
        let line = shell_template(command);
        let n = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        tools::output_within_unowned(std::process::Command::new("/bin/sh").arg("-c").arg(line).current_dir(&dir).env("SD_FILE", src).env("SD_DIR", &dir).env("SD_NAME", n).env("SD_STEM", stem(src)).env("SD_EXT", ext(src)), std::time::Duration::from_secs(3600))?
    };
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let o = tools::output_within_unowned(std::process::Command::new("sh").arg("-c").arg(&line).current_dir(&dir), std::time::Duration::from_secs(3600))?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        bail!("{}", err.lines().last().unwrap_or("the command failed"));
    }
    if let Some(last) = String::from_utf8_lossy(&o.stdout).lines().rfind(|l| !l.trim().is_empty()) {
        note(last.trim());
    }
    // whatever appeared in the folder counts as the result
    Ok(std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).filter(|p| !before.contains(p)).collect()).unwrap_or_default())
}

/// Turns {file} and friends into shell variables, quoted correctly for where each one sits, so
/// the value is always one word and never read as shell code.
#[cfg(target_os = "macos")]
pub(crate) fn shell_template(command: &str) -> String {
    const VARS: [(&str, &str); 5] = [("{file}", "SD_FILE"), ("{dir}", "SD_DIR"), ("{name}", "SD_NAME"), ("{stem}", "SD_STEM"), ("{ext}", "SD_EXT")];
    let (mut out, mut rest, mut quote) = (String::new(), command, None::<char>);
    // $(...) and `...` start a fresh quoting context, even inside "...": remember the outer
    // quote, what closes the substitution, and how many plain ( are open inside it
    let mut subs: Vec<(Option<char>, char, usize)> = Vec::new();
    'scan: while let Some(c) = rest.chars().next() {
        for (token, var) in VARS {
            if rest.starts_with(token) {
                match quote {
                    Some('"') => out += &format!("${{{var}}}"),
                    Some(_) => out += &format!("'\"${{{var}}}\"'"),
                    None => out += &format!("\"${{{var}}}\""),
                }
                rest = &rest[token.len()..];
                continue 'scan;
            }
        }
        match (c, quote) {
            ('\\', q) if q != Some('\'') => {
                let mut it = rest.chars();
                out.push(it.next().unwrap_or(c));
                if let Some(n) = it.next() {
                    out.push(n);
                }
                rest = it.as_str();
                continue;
            }
            ('$', None | Some('"')) if rest.starts_with("$(") && !rest.starts_with("$((") => {
                subs.push((quote, ')', 0));
                quote = None;
                out += "$(";
                rest = &rest[2..];
                continue;
            }
            ('`', None) if subs.last().is_some_and(|f| f.1 == '`') => quote = subs.pop().and_then(|f| f.0),
            ('`', None | Some('"')) => {
                subs.push((quote, '`', 0));
                quote = None;
            }
            ('(', None) => {
                if let Some(f) = subs.last_mut() {
                    f.2 += 1;
                }
            }
            (')', None) if subs.last().is_some_and(|f| f.1 == ')') => match subs.last_mut() {
                Some(f) if f.2 > 0 => f.2 -= 1,
                _ => quote = subs.pop().and_then(|f| f.0),
            },
            ('"' | '\'', None) => quote = Some(c),
            (_, Some(q)) if c == q => quote = None,
            _ => {}
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

fn merge(paths: &[PathBuf], progress: Progress) -> Result<Vec<PathBuf>> {
    if paths.len() < 2 {
        bail!("merge needs at least two files");
    }
    let kinds: Vec<Kind> = paths.iter().map(|p| kind(p)).collect();
    let all = |k: Kind| kinds.iter().all(|x| *x == k);
    if all(Kind::Video) || all(Kind::Audio) {
        media::merge(paths, progress)
    } else if kinds.iter().all(|k| matches!(k, Kind::Pdf | Kind::Image)) {
        pdf::merge(paths, progress)
    } else {
        bail!("these files can't be merged together")
    }
}
