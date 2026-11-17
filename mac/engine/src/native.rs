//! Apple's frameworks, reached through the bundled `sd-native` tool so that they work inside
//! recipes and cancel along with everything else in the process group.
use crate::engines::{note, Action};
use crate::naming::{ext, out_path};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub fn path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SD_NATIVE") {
        return Some(PathBuf::from(p));
    }
    std::env::current_exe().ok()?.parent().map(|d| d.join("sd-native")).filter(|p| p.is_file())
}

fn call(args: &[&std::ffi::OsStr]) -> Result<Vec<u8>> {
    call_within(args, Duration::from_secs(120))
}

pub fn fit(side: u32, src: &Path, out: &Path) -> Result<()> {
    call(&["fit".as_ref(), side.to_string().as_ref(), src.as_os_str(), out.as_os_str()]).map(|_| ())
}

pub fn call_within(args: &[&std::ffi::OsStr], limit: Duration) -> Result<Vec<u8>> {
    let tool = path().context("sd-native is missing from the app")?;
    let o = crate::engines::tools::output_within(Command::new(tool).args(args), limit)?;
    if !o.status.success() {
        bail!("{}", String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(o.stdout)
}

pub fn decode(src: &Path) -> Result<Vec<u8>> {
    let tmp = crate::naming::temp_file("png");
    call(&["decode".as_ref(), src.as_os_str(), tmp.as_os_str()])?;
    let b = std::fs::read(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    Ok(b)
}

pub fn ocr(src: &Path) -> Result<String> {
    Ok(String::from_utf8_lossy(&call(&["ocr".as_ref(), src.as_os_str()])?).into_owned())
}

pub fn signature(p: &Path) -> String {
    match call(&["signature".as_ref(), p.as_os_str()]) {
        Ok(o) => String::from_utf8_lossy(&o).trim().to_string(),
        Err(e) => format!("couldn't check: {e}"),
    }
}

/// Vision does these on the device with nothing to download.
pub fn run(paths: &[PathBuf], action: &Action) -> Option<Result<Vec<PathBuf>>> {
    let src = paths.first()?;
    match action {
        Action::RemoveBg => {
            Some((|| {
                let out = out_path(src, Some("No background"), "png");
                call(&["subject".as_ref(), src.as_os_str(), out.as_os_str()])?;
                Ok(vec![out])
            })())
        }
        Action::BlurFaces => {
            Some((|| {
                let e = if matches!(ext(src).as_str(), "jpg" | "jpeg") { "jpg" } else { "png" };
                let out = out_path(src, Some("Faces blurred"), e);
                let n = call(&["faces".as_ref(), src.as_os_str(), out.as_os_str()])?;
                let n = String::from_utf8_lossy(&n).trim().parse::<u32>().unwrap_or(0);
                if n == 0 {
                    bail!("no faces found in this picture");
                }
                note(&format!("{n} faces blurred"));
                Ok(vec![out])
            })())
        }
        Action::Native { verb, arg } => crate::desk::run(paths, verb, arg).or_else(|| crate::smart::run(paths, verb, arg)).or_else(|| Some(native(src, verb, arg))),
        _ => None,
    }
}

fn native(src: &Path, verb: &str, arg: &str) -> Result<Vec<PathBuf>> {
    match verb {
        "filter" | "fit" => {
            let (suffix, e) = (if verb == "fit" { format!("{arg}px") } else { title_case(arg) }, if matches!(ext(src).as_str(), "jpg" | "jpeg") { "jpg" } else { "png" });
            let out = out_path(src, Some(&suffix), e);
            call(&[verb.as_ref(), arg.as_ref(), src.as_os_str(), out.as_os_str()])?;
            Ok(vec![out])
        }
        "text" => text(src, arg),
        // a scan or photo of a page becomes a PDF whose text can be searched, selected and copied
        "ocrpdf" => {
            let out = out_path(src, Some("Searchable"), "pdf");
            let n = call_within(&["ocrpdf".as_ref(), src.as_os_str(), out.as_os_str()], Duration::from_secs(1800))?;
            note(&format!("{} words recognised", String::from_utf8_lossy(&n).trim()));
            Ok(vec![out])
        }
        _ => bail!("not available"),
    }
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn text(src: &Path, how: &str) -> Result<Vec<PathBuf>> {
    if std::fs::metadata(src)?.len() > 50 << 20 {
        bail!("this file is too big for this (over 50 MB)");
    }
    let body = String::from_utf8(std::fs::read(src)?).map_err(|_| anyhow::anyhow!("not a text file"))?;
    if how == "count" {
        note(&format!("{} lines, {} words, {} characters", body.lines().count(), body.split_whitespace().count(), body.chars().count()));
        return Ok(vec![]);
    }
    let (suffix, result) = match how {
        "sort" => {
            let mut l: Vec<&str> = body.lines().collect();
            l.sort_unstable();
            ("Sorted", l.join("\n") + "\n")
        }
        "dedupe" => {
            let mut seen = std::collections::HashSet::new();
            ("Unique", body.lines().filter(|l| seen.insert(*l)).collect::<Vec<_>>().join("\n") + "\n")
        }
        "upper" => ("Uppercase", body.to_uppercase()),
        "lower" => ("Lowercase", body.to_lowercase()),
        "trim" => ("Trimmed", body.lines().map(str::trim_end).collect::<Vec<_>>().join("\n") + "\n"),
        // what a file is saved with decides whether git, a shell script or Windows Notepad accepts it
        "lf" => ("LF", body.replace("\r\n", "\n").replace('\r', "\n")),
        "crlf" => ("CRLF", body.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n")),
        _ => bail!("not available"),
    };
    let out = out_path(src, Some(suffix), &ext(src));
    std::fs::write(&out, result)?;
    Ok(vec![out])
}

/// A small archive can unpack to fill the disk; look at what it claims before starting.
pub fn check_archive(src: &Path, dir: &Path) -> Result<()> {
    let o = crate::engines::tools::output_within(Command::new("/usr/bin/tar").arg("-tvf").arg(src), Duration::from_secs(120))?;
    if !o.status.success() {
        bail!("can't extract: {}", String::from_utf8_lossy(&o.stderr).trim());
    }
    let (mut files, mut bytes) = (0u64, 0u64);
    for line in String::from_utf8_lossy(&o.stdout).lines() {
        files += 1;
        // mode links owner group size month day time name
        bytes = bytes.saturating_add(line.split_whitespace().nth(4).and_then(|s| s.parse().ok()).unwrap_or(0));
    }
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    let c = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(dir.as_os_str()))?;
    let free = if unsafe { libc::statfs(c.as_ptr(), &mut st) } == 0 { st.f_bavail.saturating_mul(st.f_bsize as u64) } else { u64::MAX };
    if files > 500_000 {
        bail!("this archive is too big for this ({files} files)");
    }
    if bytes > free {
        bail!("No space left: this archive needs more room than the disk has");
    }
    Ok(())
}

pub fn carry_quarantine(src: &Path, out: &Path) {
    let xattr = "/usr/bin/xattr";
    let Ok(o) = Command::new(xattr).args(["-p", "com.apple.quarantine"]).arg(src).output() else { return };
    if o.status.success() {
        let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
        let _ = Command::new(xattr).args(["-r", "-w", "com.apple.quarantine", &v]).arg(out).output();
    }
}
