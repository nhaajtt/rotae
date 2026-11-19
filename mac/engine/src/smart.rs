//! Work that takes judgement: hiding private details, and the language model that ships with
//! macOS. Both run in `sd-native`, on this Mac, with nothing sent anywhere.
use crate::engines::note;
use crate::naming::{ext, out_path, temp_file};
use crate::native::call_within;
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn run(paths: &[PathBuf], verb: &str, arg: &str) -> Option<Result<Vec<PathBuf>>> {
    let src = paths.first()?;
    Some(match (verb, arg) {
        ("redact", _) => redact(src),
        ("think", "summary") => think(src, "summary", "Summary", "md"),
        ("think", "actions") => think(src, "actions", "To do", "md"),
        ("think", "name") => name(src),
        ("think", a) if a.starts_with("translate:") => translate(src, &a[10..]),
        ("ask", q) => ask(src, q),
        // the column comes typed by the user, so it travels as the whole argument
        ("split-by", header) => return crate::ledger::run(paths, "ledger", &format!("split={}", header.trim())),
        _ => return None,
    })
}

fn redact(src: &Path) -> Result<Vec<PathBuf>> {
    let e = match ext(src).as_str() {
        "pdf" => "pdf",
        "jpg" | "jpeg" => "jpg",
        _ => "png",
    };
    let out = out_path(src, Some("Redacted"), e);
    let n = call_within(&["redact".as_ref(), src.as_os_str(), out.as_os_str()], Duration::from_secs(1800))?;
    note(&format!("{} private details hidden", String::from_utf8_lossy(&n).trim()));
    Ok(vec![out])
}

fn model(mode: &str, src: &Path, out: &Path) -> Result<String> {
    call_within(&["think".as_ref(), mode.as_ref(), src.as_os_str(), out.as_os_str()], Duration::from_secs(1800))?;
    Ok(std::fs::read_to_string(out)?)
}

fn think(src: &Path, mode: &str, suffix: &str, e: &str) -> Result<Vec<PathBuf>> {
    let out = out_path(src, Some(suffix), e);
    model(mode, src, &out)?;
    Ok(vec![out])
}

/// A copy under a name that says what the document is; the original keeps its own.
fn name(src: &Path) -> Result<Vec<PathBuf>> {
    let tmp = temp_file("txt");
    let said = model("name", src, &tmp);
    let _ = std::fs::remove_file(&tmp);
    // whatever the model wrote becomes one plain file name: no separators, no dots in front
    let line = said?.lines().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
    let clean: String = line.chars().map(|c| if matches!(c, '/' | ':' | '\\' | '\0'..='\x1f' | '"' | '*' | '?' | '<' | '>' | '|') { ' ' } else { c }).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean: String = clean.trim_start_matches('.').trim().chars().take(90).collect();
    if clean.chars().filter(|c| c.is_alphanumeric()).count() < 3 {
        bail!("the answer was empty");
    }
    let e = ext(src);
    let dir = src.parent().unwrap_or(Path::new("."));
    // `unique` has already claimed the name with an empty file, which the copy then fills
    let out = crate::naming::unique(dir, &clean, &e);
    std::fs::copy(src, &out)?;
    note(&format!("Copy named: {clean}"));
    Ok(vec![out])
}

fn translate(src: &Path, lang: &str) -> Result<Vec<PathBuf>> {
    // the language ends up in a file name and in the instructions: letters and spaces only
    let lang: String = lang.chars().filter(|c| c.is_alphabetic() || *c == ' ').take(40).collect();
    if lang.trim().is_empty() {
        bail!("not available");
    }
    let e = ext(src);
    think(src, &format!("translate:{}", lang.trim()), lang.trim(), if matches!(e.as_str(), "txt" | "md" | "markdown") { &e } else { "txt" })
}

fn ask(src: &Path, question: &str) -> Result<Vec<PathBuf>> {
    let q = question.trim();
    if q.is_empty() {
        bail!("there was no question");
    }
    // by file, not by argument: other programs can read a process's arguments
    let qf = temp_file("txt");
    std::fs::write(&qf, q)?;
    let out = out_path(src, Some("Answer"), "md");
    let r = model(&format!("ask:{}", qf.display()), src, &out);
    let _ = std::fs::remove_file(&qf);
    let answer = r?;
    std::fs::write(&out, format!("**{}**\n\n{}", q.replace('\n', " "), answer))?;
    crate::clipboard::set(answer.trim())?;
    let line: String = answer.lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(140).collect();
    note(&line);
    Ok(vec![out])
}
