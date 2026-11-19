//! Chores around writing software: app icons, a look through a project, a log or a package,
//! credentials left lying in files, and types written from data. Nothing here runs or unpacks
//! what it is given: files are read, counted and described.
use crate::desk::{attr, entry, json};
use crate::engines::note;
use crate::naming::{ext, out_dir, out_path, stem, temp_file, unique};
use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::{BytesStart, Event};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, HashMap},
    io::{BufRead, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub fn run(paths: &[PathBuf], verb: &str, arg: &str) -> Option<Result<Vec<PathBuf>>> {
    if verb != "kit" {
        return None;
    }
    let src = paths.first()?;
    Some(match arg {
        "iconset" => iconset(src),
        "project" => project(src),
        "secrets" => secrets(src),
        "pack" => pack(src),
        "log" => log(src),
        "swiftui" => swiftui(src),
        "cssvars" => cssvars(src),
        "bundle" => bundle(src),
        _ => match arg.strip_prefix("struct=") {
            Some(lang @ ("swift" | "rust" | "go" | "schema")) => structs(src, lang),
            _ => return None,
        },
    })
}

/// Every action here works on one file or folder at a time.
pub fn is_group(_verb: &str, _arg: &str) -> bool {
    false
}

// ---------------------------------------------------------------- shared

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "folder".into())
}

/// A result next to a file or a folder. `stem` would cut the folder "app.v2" down to "app".
fn beside(src: &Path, suffix: &str, e: &str) -> Result<PathBuf> {
    let base = if src.is_dir() { name_of(src) } else { stem(src) };
    Ok(unique(src.parent().context("no folder")?, &format!("{base}{suffix}"), e))
}

fn count(n: u64) -> String {
    let d = n.to_string();
    let mut o = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i).is_multiple_of(3) {
            o.push(',');
        }
        o.push(c);
    }
    o
}

fn size(b: u64) -> String {
    match b {
        0..=999 => format!("{b} bytes"),
        1_000..=999_999 => format!("{:.1} KB", b as f64 / 1e3),
        1_000_000..=999_999_999 => format!("{:.1} MB", b as f64 / 1e6),
        _ => format!("{:.2} GB", b as f64 / 1e9),
    }
}

/// Names and lines come from strangers: one holding a newline, a backtick or a text-direction
/// mark must not be able to redraw the report it is quoted in.
fn code(s: &str) -> String {
    let clean: String = s
        .chars()
        .take(300)
        .map(|c| match c {
            '`' => '\'',
            c if c.is_control() || matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}') => ' ',
            c => c,
        })
        .collect();
    format!("`{}`", if clean.trim().is_empty() { "(no name)" } else { clean.trim() })
}

fn clip(text: &str) -> Result<bool> {
    if text.len() > 1 << 20 {
        return Ok(false);
    }
    crate::clipboard::set(text)?;
    Ok(true)
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().into_owned()
}

const HEAVY: [&str; 11] = [".git", "node_modules", "target", "build", ".build", "dist", "Pods", "DerivedData", "venv", ".venv", "__pycache__"];
const MAX_ENTRIES: usize = 200_000;

#[derive(Default)]
struct Walk {
    files: Vec<(PathBuf, u64)>,
    left: BTreeMap<&'static str, u64>,
    links: u64,
    odd: u64,
    unreadable: u64,
    capped: bool,
}

/// Every regular file under `root`. Links are counted and never followed, so a loop or a link
/// out of the folder leads nowhere; pipes and devices are never opened (a read would not return).
fn walk(root: &Path, skip: &dyn Fn(&str, bool) -> Option<&'static str>) -> Walk {
    let mut w = Walk::default();
    let mut todo = vec![(root.to_path_buf(), 0u32)];
    let mut seen = 0usize;
    'all: while let Some((dir, depth)) = todo.pop() {
        let Ok(list) = std::fs::read_dir(&dir) else {
            w.unreadable += 1;
            continue;
        };
        for e in list {
            seen += 1;
            if seen > MAX_ENTRIES {
                w.capped = true;
                break 'all;
            }
            let Some((e, t)) = e.ok().and_then(|e| e.file_type().ok().map(|t| (e, t))) else {
                w.unreadable += 1;
                continue;
            };
            if let Some(why) = skip(&e.file_name().to_string_lossy(), t.is_dir()) {
                *w.left.entry(why).or_default() += 1;
            } else if t.is_symlink() {
                w.links += 1;
            } else if t.is_dir() {
                if depth >= 64 {
                    w.unreadable += 1;
                } else {
                    todo.push((e.path(), depth + 1));
                }
            } else if t.is_file() {
                w.files.push((e.path(), e.metadata().map(|m| m.len()).unwrap_or(0)));
            } else {
                w.odd += 1;
            }
        }
    }
    w.files.sort();
    w
}

fn heavy(name: &str, dir: bool) -> Option<&'static str> {
    if dir { HEAVY.iter().find(|h| **h == name).copied() } else { None }
}

// ---------------------------------------------------------------- icons

fn iconset(src: &Path) -> Result<Vec<PathBuf>> {
    use image::imageops::{self, FilterType::Lanczos3};
    let img = crate::engines::image::open(src)?;
    let (w, h) = (img.width(), img.height());
    if w.max(h) < 256 {
        bail!("this picture is too small for app icons ({w}×{h}): it needs at least 256 pixels on its longer side");
    }
    // nothing bigger than 1024 is written, and a huge picture must not become a huger square
    let img = if w.max(h) > 1024 { img.resize(1024, 1024, Lanczos3) } else { img }.to_rgba8();
    let side = img.width().max(img.height());
    let mut sq = image::RgbaImage::new(side, side);
    imageops::overlay(&mut sq, &img, ((side - img.width()) / 2) as i64, ((side - img.height()) / 2) as i64);
    let mut made: BTreeMap<u32, image::RgbaImage> = BTreeMap::new();
    let mut sized = |n: u32| made.entry(n).or_insert_with(|| if n == side { sq.clone() } else { imageops::resize(&sq, n, n, Lanczos3) }).clone();

    let dir = out_dir(src, "Icons")?;
    let (app, web) = (dir.join("AppIcon.appiconset"), dir.join("web"));
    std::fs::create_dir_all(&app)?;
    std::fs::create_dir_all(&web)?;
    let mut images = Vec::new();
    for pt in [16u32, 32, 128, 256, 512] {
        for scale in [1u32, 2] {
            let file = format!("icon_{pt}x{pt}{}.png", if scale == 2 { "@2x" } else { "" });
            sized(pt * scale).save(app.join(&file))?;
            images.push(serde_json::json!({"filename": file, "idiom": "mac", "scale": format!("{scale}x"), "size": format!("{pt}x{pt}")}));
        }
    }
    // the App Store refuses an iOS icon with transparency, so this one sits on white
    let mut ios = image::RgbImage::from_pixel(1024, 1024, image::Rgb([255, 255, 255]));
    for (o, p) in ios.pixels_mut().zip(sized(1024).pixels()) {
        let a = p[3] as u32;
        *o = image::Rgb([0, 1, 2].map(|i| ((p[i] as u32 * a + 255 * (255 - a)) / 255) as u8));
    }
    ios.save(app.join("icon_ios_1024.png"))?;
    images.push(serde_json::json!({"filename": "icon_ios_1024.png", "idiom": "universal", "platform": "ios", "size": "1024x1024"}));
    std::fs::write(app.join("Contents.json"), serde_json::to_string_pretty(&serde_json::json!({"images": images, "info": {"author": "xcode", "version": 1}}))? + "\n")?;

    use image::codecs::ico::{IcoEncoder, IcoFrame};
    let frames = [16u32, 32, 48].iter().map(|&n| IcoFrame::as_png(sized(n).as_raw(), n, n, image::ExtendedColorType::Rgba8)).collect::<Result<Vec<_>, _>>()?;
    IcoEncoder::new(std::fs::File::create(web.join("favicon.ico"))?).encode_images(&frames)?;
    for (n, file) in [(180, "apple-touch-icon.png"), (192, "icon-192.png"), (512, "icon-512.png")] {
        sized(n).save(web.join(file))?;
    }
    let manifest = serde_json::json!({"name": stem(src), "icons": [{"src": "icon-192.png", "sizes": "192x192", "type": "image/png"}, {"src": "icon-512.png", "sizes": "512x512", "type": "image/png"}], "display": "standalone"});
    std::fs::write(web.join("site.webmanifest"), serde_json::to_string_pretty(&manifest)? + "\n")?;
    note("AppIcon.appiconset for Xcode, and favicons for the web");
    Ok(vec![dir])
}

/// The picture's main colours as CSS custom properties.
fn cssvars(src: &Path) -> Result<Vec<PathBuf>> {
    let img = crate::engines::image::open(src)?.thumbnail(160, 160).to_rgba8();
    // what can't be seen has no colour: a logo on transparency is not mostly black
    let px: Vec<[f32; 3]> = img.pixels().filter(|p| p[3] >= 128).map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
    if px.is_empty() {
        bail!("this picture has nothing visible in it");
    }
    let k = 6.min(px.len());
    let far = |a: &[f32; 3], b: &[f32; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>();
    let mut centers: Vec<[f32; 3]> = (0..k).filter_map(|i| px.get(i * px.len() / k).copied()).collect();
    let mut counts = vec![0usize; centers.len()];
    for _ in 0..12 {
        let mut sums = vec![[0f32; 3]; centers.len()];
        counts.iter_mut().for_each(|c| *c = 0);
        for p in &px {
            let Some(c) = (0..centers.len()).min_by(|&a, &b| far(p, &centers[a]).total_cmp(&far(p, &centers[b]))) else { continue };
            counts[c] += 1;
            for i in 0..3 {
                sums[c][i] += p[i];
            }
        }
        for (c, sum) in sums.iter().enumerate() {
            if counts[c] > 0 {
                centers[c] = sum.map(|v| v / counts[c] as f32);
            }
        }
    }
    let mut order: Vec<usize> = (0..centers.len()).filter(|&c| counts[c] > 0).collect();
    order.sort_by_key(|&c| std::cmp::Reverse(counts[c]));
    let mut hex: Vec<String> = Vec::new();
    for c in order {
        let h = format!("#{:02X}{:02X}{:02X}", centers[c][0].round() as u8, centers[c][1].round() as u8, centers[c][2].round() as u8);
        if !hex.contains(&h) {
            hex.push(h);
        }
    }
    let css = format!(":root {{\n{}}}\n", hex.iter().enumerate().map(|(i, h)| format!("  --color-{}: {h};\n", i + 1)).collect::<String>());
    crate::clipboard::set(&css)?;
    note(&format!("CSS variables copied: {}", hex.join(" ")));
    Ok(vec![])
}

// ---------------------------------------------------------------- a project, counted

fn language(p: &Path) -> &'static str {
    match ext(p).as_str() {
        "rs" => "Rust",
        "swift" => "Swift",
        "py" | "pyi" => "Python",
        "js" | "mjs" | "cjs" | "jsx" => "JavaScript",
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "go" => "Go",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "c" => "C",
        "h" => "C/C++ header",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "C++",
        "cs" => "C#",
        "m" | "mm" => "Objective-C",
        "rb" => "Ruby",
        "php" => "PHP",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "scss" | "sass" | "less" => "Sass/Less",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "json" => "JSON",
        "yaml" | "yml" => "YAML",
        "toml" => "TOML",
        "xml" | "plist" | "xib" | "storyboard" => "XML",
        "md" | "markdown" => "Markdown",
        "sh" | "bash" | "zsh" | "fish" => "Shell",
        "sql" => "SQL",
        "lua" => "Lua",
        "dart" => "Dart",
        "scala" => "Scala",
        "ex" | "exs" => "Elixir",
        "hs" => "Haskell",
        "r" => "R",
        "pl" | "pm" => "Perl",
        "zig" => "Zig",
        "gradle" => "Gradle",
        "tf" => "Terraform",
        "proto" => "Protobuf",
        "graphql" | "gql" => "GraphQL",
        "csv" | "tsv" => "CSV",
        "txt" => "Text",
        "" => match name_of(p).as_str() {
            "Makefile" | "makefile" | "GNUmakefile" => "Makefile",
            "Dockerfile" => "Dockerfile",
            _ => "Other",
        },
        _ => "Other",
    }
}

enum Lines {
    Text(u64),
    Binary,
    Unread,
}

/// A zero byte near the start is how git and grep tell a binary file, too.
fn lines(p: &Path, buf: &mut [u8]) -> Lines {
    let Ok(mut f) = std::fs::File::open(p) else { return Lines::Unread };
    let (mut n, mut last, mut first) = (0u64, b'\n', true);
    loop {
        match f.read(buf) {
            Ok(0) => break,
            Ok(k) => {
                let chunk = buf.get(..k).unwrap_or_default();
                if first && chunk.iter().take(8000).any(|b| *b == 0) {
                    return Lines::Binary;
                }
                first = false;
                n += chunk.iter().filter(|b| **b == b'\n').count() as u64;
                last = chunk.last().copied().unwrap_or(last);
            }
            Err(_) => return Lines::Unread,
        }
    }
    Lines::Text(if last == b'\n' { n } else { n + 1 })
}

fn project(src: &Path) -> Result<Vec<PathBuf>> {
    if !src.is_dir() {
        bail!("drop a folder for this");
    }
    let w = walk(src, &heavy);
    if w.files.is_empty() {
        bail!("no files found in this folder");
    }
    let mut langs: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    let (mut binary, mut unread, mut uncounted, mut all_lines, mut bytes) = (0u64, w.unreadable, 0u64, 0u64, 0u64);
    // a folder of disk images must not turn counting lines into reading a terabyte
    let mut budget = 8u64 << 30;
    let mut buf = vec![0u8; 1 << 16];
    for (p, len) in &w.files {
        bytes = bytes.saturating_add(*len);
        if *len > budget {
            uncounted += 1;
            continue;
        }
        budget -= len;
        match lines(p, &mut buf) {
            Lines::Text(n) => {
                let l = langs.entry(language(p)).or_default();
                l.0 += 1;
                l.1 += n;
                all_lines += n;
            }
            Lines::Binary => binary += 1,
            Lines::Unread => unread += 1,
        }
    }
    let mut by: Vec<(&str, (u64, u64))> = langs.into_iter().collect();
    by.sort_by_key(|l| std::cmp::Reverse((l.1 .1, l.1 .0)));
    let mut md = format!("# {}\n\n{} files · {} lines · {}\n\n## Languages\n\n| Language | Files | Lines |\n|---|---:|---:|\n", code(&name_of(src)), count(w.files.len() as u64), count(all_lines), size(bytes));
    for (name, (files, n)) in by.iter().take(12) {
        md.push_str(&format!("| {name} | {} | {} |\n", count(*files), count(*n)));
    }
    if by.len() > 12 {
        let rest = by.iter().skip(12).fold((0, 0), |a, l| (a.0 + l.1 .0, a.1 + l.1 .1));
        md.push_str(&format!("| {} more | {} | {} |\n", by.len() - 12, count(rest.0), count(rest.1)));
    }
    md.push_str(&format!("| **Total** | {} | {} |\n\n## Largest files\n\n", count(by.iter().map(|l| l.1 .0).sum()), count(all_lines)));
    let mut big: Vec<&(PathBuf, u64)> = w.files.iter().collect();
    big.sort_by_key(|f| std::cmp::Reverse(f.1));
    for (p, len) in big.into_iter().take(10) {
        md.push_str(&format!("- {} — {}\n", code(&rel(src, p)), size(*len)));
    }
    let mut left: Vec<String> = w.left.iter().map(|(k, n)| format!("{} × {} (not looked into)", count(*n), code(k))).collect();
    for (n, what) in [
        (binary, "binary files (counted as files, no lines)"),
        (uncounted, "files too large to count lines in"),
        (w.links, "links (not followed)"),
        (w.odd, "pipes, sockets or devices"),
        (unread, "files or folders that couldn't be read"),
    ] {
        if n > 0 {
            left.push(format!("{} {what}", count(n)));
        }
    }
    if w.capped {
        left.push(format!("everything after the first {} entries: this folder is too large to count whole", count(MAX_ENTRIES as u64)));
    }
    if !left.is_empty() {
        md.push_str(&format!("\n## Left out\n\n{}", left.iter().map(|l| format!("- {l}\n")).collect::<String>()));
    }
    let out = beside(src, " Report", "md")?;
    std::fs::write(&out, md)?;
    let mostly = by.first().map(|l| format!(", mostly {}", l.0)).unwrap_or_default();
    note(&format!("{} files, {} lines{mostly}", count(w.files.len() as u64), count(all_lines)));
    Ok(vec![out])
}

// ---------------------------------------------------------------- credentials left in files

pub(crate) struct Hit {
    line: usize,
    kind: &'static str,
    shown: String,
}

const AWS_ID: &str = "AWS access key ID";
const AWS_SECRET: &str = "AWS secret key";
const ASSIGNED: &str = "Password or token";

/// What a report may show of a secret: enough to find it, never enough to use it.
fn mask(v: &str) -> String {
    let n = v.chars().count();
    format!("{}…", v.chars().take(if n >= 12 { 4 } else { n / 3 }).collect::<String>())
}

fn word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn b64u(c: u8) -> bool {
    word(c) || c == b'-'
}

/// Each place `prefix` begins a word, with the run of `ok` bytes that follows it.
fn after<'a>(line: &'a str, prefix: &str, ok: impl Fn(u8) -> bool) -> Vec<&'a str> {
    let b = line.as_bytes();
    let (mut out, mut from) = (Vec::new(), 0);
    while let Some(i) = line.get(from..).and_then(|s| s.find(prefix)).map(|i| i + from) {
        let start = i + prefix.len();
        let end = start + b.get(start..).unwrap_or_default().iter().take_while(|c| ok(**c)).count();
        if i == 0 || !b.get(i - 1).is_some_and(|c| word(*c)) {
            out.extend(line.get(i..end));
        }
        from = end;
    }
    out
}

/// Tokens whose shape alone says what they are.
fn known(line: &str) -> Vec<(&'static str, String)> {
    let mut hits = Vec::new();
    let alnum = |c: u8| c.is_ascii_alphanumeric();
    let mixed = |t: &str| t.bytes().any(|c| c.is_ascii_digit()) && t.bytes().any(|c| c.is_ascii_uppercase()) && t.bytes().any(|c| c.is_ascii_lowercase());
    if line.split_once("-----BEGIN ").and_then(|(_, r)| r.split_once("-----")).is_some_and(|(what, _)| what.ends_with("PRIVATE KEY") || what.ends_with("PRIVATE KEY BLOCK")) {
        hits.push(("Private key", "(key block)".to_string()));
    }
    for p in ["AKIA", "ASIA"] {
        // AWS's own documentation key ends in EXAMPLE
        hits.extend(after(line, p, |c| c.is_ascii_uppercase() || c.is_ascii_digit()).into_iter().filter(|t| t.len() == 20 && !t.ends_with("EXAMPLE")).map(|t| (AWS_ID, mask(t))));
    }
    for p in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
        hits.extend(after(line, p, alnum).into_iter().filter(|t| t.len() >= 40).map(|t| ("GitHub token", mask(t))));
    }
    hits.extend(after(line, "github_pat_", word).into_iter().filter(|t| t.len() >= 60).map(|t| ("GitHub token", mask(t))));
    hits.extend(after(line, "glpat-", b64u).into_iter().filter(|t| t.len() >= 26).map(|t| ("GitLab token", mask(t))));
    for p in ["xoxb-", "xoxa-", "xoxp-", "xoxr-", "xoxs-"] {
        hits.extend(after(line, p, |c| c.is_ascii_alphanumeric() || c == b'-').into_iter().filter(|t| t.len() >= 25 && t.bytes().any(|c| c.is_ascii_digit())).map(|t| ("Slack token", mask(t))));
    }
    for p in ["sk_live_", "rk_live_"] {
        hits.extend(after(line, p, alnum).into_iter().filter(|t| t.len() >= 28).map(|t| ("Stripe live key", mask(t))));
    }
    hits.extend(after(line, "AIza", b64u).into_iter().filter(|t| t.len() == 39).map(|t| ("Google API key", mask(t))));
    // a long hyphenated CSS class also starts with sk-; a key has digits and both cases
    hits.extend(after(line, "sk-", b64u).into_iter().filter(|t| t.len() >= 35 && mixed(t)).map(|t| (if t.starts_with("sk-ant-") { "Anthropic API key" } else { "OpenAI-style API key" }, mask(t))));
    for t in after(line, "eyJ", |c| b64u(c) || c == b'.') {
        if let [head, body, sig, ..] = t.split('.').collect::<Vec<_>>().as_slice() {
            if head.len() >= 10 && body.len() >= 10 && body.starts_with("eyJ") && sig.len() >= 10 {
                hits.push(("JSON Web Token", mask(t)));
            }
        }
    }
    hits
}

/// Forty characters of base64 with digits and both cases: what an AWS secret looks like. Only
/// believed next to a key ID or a name that says so, since much else looks like this.
fn aws_secret(line: &str) -> Option<&str> {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '/' || c == '+'))
        .find(|t| t.len() == 40 && t.bytes().any(|c| c.is_ascii_digit()) && t.bytes().any(|c| c.is_ascii_uppercase()) && t.bytes().any(|c| c.is_ascii_lowercase()))
}

const NAMES: [&str; 13] = ["password", "passwd", "secret", "token", "apikey", "api_key", "api-key", "secretkey", "secret_key", "accesskey", "access_key", "privatekey", "private_key"];

fn placeholder(v: &str) -> bool {
    let l = v.to_lowercase();
    v.chars().count() < 6
        || v.contains(char::is_whitespace)
        || v.starts_with(['$', '%', '@', '{', '<'])
        || ["changeme", "change_me", "change-me", "example", "placeholder", "your_", "your-", "<", "${", "$(", "{{", "process.env", "os.environ", "getenv", "xxx", "***", "...", "dummy", "redacted", "todo", "_here", "insert"].iter().any(|p| l.contains(p))
        || ["password", "passwd", "secret", "token", "apikey", "api_key", "bearer", "string", "undefined", "testing", "foobar"].contains(&l.as_str())
        || l.chars().all(|c| l.starts_with(c))
}

/// The literal a name is set to, when the name ends at `at`: `token_type = "Bearer"` is no token.
fn value_after(line: &str, at: usize, bare: bool) -> Option<&str> {
    let r = line.get(at..)?;
    let r = r.strip_prefix(['"', '\'', '`']).unwrap_or(r);
    let r = r.strip_prefix(']').unwrap_or(r).trim_start_matches([' ', '\t']);
    let r = r.strip_prefix(":=").or_else(|| r.strip_prefix("=>")).or_else(|| r.strip_prefix('=')).or_else(|| r.strip_prefix(':'))?;
    // == is a comparison
    if r.starts_with('=') {
        return None;
    }
    let r = r.trim_start_matches([' ', '\t']);
    let q = r.chars().next()?;
    if matches!(q, '"' | '\'' | '`') {
        let body = r.get(1..)?;
        return body.find(q).and_then(|e| body.get(..e));
    }
    if bare { r.split([' ', '\t', '#']).next().filter(|v| !v.is_empty()) } else { None }
}

/// `password = "…"` and its cousins. `bare` also takes unquoted values, as .env files write them.
fn assigned(line: &str, bare: bool) -> Option<&str> {
    // ASCII lowering keeps every byte where it was
    let low = line.to_ascii_lowercase();
    for key in NAMES {
        let mut from = 0;
        while let Some(i) = low.get(from..).and_then(|s| s.find(key)).map(|i| i + from) {
            from = i + key.len();
            if let Some(v) = value_after(line, from, bare).filter(|v| !placeholder(v)) {
                return Some(v);
            }
        }
    }
    None
}

pub(crate) fn scan(text: &str, bare: bool) -> Vec<Hit> {
    let (mut hits, mut ids) = (Vec::new(), Vec::new());
    for (i, l) in text.lines().enumerate() {
        let found = known(l);
        if found.iter().any(|f| f.0 == AWS_ID) {
            ids.push(i);
        }
        if found.is_empty() {
            hits.extend(assigned(l, bare).map(|v| Hit { line: i + 1, kind: ASSIGNED, shown: mask(v) }));
        }
        hits.extend(found.into_iter().map(|(kind, shown)| Hit { line: i + 1, kind, shown }));
    }
    for (i, l) in text.lines().enumerate() {
        let k = ids.partition_point(|x| x + 3 < i);
        if ids.get(k).is_some_and(|x| *x <= i + 3) || l.to_ascii_lowercase().contains("secret_access_key") {
            if let Some(t) = aws_secret(l) {
                // said once, as what it is
                hits.retain(|h| !(h.line == i + 1 && h.kind == ASSIGNED));
                hits.push(Hit { line: i + 1, kind: AWS_SECRET, shown: mask(t) });
            }
        }
    }
    hits.sort_by_key(|h| h.line);
    hits
}

fn secrets(src: &Path) -> Result<Vec<PathBuf>> {
    let dir = src.is_dir();
    let (files, capped) = if dir {
        let w = walk(src, &heavy);
        (w.files, w.capped)
    } else {
        (vec![(src.to_path_buf(), std::fs::metadata(src)?.len())], false)
    };
    let (mut read, mut with, mut budget, mut stopped) = (0u64, 0u64, 2u64 << 30, capped);
    let mut found: Vec<(String, Hit)> = Vec::new();
    for (p, len) in &files {
        // keys live in small files; minified bundles and data dumps are neither small nor theirs
        if *len > 5 << 20 {
            if !dir {
                bail!("this file is too big for this (over 5 MB)");
            }
            continue;
        }
        if *len > budget || found.len() >= 5000 {
            stopped = true;
            break;
        }
        let Ok(bytes) = std::fs::read(p) else { continue };
        if bytes.iter().take(8000).any(|b| *b == 0) {
            continue;
        }
        budget = budget.saturating_sub(bytes.len() as u64);
        read += 1;
        let name = name_of(p);
        let env = name == ".env" || name.strip_prefix(".env.").is_some_and(|r| !["example", "sample", "template", "dist", "defaults"].contains(&r));
        let mut hits = scan(&String::from_utf8_lossy(&bytes), env);
        if env && dir {
            hits.insert(0, Hit { line: 0, kind: ".env file kept in the project", shown: "(whole file)".into() });
        }
        if !hits.is_empty() {
            with += 1;
        }
        let shown = if dir { rel(src, p) } else { name };
        found.extend(hits.into_iter().map(|h| (shown.clone(), h)));
    }
    if !dir && read == 0 {
        bail!("not a text file");
    }
    let more = if stopped { " (stopped early: too much to read)" } else { "" };
    if found.is_empty() {
        note(&format!("no secrets found in {} files{more}", count(read)));
        return Ok(vec![]);
    }
    let mut md = format!(
        "# Possible secrets in {}\n\n{} found in {} of {} files{more}. Values are masked here; open the file to see them.\n\nAnything that was ever pushed, shared or backed up should be replaced, not just deleted.\n\n",
        code(&name_of(src)),
        count(found.len() as u64),
        count(with),
        count(read)
    );
    for (file, h) in &found {
        let at = if h.line == 0 { file.clone() } else { format!("{file}:{}", h.line) };
        md.push_str(&format!("- {} — {} — {}\n", code(&at), h.kind, code(&h.shown)));
    }
    let out = beside(src, " Secrets", "md")?;
    std::fs::write(&out, md)?;
    note(&format!("{} possible secrets in {} files", count(found.len() as u64), count(with)));
    Ok(vec![out])
}

// ---------------------------------------------------------------- a folder, packed to hand over

const PACK_DIRS: [&str; 7] = [".git", "node_modules", "target", ".build", "build", "dist", "DerivedData"];

/// Must say what the zip patterns in `pack` say, or the toast would lie about what was left out.
fn unpacked(name: &str, dir: bool) -> Option<&'static str> {
    if name == ".git" || name == "node_modules" || dir && PACK_DIRS.contains(&name) {
        PACK_DIRS.iter().find(|d| **d == name).copied()
    } else if name == ".DS_Store" {
        Some(".DS_Store")
    } else if name.starts_with(".env") {
        Some(".env files")
    } else if !dir && (name.ends_with(".pem") || name.ends_with(".key")) {
        Some("key files")
    } else {
        None
    }
}

fn pack(src: &Path) -> Result<Vec<PathBuf>> {
    if !src.is_dir() {
        bail!("drop a folder for this");
    }
    // zip is told to store links as links, and would store this one instead of the folder
    if std::fs::symlink_metadata(src)?.file_type().is_symlink() {
        bail!("this is a link to a folder: drop the folder itself");
    }
    let (parent, name) = (src.parent().context("no folder")?, name_of(src));
    let w = walk(src, &unpacked);
    // zip would still make an archive, of one empty folder
    if w.files.is_empty() && w.links == 0 && !w.capped {
        bail!("nothing left to pack in this folder");
    }
    let mut skip: Vec<String> = Vec::new();
    for d in PACK_DIRS {
        skip.push(format!("*/{d}/*"));
    }
    // a submodule's .git is a file, pnpm's node_modules a link
    skip.extend(["*/.git", "*/node_modules", "*/.DS_Store", "*/.env*", "*.pem", "*.key"].map(String::from));
    let tmp = temp_file("zip");
    let out = unique(parent, &name, "zip");
    // "./" so a folder called "-r" is a name, not an option
    let o = crate::engines::tools::output_within(Command::new("/usr/bin/zip").current_dir(parent).args(["-q", "-r", "-y", "-X"]).arg(&tmp).arg(format!("./{name}")).arg("-x").args(&skip), Duration::from_secs(3600))?;
    // 12: nothing to do. 18: some files couldn't be opened, the rest are in.
    match o.status.code() {
        Some(0 | 18) if tmp.is_file() => {}
        Some(12) => bail!("nothing left to pack in this folder"),
        _ => bail!("couldn't make the zip: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("zip failed").trim()),
    }
    let _ = mark_utf8(&tmp);
    // the name is already reserved; the finished archive takes its place
    if std::fs::rename(&tmp, &out).is_err() {
        std::fs::copy(&tmp, &out)?;
        let _ = std::fs::remove_file(&tmp);
    }
    let left: Vec<String> = w.left.iter().map(|(k, n)| if *n > 1 { format!("{k} ×{n}") } else { k.to_string() }).collect();
    let left = if left.is_empty() { "nothing left out".to_string() } else { format!("left out: {}", left.join(", ")) };
    let files = if w.capped { "a very large folder".to_string() } else { format!("{} files", count(w.files.len() as u64)) };
    let unread = if o.status.code() == Some(18) { "; some files couldn't be read" } else { "" };
    note(&format!("{}, {files}; {left}{unread}", size(std::fs::metadata(&out)?.len())));
    Ok(vec![out])
}

/// The system's zip writes names in UTF-8 without setting the flag that says so, and Windows
/// then shows "Tài liệu" as noise. Sets the flag, in the list and at each file. Leaves the
/// archive alone if anything about it is unexpected.
fn mark_utf8(zip: &Path) -> Option<()> {
    let mut f = std::fs::OpenOptions::new().read(true).write(true).open(zip).ok()?;
    let (items, _) = central(&mut f)?;
    let mut two = [0u8; 2];
    let mut places = Vec::new();
    for i in items.iter().filter(|i| !i.name.is_ascii()) {
        // a name that isn't UTF-8, or an archive past 4 GB: not ours to relabel
        if i.name.contains('\u{fffd}') || i.data == 0xFFFF_FFFF {
            return None;
        }
        let mut sig = [0u8; 4];
        f.seek(SeekFrom::Start(i.data)).ok()?;
        f.read_exact(&mut sig).ok()?;
        if &sig != b"PK\x03\x04" {
            return None;
        }
        places.extend([i.listed + 8, i.data + 6]);
    }
    for at in places {
        f.seek(SeekFrom::Start(at)).ok()?;
        f.read_exact(&mut two).ok()?;
        f.seek(SeekFrom::Start(at)).ok()?;
        std::io::Write::write_all(&mut f, &(u16::from_le_bytes(two) | 0x0800).to_le_bytes()).ok()?;
    }
    Some(())
}

// ---------------------------------------------------------------- a log, digested

/// Calls `on` with each line, holding at most `keep` bytes of one: a 200 MB file can be one line.
fn each_line(f: impl Read, keep: usize, mut on: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
    let mut r = std::io::BufReader::with_capacity(1 << 16, f);
    let mut line: Vec<u8> = Vec::new();
    let mut open = false;
    loop {
        let buf = r.fill_buf()?;
        if buf.is_empty() {
            if open {
                on(&line)?;
            }
            return Ok(());
        }
        let (take, end) = match buf.iter().position(|b| *b == b'\n') {
            Some(i) => (i, true),
            None => (buf.len(), false),
        };
        let room = keep.saturating_sub(line.len()).min(take);
        line.extend_from_slice(buf.get(..room).unwrap_or_default());
        r.consume(take + end as usize);
        open = !end;
        if end {
            on(&line)?;
            line.clear();
        }
    }
}

fn digits(s: &[u8], at: usize, n: usize) -> bool {
    s.get(at..at + n).is_some_and(|d| d.iter().all(u8::is_ascii_digit))
}

fn is(s: &[u8], at: usize, c: u8) -> bool {
    s.get(at) == Some(&c)
}

/// 12:34:56 with optional fraction and zone; the length matched.
fn clock(s: &[u8]) -> Option<usize> {
    if !(digits(s, 0, 2) && is(s, 2, b':') && digits(s, 3, 2) && is(s, 5, b':') && digits(s, 6, 2)) {
        return None;
    }
    let mut n = 8;
    if (is(s, n, b'.') || is(s, n, b',')) && digits(s, n + 1, 1) {
        n += 1;
        while digits(s, n, 1) {
            n += 1;
        }
    }
    if is(s, n, b'Z') {
        n += 1;
    } else if (is(s, n, b'+') || is(s, n, b'-')) && digits(s, n + 1, 2) {
        n += 3;
        if is(s, n, b':') && digits(s, n + 1, 2) {
            n += 3;
        } else if digits(s, n, 2) {
            n += 2;
        }
    }
    Some(n)
}

fn month(s: &[u8]) -> bool {
    s.get(..3).is_some_and(|m| [b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov", b"Dec"].iter().any(|k| *k == m))
}

/// 2026-11-13T04:36:00Z · 2026/11/13 04:36:00 · 13/Nov/2026:04:36:00 +0700 · Nov 13 04:36:00 · 04:36:00.123
fn timestamp(s: &[u8]) -> Option<usize> {
    let sep = s.get(4).copied().filter(|c| matches!(c, b'-' | b'/'))?;
    if digits(s, 0, 4) && digits(s, 5, 2) && is(s, 7, sep) && digits(s, 8, 2) && !digits(s, 10, 1) {
        let t = s.get(10).filter(|c| matches!(c, b'T' | b' ' | b'_')).and_then(|_| clock(s.get(11..)?)).map_or(0, |n| n + 1);
        return Some(10 + t);
    }
    None
}

fn stamp(s: &[u8]) -> Option<usize> {
    if let Some(n) = timestamp(s).or_else(|| clock(s)) {
        return Some(n);
    }
    if digits(s, 0, 2) && is(s, 2, b'/') && month(s.get(3..)?) && is(s, 6, b'/') && digits(s, 7, 4) && is(s, 11, b':') {
        let n = 12 + clock(s.get(12..)?)?;
        return Some(if is(s, n, b' ') && (is(s, n + 1, b'+') || is(s, n + 1, b'-')) && digits(s, n + 2, 4) { n + 6 } else { n });
    }
    if month(s) && is(s, 3, b' ') {
        let day = if is(s, 4, b' ') && digits(s, 5, 1) || digits(s, 4, 2) { 2 } else { return None };
        if is(s, 4 + day, b' ') {
            return clock(s.get(5 + day..)?).map(|n| 5 + day + n);
        }
    }
    None
}

fn uuid(s: &[u8]) -> bool {
    s.get(..36).is_some_and(|u| u.iter().enumerate().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { *c == b'-' } else { c.is_ascii_hexdigit() })) && !s.get(36).is_some_and(u8::is_ascii_alphanumeric)
}

fn ip(s: &[u8]) -> Option<usize> {
    let mut n = 0;
    for part in 0..4 {
        let d = s.get(n..)?.iter().take(4).take_while(|c| c.is_ascii_digit()).count();
        if !(1..=3).contains(&d) {
            return None;
        }
        n += d;
        if part < 3 {
            if !is(s, n, b'.') {
                return None;
            }
            n += 1;
        }
    }
    if is(s, n, b'.') && digits(s, n + 1, 1) {
        return None;
    }
    if is(s, n, b':') && digits(s, n + 1, 1) {
        n += 1;
        while digits(s, n, 1) {
            n += 1;
        }
    }
    Some(n)
}

/// A line with everything that changes from one occurrence to the next replaced by what it was,
/// and the first time written in it.
pub(crate) fn template(line: &str) -> (String, Option<&str>) {
    let b = line.as_bytes();
    let (mut out, mut when, mut i) = (String::with_capacity(line.len().min(256)), None, 0);
    let stop = |c: &u8| c.is_ascii_whitespace() || matches!(c, b'"' | b'\'' | b')' | b']' | b'}' | b'>' | b',' | b';');
    while i < b.len() {
        if out.len() >= 240 {
            out.push('…');
            break;
        }
        let (c, s) = (b.get(i).copied().unwrap_or(0), b.get(i..).unwrap_or_default());
        let prev = if i == 0 { b' ' } else { b.get(i - 1).copied().unwrap_or(b' ') };
        let fresh = !prev.is_ascii_alphanumeric();
        if fresh && c.is_ascii_alphanumeric() {
            if let Some(n) = stamp(s) {
                when = when.or(line.get(i..i + n));
                out.push_str("<time>");
                i += n;
                continue;
            }
            if uuid(s) {
                out.push_str("<uuid>");
                i += 36;
                continue;
            }
            if s.starts_with(b"http://") || s.starts_with(b"https://") {
                out.push_str("<url>");
                i += s.iter().take_while(|c| !stop(c)).count();
                continue;
            }
            if let Some(n) = ip(s) {
                out.push_str("<ip>");
                i += n;
                continue;
            }
            let run = s.iter().take_while(|c| c.is_ascii_alphanumeric()).count();
            let tok = s.get(..run).unwrap_or_default();
            let nums = tok.iter().filter(|c| c.is_ascii_digit()).count();
            if tok.len() > 2 && (tok.starts_with(b"0x") || tok.starts_with(b"0X")) && tok.iter().skip(2).all(u8::is_ascii_hexdigit) {
                out.push_str("<hex>");
            } else if nums == run {
                // 12.5 is one number; 1.2.3 was an address or a version and is three
                let frac = if is(s, run, b'.') { s.get(run + 1..).unwrap_or_default().iter().take_while(|c| c.is_ascii_digit()).count() } else { 0 };
                out.push_str("<n>");
                i += run + if frac > 0 { frac + 1 } else { 0 };
                continue;
            } else if run >= 6 && nums > 0 && tok.iter().all(u8::is_ascii_hexdigit) {
                out.push_str("<hex>");
            } else if run >= 8 && nums >= 2 {
                out.push_str("<id>");
            } else if c.is_ascii_digit() {
                // 12ms, 3GB: the unit stays
                let d = tok.iter().take_while(|c| c.is_ascii_digit()).count();
                out.push_str("<n>");
                out.push_str(line.get(i + d..i + run).unwrap_or_default());
            } else {
                out.push_str(line.get(i..i + run).unwrap_or_default());
            }
            i += run;
            continue;
        }
        let rooted = c == b'/' && s.get(1).is_some_and(|n| n.is_ascii_alphanumeric() || matches!(n, b'.' | b'_' | b'~'));
        let homed = matches!(c, b'~' | b'.') && is(s, 1, b'/');
        if fresh && prev != b'/' && prev != b'<' && (rooted || homed) {
            let n = s.iter().take_while(|c| !stop(c) && **c != b':').count();
            // one piece is a route or a flag worth reading; two or more is somewhere on a disk
            if s.get(1..n).unwrap_or_default().contains(&b'/') {
                out.push_str("<path>");
                i += n;
                continue;
            }
        }
        if c == b'"' || (c == b'\'' && fresh) {
            let close = s.iter().enumerate().skip(1).take(400).find(|(k, x)| **x == c && s.get(k - 1) != Some(&b'\\')).map(|(k, _)| k);
            if let Some(k) = close.filter(|k| c == b'"' || !s.get(k + 1).is_some_and(u8::is_ascii_alphanumeric)) {
                let inner = s.get(1..k).unwrap_or_default();
                // "level", "error": words like these are the shape of the line, not its cargo
                if !inner.is_empty() && inner.len() <= 24 && inner.iter().all(|x| x.is_ascii_alphabetic() || matches!(x, b'_' | b'-' | b'.')) {
                    out.push_str(line.get(i..i + k + 1).unwrap_or_default());
                } else {
                    out.push_str("<str>");
                }
                i += k + 1;
                continue;
            }
        }
        let n = line.get(i..).and_then(|r| r.chars().next()).map_or(1, char::len_utf8);
        out.push_str(line.get(i..i + n).unwrap_or(" "));
        i += n;
    }
    // one unbroken run can be as long as the line
    if out.len() > 260 {
        let cut = (0..=260).rev().find(|k| out.is_char_boundary(*k)).unwrap_or(0);
        out.truncate(cut);
        out.push('…');
    }
    (out, when)
}

/// 2 for errors, 1 for warnings. A level the line states itself wins over words in its message,
/// so "INFO no error found" stays quiet.
pub(crate) fn severity(t: &str) -> u8 {
    let words = || t.split(|c: char| !c.is_ascii_alphabetic()).filter(|w| !w.is_empty());
    for w in words() {
        match w {
            "TRACE" | "DEBUG" | "INFO" | "NOTICE" => return 0,
            "WARN" | "WARNING" => return 1,
            "ERROR" | "ERR" | "FATAL" | "CRITICAL" | "CRIT" | "PANIC" | "SEVERE" | "EMERG" => return 2,
            _ => {}
        }
    }
    let mut level = 0;
    for w in words() {
        // ValueError and NullPointerException are errors; terror is not
        let named = w.ends_with("Error") || w.ends_with("Exception");
        let w = w.to_ascii_lowercase();
        if named || matches!(w.as_str(), "error" | "err" | "fatal" | "panic" | "panicked" | "critical" | "traceback" | "exception") {
            return 2;
        }
        if matches!(w.as_str(), "warn" | "warning") {
            level = 1;
        }
    }
    level
}

struct Kind {
    n: u64,
    level: u8,
    first: String,
    last: String,
    example: String,
}

fn log(src: &Path) -> Result<Vec<PathBuf>> {
    let f = std::fs::File::open(src)?;
    if f.metadata()?.len() > 4 << 30 {
        bail!("this file is too big for this (over 4096 MB)");
    }
    let mut kinds: HashMap<String, Kind> = HashMap::new();
    let (mut total, mut blank, mut span) = (0u64, 0u64, (String::new(), String::new()));
    each_line(f, 4096, |raw| {
        total += 1;
        if total <= 64 && raw.contains(&0) {
            bail!("not a text file");
        }
        let text = String::from_utf8_lossy(raw);
        let line = text.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            return Ok(());
        }
        let (mut key, when) = template(line);
        let at = when.map(str::to_string).unwrap_or_else(|| format!("line {total}"));
        if let Some(w) = when {
            if span.0.is_empty() {
                span.0 = w.to_string();
            }
            span.1.clear();
            span.1.push_str(w);
        }
        // a log where every line is its own kind must not be held in memory whole
        if kinds.len() >= 50_000 && !kinds.contains_key(&key) {
            key = "(many more kinds, each seen rarely)".into();
        }
        match kinds.get_mut(&key) {
            Some(k) => {
                k.n += 1;
                k.last = at;
            }
            None => {
                let level = severity(&key);
                kinds.insert(key, Kind { n: 1, level, first: at.clone(), last: at, example: line.chars().take(300).collect() });
            }
        }
        Ok(())
    })?;
    if kinds.is_empty() {
        bail!("this log is empty");
    }
    let mut all: Vec<(&String, &Kind)> = kinds.iter().collect();
    all.sort_by(|a, b| b.1.n.cmp(&a.1.n).then(a.0.cmp(b.0)));
    let errors: u64 = all.iter().filter(|k| k.1.level == 2).map(|k| k.1.n).sum();
    let period = if span.0.is_empty() { String::new() } else { format!(" · {} → {}", span.0, span.1) };
    let mut md = format!("# {}\n\n{} lines · {} kinds · {} error lines{period}\n", code(&name_of(src)), count(total - blank), count(all.len() as u64), count(errors));
    for (title, level, most) in [("Errors", 2u8, 100usize), ("Warnings", 1, 50), ("Most common others", 0, 30)] {
        let of: Vec<&(&String, &Kind)> = all.iter().filter(|k| k.1.level == level).collect();
        if of.is_empty() {
            continue;
        }
        md.push_str(&format!("\n## {title} ({} kinds, {} lines)\n\n", count(of.len() as u64), count(of.iter().map(|k| k.1.n).sum())));
        for (key, k) in of.iter().take(most) {
            let seen = if k.n == 1 { k.first.clone() } else { format!("first {}, last {}", k.first, k.last) };
            md.push_str(&format!("- **{}×** {}\n  {}\n  {}\n", count(k.n), code(key), code(&seen).trim_matches('`'), code(&k.example)));
        }
        if of.len() > most {
            md.push_str(&format!("- and {} more kinds\n", count((of.len() - most) as u64)));
        }
    }
    let out = out_path(src, Some("Digest"), "md");
    std::fs::write(&out, md)?;
    note(&format!("{} lines, {} kinds, {} errors", count(total - blank), count(all.len() as u64), count(errors)));
    Ok(vec![out])
}

// ---------------------------------------------------------------- types from JSON

#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Ty {
    Any,
    Bool,
    Int,
    Float,
    Str,
    List(Box<Ty>),
    Obj(usize),
    Opt(Box<Ty>),
}

pub(crate) struct Field {
    key: String,
    ty: Ty,
    /// absent from some of the objects seen, as opposed to present and null
    missing: bool,
}

/// One struct per object shape found in a JSON file; the emitters below write them out in each
/// language. A sibling of `desk::Types`, which does the same for TypeScript.
#[derive(Default)]
pub(crate) struct Shapes {
    all: Vec<(String, Vec<Field>)>,
}

/// Letters every one of Swift, Rust and Go takes in a name: ASCII, and the Latin, Greek and
/// Cyrillic blocks (so Vietnamese keys keep their accents).
fn letter(c: char) -> bool {
    c.is_ascii_alphanumeric() || (c.is_alphabetic() && matches!(c as u32, 0xC0..=0x24F | 0x370..=0x3FF | 0x400..=0x4FF | 0x1E00..=0x1EFF))
}

/// user_id, userID, "User Id" and HTTPServer as lowercase words.
pub(crate) fn words(s: &str) -> Vec<String> {
    let cs: Vec<char> = s.chars().take(200).collect();
    let (mut out, mut cur) = (Vec::new(), String::new());
    for (i, &c) in cs.iter().enumerate() {
        let p = cur.chars().last();
        let hump = p.is_some_and(|p| c.is_uppercase() && (p.is_lowercase() || p.is_ascii_digit() || (p.is_uppercase() && cs.get(i + 1).is_some_and(|n| n.is_lowercase()))));
        if (!letter(c) || hump) && !cur.is_empty() {
            out.push(std::mem::take(&mut cur).to_lowercase());
        }
        if letter(c) {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur.to_lowercase());
    }
    out
}

fn cap(w: &str) -> String {
    let mut c = w.chars();
    // an uppercase form can be two characters, and not all of them belong in a name
    c.next().map(|f| f.to_uppercase().filter(|u| letter(*u)).chain(c).collect()).unwrap_or_default()
}

fn pascal(w: &[String]) -> String {
    w.iter().map(|p| cap(p)).collect()
}

fn camel(w: &[String]) -> String {
    w.iter().enumerate().map(|(i, p)| if i == 0 { p.clone() } else { cap(p) }).collect()
}

const TYPE_NAMES: [&str; 34] = [
    "String", "Int", "Double", "Float", "Bool", "Any", "Type", "Self", "Data", "Date", "Array", "Dictionary", "Optional", "Set", "Error", "Result", "Codable", "Decoder", "Encoder", "CodingKeys", "JSONValue", "Vec", "Option", "Box", "Value",
    "Some", "None", "Ok", "Err", "Protocol", "Character", "Never", "Void", "Object",
];
const SWIFT_WORDS: [&str; 62] = [
    "associatedtype", "class", "deinit", "enum", "extension", "fileprivate", "func", "import", "init", "inout", "internal", "let", "open", "operator", "private", "protocol", "public", "rethrows", "static", "struct", "subscript",
    "typealias", "var", "break", "case", "continue", "default", "defer", "do", "else", "fallthrough", "for", "guard", "if", "in", "repeat", "return", "switch", "where", "while", "as", "catch", "false", "is", "nil", "super", "throw", "throws",
    "true", "try", "any", "await", "async", "some", "actor", "macro", "consume", "copy", "borrowing", "consuming", "nonisolated", "isolated",
];
const RUST_WORDS: [&str; 50] = [
    "as", "break", "const", "continue", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen", "union", "safe",
];

impl Shapes {
    fn name(&self, hint: &str) -> String {
        let mut n = pascal(&words(hint));
        if n.is_empty() || n.starts_with(|c: char| c.is_ascii_digit()) {
            n = format!("Item{n}");
        }
        if TYPE_NAMES.contains(&n.as_str()) {
            n.push_str("Item");
        }
        let (base, mut i) = (n.clone(), 2);
        while self.all.iter().any(|s| s.0 == n) {
            n = format!("{base}{i}");
            i += 1;
        }
        n
    }

    fn object(&mut self, hint: &str, shapes: &[&Map<String, Value>]) -> Result<usize> {
        let at = self.all.len();
        // a map from IDs to records is data, not a shape: it would be a struct per record
        if at >= 500 {
            bail!("this JSON has too many different shapes to write types for");
        }
        self.all.push((self.name(hint), Vec::new())); // reserve the name before going deeper
        let mut keys: Vec<&String> = Vec::new();
        let mut known = std::collections::HashSet::new();
        for k in shapes.iter().flat_map(|s| s.keys()) {
            if known.insert(k) {
                keys.push(k);
            }
        }
        if keys.len() > 2000 {
            bail!("this JSON has too many different shapes to write types for");
        }
        let mut fields = Vec::new();
        for k in keys {
            let vals: Vec<&Value> = shapes.iter().filter_map(|s| s.get(k)).collect();
            fields.push(Field { key: k.clone(), ty: self.of(k, &vals)?, missing: vals.len() < shapes.len() });
        }
        if let Some(slot) = self.all.get_mut(at) {
            slot.1 = fields;
        }
        Ok(at)
    }

    fn of(&mut self, hint: &str, vals: &[&Value]) -> Result<Ty> {
        let some: Vec<&Value> = vals.iter().copied().filter(|v| !v.is_null()).collect();
        let ty = if some.is_empty() {
            Ty::Any
        } else if some.iter().all(|v| v.is_boolean()) {
            Ty::Bool
        } else if some.iter().all(|v| v.is_number()) {
            // one 1.5 among the 1s and 2s makes them all fractions
            if some.iter().all(|v| v.is_i64()) { Ty::Int } else { Ty::Float }
        } else if some.iter().all(|v| v.is_string()) {
            Ty::Str
        } else if some.iter().all(|v| v.is_object()) {
            Ty::Obj(self.object(hint, &some.iter().filter_map(|v| v.as_object()).collect::<Vec<_>>())?)
        } else if some.iter().all(|v| v.is_array()) {
            let items: Vec<&Value> = some.iter().filter_map(|v| v.as_array()).flatten().collect();
            let one = hint.strip_suffix("ies").map(|h| format!("{h}y")).or_else(|| hint.strip_suffix('s').filter(|h| !h.ends_with('s') && !h.is_empty()).map(str::to_string)).unwrap_or_else(|| hint.to_string());
            Ty::List(Box::new(self.of(&one, &items)?))
        } else {
            Ty::Any
        };
        Ok(if some.len() < vals.len() && ty != Ty::Any { Ty::Opt(Box::new(ty)) } else { ty })
    }
}

/// Field names for one struct in one language's spelling, none of them twice.
fn idents(fields: &[Field], spell: impl Fn(&[String]) -> String) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for f in fields {
        let n = spell(&words(&f.key));
        let (base, mut i) = (n.clone(), 2);
        let mut n = n;
        while out.contains(&n) {
            n = format!("{base}{i}");
            i += 1;
        }
        out.push(n);
    }
    out
}

fn swift_name(w: &[String]) -> String {
    let n = camel(w);
    match n.as_str() {
        "" => "field".into(),
        // these can't be told from the language's own even in backticks
        "self" | "init" | "type" | "protocol" => format!("{n}_"),
        _ if n.starts_with(|c: char| c.is_ascii_digit()) => format!("_{n}"),
        _ => n,
    }
}

fn swift_type(t: &Ty, s: &Shapes) -> String {
    match t {
        Ty::Any => "JSONValue".into(),
        Ty::Bool => "Bool".into(),
        Ty::Int => "Int".into(),
        Ty::Float => "Double".into(),
        Ty::Str => "String".into(),
        Ty::List(i) => format!("[{}]", swift_type(i, s)),
        Ty::Obj(i) => s.all.get(*i).map(|o| o.0.clone()).unwrap_or_default(),
        Ty::Opt(i) => format!("{}?", swift_type(i, s)),
    }
}

/// A Swift string literal. JSON keys can hold quotes, backslashes and line breaks.
fn swift_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') => o.push_str(&format!("\\u{{{:X}}}", c as u32)),
            c => o.push(c),
        }
    }
    o + "\""
}

const SWIFT_ANY: &str = "/// A value whose type differed from one place to the next in the sample.\nenum JSONValue: Codable {\n    case null\n    case bool(Bool)\n    case number(Double)\n    case string(String)\n    case array([JSONValue])\n    case object([String: JSONValue])\n\n    init(from decoder: Decoder) throws {\n        let c = try decoder.singleValueContainer()\n        if c.decodeNil() {\n            self = .null\n        } else if let v = try? c.decode(Bool.self) {\n            self = .bool(v)\n        } else if let v = try? c.decode(Double.self) {\n            self = .number(v)\n        } else if let v = try? c.decode(String.self) {\n            self = .string(v)\n        } else if let v = try? c.decode([JSONValue].self) {\n            self = .array(v)\n        } else {\n            self = .object(try c.decode([String: JSONValue].self))\n        }\n    }\n\n    func encode(to encoder: Encoder) throws {\n        var c = encoder.singleValueContainer()\n        switch self {\n        case .null: try c.encodeNil()\n        case .bool(let v): try c.encode(v)\n        case .number(let v): try c.encode(v)\n        case .string(let v): try c.encode(v)\n        case .array(let v): try c.encode(v)\n        case .object(let v): try c.encode(v)\n        }\n    }\n}\n";

pub(crate) fn swift(s: &Shapes, top: &Ty, alias: &str) -> String {
    let mut o = String::from("import Foundation\n\n");
    if !matches!(top, Ty::Obj(_)) {
        o.push_str(&format!("typealias {alias} = {}\n\n", swift_type(top, s)));
    }
    for (name, fields) in &s.all {
        let names = idents(fields, swift_name);
        let tick = |n: &str| if SWIFT_WORDS.contains(&n) { format!("`{n}`") } else { n.to_string() };
        o.push_str(&format!("struct {name}: Codable {{\n"));
        for (f, n) in fields.iter().zip(&names) {
            let t = swift_type(&f.ty, s);
            o.push_str(&format!("    var {}: {t}{}\n", tick(n), if f.missing && !t.ends_with('?') { "?" } else { "" }));
        }
        if fields.iter().zip(&names).any(|(f, n)| f.key != *n) {
            o.push_str("\n    enum CodingKeys: String, CodingKey {\n");
            for (f, n) in fields.iter().zip(&names) {
                o.push_str(&if f.key == *n { format!("        case {}\n", tick(n)) } else { format!("        case {} = {}\n", tick(n), swift_str(&f.key)) });
            }
            o.push_str("    }\n");
        }
        o.push_str("}\n\n");
    }
    if o.contains("JSONValue") {
        o.push_str(SWIFT_ANY);
    }
    o.trim_end().to_string() + "\n"
}

fn rust_name(w: &[String]) -> String {
    let n = w.join("_");
    match n.as_str() {
        "" => "field".into(),
        // the four that can't be written r#
        "self" | "super" | "crate" => format!("{n}_"),
        _ if n.starts_with(|c: char| c.is_ascii_digit()) => format!("_{n}"),
        _ => n,
    }
}

fn rust_type(t: &Ty, s: &Shapes) -> String {
    match t {
        Ty::Any => "serde_json::Value".into(),
        Ty::Bool => "bool".into(),
        Ty::Int => "i64".into(),
        Ty::Float => "f64".into(),
        Ty::Str => "String".into(),
        Ty::List(i) => format!("Vec<{}>", rust_type(i, s)),
        Ty::Obj(i) => s.all.get(*i).map(|o| o.0.clone()).unwrap_or_default(),
        Ty::Opt(i) => format!("Option<{}>", rust_type(i, s)),
    }
}

pub(crate) fn rust(s: &Shapes, top: &Ty, alias: &str) -> String {
    // a file that is one number has no struct to derive anything for
    let mut o = String::from(if s.all.is_empty() { "" } else { "use serde::{Deserialize, Serialize};\n\n" });
    if !matches!(top, Ty::Obj(_)) {
        o.push_str(&format!("pub type {alias} = {};\n\n", rust_type(top, s)));
    }
    for (name, fields) in &s.all {
        o.push_str(&format!("#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct {name} {{\n"));
        for (f, n) in fields.iter().zip(idents(fields, rust_name)) {
            let t = rust_type(&f.ty, s);
            let mut how = Vec::new();
            if f.key != n {
                how.push(format!("rename = {:?}", f.key));
            }
            // a key that was absent goes back out absent, not as null
            if f.missing {
                how.push("default".into());
                how.push(format!("skip_serializing_if = \"{}\"", if t.starts_with("Option<") { "Option::is_none" } else { "is_absent" }));
            }
            if !how.is_empty() {
                o.push_str(&format!("    #[serde({})]\n", how.join(", ")));
            }
            let t = if f.missing && !t.starts_with("Option<") { format!("Option<{t}>") } else { t };
            o.push_str(&format!("    pub {}{n}: {t},\n", if RUST_WORDS.contains(&n.as_str()) { "r#" } else { "" }));
        }
        o.push_str("}\n\n");
    }
    o.replace("\"is_absent\"", "\"Option::is_none\"").trim_end().to_string() + "\n"
}

fn go_name(w: &[String]) -> String {
    let n = pascal(w);
    // only a name starting with a capital is visible to encoding/json
    if n.is_empty() {
        "Field".into()
    } else if n.starts_with(char::is_uppercase) {
        n
    } else {
        format!("X{n}")
    }
}

fn go_type(t: &Ty, s: &Shapes) -> String {
    match t {
        Ty::Any => "any".into(),
        Ty::Bool => "bool".into(),
        Ty::Int => "int64".into(),
        Ty::Float => "float64".into(),
        Ty::Str => "string".into(),
        Ty::List(i) => format!("[]{}", go_type(i, s)),
        Ty::Obj(i) => s.all.get(*i).map(|o| o.0.clone()).unwrap_or_default(),
        // a nil slice and a nil interface already say "nothing"
        Ty::Opt(i) => if matches!(**i, Ty::List(_) | Ty::Any) { go_type(i, s) } else { format!("*{}", go_type(i, s)) },
    }
}

pub(crate) fn go(s: &Shapes, top: &Ty, alias: &str) -> String {
    let mut o = String::from("package model\n\n");
    if !matches!(top, Ty::Obj(_)) {
        o.push_str(&format!("type {alias} {}\n\n", go_type(top, s)));
    }
    for (name, fields) in &s.all {
        o.push_str(&format!("type {name} struct {{\n"));
        for (f, n) in fields.iter().zip(idents(fields, go_name)) {
            let t = go_type(&f.ty, s);
            let t = if f.missing && !t.starts_with(['*', '[']) && t != "any" { format!("*{t}") } else { t };
            // a struct tag is a raw string: these can't be written inside one at all
            if f.key.is_empty() || f.key.chars().any(|c| c.is_control() || matches!(c, '`' | '"' | '\\' | ',')) {
                o.push_str(&format!("\t{n} {t} `json:\"-\"` // this key can't be written as a Go tag\n"));
            } else {
                o.push_str(&format!("\t{n} {t} `json:\"{}{}\"`\n", f.key, if f.missing { ",omitempty" } else { "" }));
            }
        }
        o.push_str("}\n\n");
    }
    o.trim_end().to_string() + "\n"
}

fn schema_of(t: &Ty, s: &Shapes) -> Value {
    use serde_json::json;
    match t {
        Ty::Any => json!({}),
        Ty::Bool => json!({"type": "boolean"}),
        Ty::Int => json!({"type": "integer"}),
        Ty::Float => json!({"type": "number"}),
        Ty::Str => json!({"type": "string"}),
        Ty::List(i) => json!({"type": "array", "items": schema_of(i, s)}),
        Ty::Obj(i) => {
            let fields = s.all.get(*i).map(|o| o.1.as_slice()).unwrap_or_default();
            let props: Map<String, Value> = fields.iter().map(|f| (f.key.clone(), schema_of(&f.ty, s))).collect();
            let required: Vec<&String> = fields.iter().filter(|f| !f.missing).map(|f| &f.key).collect();
            json!({"type": "object", "properties": props, "required": required})
        }
        Ty::Opt(i) => match schema_of(i, s) {
            Value::Object(mut m) if m.get("type").is_some_and(Value::is_string) => {
                let ty = m.remove("type").unwrap_or_default();
                let mut out: Map<String, Value> = Map::new();
                out.insert("type".into(), json!([ty, "null"]));
                out.extend(m);
                Value::Object(out)
            }
            other => json!({"anyOf": [other, {"type": "null"}]}),
        },
    }
}

fn structs(src: &Path, lang: &str) -> Result<Vec<PathBuf>> {
    let v = json(src)?;
    let mut s = Shapes::default();
    let root = s.name(&stem(src));
    let top = s.of(&root, &[&v])?;
    let alias = s.name(&format!("{root} file"));
    let (text, suffix, e) = match lang {
        "swift" => (swift(&s, &top, &alias), "types", "swift"),
        "rust" => (rust(&s, &top, &alias), "types", "rs"),
        "go" => (go(&s, &top, &alias), "types", "go"),
        _ => {
            let mut doc: Map<String, Value> = Map::new();
            doc.insert("$schema".into(), "https://json-schema.org/draft/2020-12/schema".into());
            doc.insert("title".into(), root.clone().into());
            if let Value::Object(m) = schema_of(&top, &s) {
                doc.extend(m);
            }
            (serde_json::to_string_pretty(&doc)? + "\n", "schema", "json")
        }
    };
    let out = out_path(src, Some(suffix), e);
    std::fs::write(&out, &text)?;
    let copied = if clip(&text)? { ", copied" } else { "" };
    note(&if lang == "schema" { format!("JSON Schema written{copied}") } else { format!("{} structs{copied}", s.all.len()) });
    Ok(vec![out])
}

// ---------------------------------------------------------------- SVG to a SwiftUI shape

/// x' = a·x + c·y + e, y' = b·x + d·y + f, as SVG writes matrix(a b c d e f).
type Mat = [f64; 6];
const SAME: Mat = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m` after `n`: what a child's transform is inside its parent's.
fn mul(m: Mat, n: Mat) -> Mat {
    [m[0] * n[0] + m[2] * n[1], m[1] * n[0] + m[3] * n[1], m[0] * n[2] + m[2] * n[3], m[1] * n[2] + m[3] * n[3], m[0] * n[4] + m[2] * n[5] + m[4], m[1] * n[4] + m[3] * n[5] + m[5]]
}

struct Nums<'a> {
    b: &'a [u8],
    i: usize,
}

impl Nums<'_> {
    fn skip(&mut self) {
        while self.b.get(self.i).is_some_and(|c| c.is_ascii_whitespace() || *c == b',') {
            self.i += 1;
        }
    }
    fn letter(&mut self) -> Option<u8> {
        self.skip();
        let c = self.b.get(self.i).copied().filter(u8::is_ascii_alphabetic)?;
        self.i += 1;
        Some(c)
    }
    fn more(&mut self) -> bool {
        self.skip();
        self.b.get(self.i).is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.'))
    }
    /// SVG packs numbers tight: "1.5.5" is two of them and "0-1" is too.
    fn num(&mut self) -> Option<f64> {
        self.skip();
        let b = self.b;
        let at = |i: usize| b.get(i).copied().unwrap_or(0);
        let mut i = self.i;
        if matches!(at(i), b'-' | b'+') {
            i += 1;
        }
        let from = i;
        while at(i).is_ascii_digit() {
            i += 1;
        }
        let mut seen = i - from;
        if at(i) == b'.' {
            i += 1;
            let from = i;
            while at(i).is_ascii_digit() {
                i += 1;
            }
            seen += i - from;
        }
        if seen == 0 {
            return None;
        }
        if matches!(at(i), b'e' | b'E') {
            let mut j = i + 1;
            if matches!(at(j), b'-' | b'+') {
                j += 1;
            }
            if at(j).is_ascii_digit() {
                while at(j).is_ascii_digit() {
                    j += 1;
                }
                i = j;
            }
        }
        let v: f64 = std::str::from_utf8(b.get(self.i..i)?).ok()?.parse().ok()?;
        self.i = i;
        (v.is_finite() && v.abs() < 1e9).then_some(v)
    }
    /// Arc flags may sit against each other and the next number: "011.5" is 0, 1, 1.5.
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let f = match self.b.get(self.i)? {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.i += 1;
        Some(f)
    }
    fn take<const N: usize>(&mut self) -> Option<[f64; N]> {
        let mut v = [0.0; N];
        for x in &mut v {
            *x = self.num()?;
        }
        Some(v)
    }
}

fn numbers(s: &str, most: usize) -> Vec<f64> {
    let mut n = Nums { b: s.as_bytes(), i: 0 };
    let mut v = Vec::new();
    while v.len() < most {
        match n.num() {
            Some(x) => v.push(x),
            None => break,
        }
    }
    v
}

/// The matrix for a `transform` attribute, and whether all of it was understood.
pub(crate) fn transform(s: &str) -> (Mat, bool) {
    let (mut m, mut rest, mut ok) = (SAME, s, true);
    let turn = |deg: f64| {
        let (sin, cos) = deg.to_radians().sin_cos();
        [cos, sin, -sin, cos, 0.0, 0.0]
    };
    while let Some((name, tail)) = rest.split_once('(') {
        let Some((args, next)) = tail.split_once(')') else { return (m, false) };
        let t = match (name.trim_matches(|c: char| c.is_whitespace() || c == ','), numbers(args, 7).as_slice()) {
            ("translate", [x]) => [1.0, 0.0, 0.0, 1.0, *x, 0.0],
            ("translate", [x, y]) => [1.0, 0.0, 0.0, 1.0, *x, *y],
            ("scale", [k]) => [*k, 0.0, 0.0, *k, 0.0, 0.0],
            ("scale", [x, y]) => [*x, 0.0, 0.0, *y, 0.0, 0.0],
            ("matrix", [a, b, c, d, e, f]) => [*a, *b, *c, *d, *e, *f],
            ("rotate", [a]) => turn(*a),
            ("rotate", [a, x, y]) => mul(mul([1.0, 0.0, 0.0, 1.0, *x, *y], turn(*a)), [1.0, 0.0, 0.0, 1.0, -x, -y]),
            ("skewX", [a]) => [1.0, 0.0, a.to_radians().tan(), 1.0, 0.0, 0.0],
            ("skewY", [a]) => [1.0, a.to_radians().tan(), 0.0, 1.0, 0.0, 0.0],
            _ => {
                ok = false;
                SAME
            }
        };
        m = mul(m, t);
        rest = next;
    }
    (m, ok && rest.trim().is_empty())
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Seg {
    Move([f64; 2]),
    Line([f64; 2]),
    Cubic([f64; 6]),
    Quad([f64; 4]),
    Close,
}

/// Draws in the element's own coordinates and records in the drawing's: every point goes
/// through the transforms in force, so the Swift side needs none.
pub(crate) struct Pen {
    m: Mat,
    segs: Vec<Seg>,
    cur: (f64, f64),
    start: (f64, f64),
    down: bool,
}

impl Pen {
    fn at(&self, x: f64, y: f64) -> [f64; 2] {
        [self.m[0] * x + self.m[2] * y + self.m[4], self.m[1] * x + self.m[3] * y + self.m[5]]
    }
    fn go(&mut self, x: f64, y: f64) {
        self.segs.push(Seg::Move(self.at(x, y)));
        (self.cur, self.start, self.down) = ((x, y), (x, y), true);
    }
    /// A path that draws before it moves starts where it is, as CoreGraphics would refuse to.
    fn ready(&mut self) {
        if !self.down {
            self.go(self.cur.0, self.cur.1);
        }
    }
    fn line(&mut self, x: f64, y: f64) {
        self.ready();
        self.segs.push(Seg::Line(self.at(x, y)));
        self.cur = (x, y);
    }
    fn cubic(&mut self, v: [f64; 6]) {
        self.ready();
        let (a, b, c) = (self.at(v[0], v[1]), self.at(v[2], v[3]), self.at(v[4], v[5]));
        self.segs.push(Seg::Cubic([a[0], a[1], b[0], b[1], c[0], c[1]]));
        self.cur = (v[4], v[5]);
    }
    fn quad(&mut self, v: [f64; 4]) {
        self.ready();
        let (a, b) = (self.at(v[0], v[1]), self.at(v[2], v[3]));
        self.segs.push(Seg::Quad([a[0], a[1], b[0], b[1]]));
        self.cur = (v[2], v[3]);
    }
    fn close(&mut self) {
        if self.down {
            self.segs.push(Seg::Close);
            (self.cur, self.down) = (self.start, false);
        }
    }

    /// An elliptical arc as cubic curves, a quarter turn at most each (SVG 1.1, appendix F.6).
    fn arc(&mut self, rx: f64, ry: f64, turn: f64, (large, sweep): (bool, bool), x: f64, y: f64) {
        use std::f64::consts::{FRAC_PI_2, TAU};
        let (x0, y0) = self.cur;
        let (mut rx, mut ry) = (rx.abs(), ry.abs());
        if (x0 - x).abs() < 1e-12 && (y0 - y).abs() < 1e-12 {
            return;
        }
        if rx < 1e-9 || ry < 1e-9 {
            return self.line(x, y);
        }
        let (sin, cos) = turn.to_radians().sin_cos();
        let (dx, dy) = ((x0 - x) / 2.0, (y0 - y) / 2.0);
        let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
        // radii too small to reach are grown until they do
        let reach = (x1 / rx).powi(2) + (y1 / ry).powi(2);
        if reach > 1.0 {
            rx *= reach.sqrt();
            ry *= reach.sqrt();
        }
        let below = (rx * y1).powi(2) + (ry * x1).powi(2);
        let k = if below == 0.0 { 0.0 } else { ((((rx * ry).powi(2) - below) / below).max(0.0)).sqrt() * if large == sweep { -1.0 } else { 1.0 } };
        let (cxp, cyp) = (k * rx * y1 / ry, -k * ry * x1 / rx);
        let (cx, cy) = (cos * cxp - sin * cyp + (x0 + x) / 2.0, sin * cxp + cos * cyp + (y0 + y) / 2.0);
        let from = ((y1 - cyp) / ry).atan2((x1 - cxp) / rx);
        let mut span = ((-y1 - cyp) / ry).atan2((-x1 - cxp) / rx) - from;
        if sweep && span < 0.0 {
            span += TAU;
        } else if !sweep && span > 0.0 {
            span -= TAU;
        }
        let n = (span.abs() / FRAC_PI_2 - 1e-9).ceil().clamp(1.0, 4.0) as usize;
        let step = span / n as f64;
        let t = 4.0 / 3.0 * (step / 4.0).tan();
        let on = |a: f64| (cx + cos * rx * a.cos() - sin * ry * a.sin(), cy + sin * rx * a.cos() + cos * ry * a.sin());
        let along = |a: f64| (-cos * rx * a.sin() - sin * ry * a.cos(), -sin * rx * a.sin() + cos * ry * a.cos());
        for i in 0..n {
            let (a, b) = (from + step * i as f64, from + step * (i + 1) as f64);
            let (p, pd, q, qd) = (on(a), along(a), on(b), along(b));
            // the last piece lands exactly where the path said, not where the arithmetic drifted
            let end = if i + 1 == n { (x, y) } else { q };
            self.cubic([p.0 + t * pd.0, p.1 + t * pd.1, q.0 - t * qd.0, q.1 - t * qd.1, end.0, end.1]);
        }
    }

    /// A `d` attribute. Stops at the first thing it can't read and keeps what came before, as
    /// browsers do.
    fn path(&mut self, d: &str) {
        let mut n = Nums { b: d.as_bytes(), i: 0 };
        let (mut cmd, mut prev, mut ctrl) = (0u8, 0u8, (0.0, 0.0));
        let _ = (|| -> Option<()> {
            loop {
                match n.letter() {
                    Some(c) => cmd = c,
                    None if cmd == 0 || !n.more() => return None,
                    None => {}
                }
                let (cx, cy) = self.cur;
                let (ox, oy) = if cmd.is_ascii_lowercase() { (cx, cy) } else { (0.0, 0.0) };
                let up = cmd.to_ascii_uppercase();
                // S and T continue the curve before them by mirroring its last control point
                let mirror = |after: &[u8]| if after.contains(&prev) { (2.0 * cx - ctrl.0, 2.0 * cy - ctrl.1) } else { (cx, cy) };
                match up {
                    b'M' => {
                        let [x, y] = n.take()?;
                        self.go(x + ox, y + oy);
                        // more pairs after a move are lines
                        cmd = if cmd == b'm' { b'l' } else { b'L' };
                    }
                    b'L' => {
                        let [x, y] = n.take()?;
                        self.line(x + ox, y + oy);
                    }
                    b'H' => self.line(n.num()? + ox, cy),
                    b'V' => self.line(cx, n.num()? + oy),
                    b'C' => {
                        let v: [f64; 6] = n.take()?;
                        ctrl = (v[2] + ox, v[3] + oy);
                        self.cubic([v[0] + ox, v[1] + oy, v[2] + ox, v[3] + oy, v[4] + ox, v[5] + oy]);
                    }
                    b'S' => {
                        let (v, (ax, ay)): ([f64; 4], _) = (n.take()?, mirror(b"CS"));
                        ctrl = (v[0] + ox, v[1] + oy);
                        self.cubic([ax, ay, v[0] + ox, v[1] + oy, v[2] + ox, v[3] + oy]);
                    }
                    b'Q' => {
                        let v: [f64; 4] = n.take()?;
                        ctrl = (v[0] + ox, v[1] + oy);
                        self.quad([v[0] + ox, v[1] + oy, v[2] + ox, v[3] + oy]);
                    }
                    b'T' => {
                        let ([x, y], at) = (n.take()?, mirror(b"QT"));
                        ctrl = at;
                        self.quad([at.0, at.1, x + ox, y + oy]);
                    }
                    b'A' => {
                        let [rx, ry, turn] = n.take()?;
                        let (large, sweep, [x, y]) = (n.flag()?, n.flag()?, n.take()?);
                        self.arc(rx, ry, turn, (large, sweep), x + ox, y + oy);
                    }
                    b'Z' => {
                        self.close();
                        // nothing may follow a close but another command
                        cmd = 0;
                    }
                    _ => return None,
                }
                prev = up;
            }
        })();
    }

    fn ellipse(&mut self, cx: f64, cy: f64, rx: f64, ry: f64) {
        if rx > 0.0 && ry > 0.0 {
            self.go(cx + rx, cy);
            self.arc(rx, ry, 0.0, (false, true), cx - rx, cy);
            self.arc(rx, ry, 0.0, (false, true), cx + rx, cy);
            self.close();
        }
    }

    /// One element, if it is one that draws.
    fn element(&mut self, name: &str, e: &BytesStart) {
        let n = |k: &str| attr(e, k).and_then(|v| numbers(&v, 1).first().copied());
        let or0 = |k: &str| n(k).unwrap_or(0.0);
        self.down = false;
        match name {
            "path" => self.path(&attr(e, "d").unwrap_or_default()),
            "rect" => {
                let (x, y, w, h) = (or0("x"), or0("y"), or0("width"), or0("height"));
                if w <= 0.0 || h <= 0.0 {
                    return;
                }
                // one radius given stands for both
                let (rx, ry) = (n("rx").or(n("ry")).unwrap_or(0.0).clamp(0.0, w / 2.0), n("ry").or(n("rx")).unwrap_or(0.0).clamp(0.0, h / 2.0));
                if rx == 0.0 || ry == 0.0 {
                    self.go(x, y);
                    for (px, py) in [(x + w, y), (x + w, y + h), (x, y + h)] {
                        self.line(px, py);
                    }
                } else {
                    self.go(x + rx, y);
                    for (lx, ly, ax, ay) in [(x + w - rx, y, x + w, y + ry), (x + w, y + h - ry, x + w - rx, y + h), (x + rx, y + h, x, y + h - ry), (x, y + ry, x + rx, y)] {
                        self.line(lx, ly);
                        self.arc(rx, ry, 0.0, (false, true), ax, ay);
                    }
                }
                self.close();
            }
            "circle" => self.ellipse(or0("cx"), or0("cy"), or0("r"), or0("r")),
            "ellipse" => self.ellipse(or0("cx"), or0("cy"), or0("rx"), or0("ry")),
            "line" => {
                self.go(or0("x1"), or0("y1"));
                self.line(or0("x2"), or0("y2"));
            }
            "polyline" | "polygon" => {
                let pts = numbers(&attr(e, "points").unwrap_or_default(), 200_000);
                for (i, [x, y]) in pts.as_chunks::<2>().0.iter().enumerate() {
                    if i == 0 { self.go(*x, *y) } else { self.line(*x, *y) }
                }
                if name == "polygon" {
                    self.close();
                }
            }
            _ => {}
        }
    }
}

pub(crate) struct Drawing {
    segs: Vec<Seg>,
    shapes: usize,
    view: [f64; 4],
    skipped: BTreeMap<&'static str, usize>,
}

pub(crate) fn drawing(text: &str) -> Result<Drawing> {
    let mut r = quick_xml::Reader::from_str(text);
    let mut pen = Pen { m: SAME, segs: Vec::new(), cur: (0.0, 0.0), start: (0.0, 0.0), down: false };
    let mut stack: Vec<(Mat, bool)> = Vec::new();
    let (mut view, mut root, mut shapes) = (None, false, 0);
    let mut skipped: BTreeMap<&'static str, usize> = BTreeMap::new();
    loop {
        let ev = r.read_event().map_err(|_| anyhow!("can't read this SVG: the file is damaged"))?;
        let (e, open) = match &ev {
            Event::Start(e) => (e, true),
            Event::Empty(e) => (e, false),
            Event::End(_) => {
                stack.pop();
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let name = e.local_name().as_ref().to_string();
        if !root {
            if name != "svg" {
                bail!("not an SVG file");
            }
            root = true;
            view = attr(e, "viewBox").map(|v| numbers(&v, 4)).and_then(|v| <[f64; 4]>::try_from(v).ok()).or_else(|| {
                let side = |k: &str| attr(e, k).filter(|v| !v.trim_end().ends_with('%')).and_then(|v| numbers(&v, 1).first().copied());
                Some([0.0, 0.0, side("width")?, side("height")?])
            });
        }
        let (above, unseen) = stack.last().copied().unwrap_or((SAME, false));
        let (own, understood) = attr(e, "transform").map(|t| transform(&t)).unwrap_or((SAME, true));
        if !understood {
            *skipped.entry("transforms").or_default() += 1;
        }
        let what = match name.as_str() {
            "text" => Some("text"),
            "linearGradient" | "radialGradient" => Some("gradients"),
            "mask" | "clipPath" => Some("masks"),
            "filter" => Some("filters"),
            "image" => Some("pictures"),
            "use" => Some("reused symbols"),
            "pattern" => Some("patterns"),
            "foreignObject" => Some("embedded pages"),
            _ => None,
        };
        if let Some(w) = what {
            *skipped.entry(w).or_default() += 1;
        }
        // what these hold is drawn only when something else refers to it
        let unseen = unseen || what.is_some() || matches!(name.as_str(), "defs" | "symbol" | "marker" | "style" | "script" | "metadata" | "title" | "desc") || attr(e, "display").as_deref() == Some("none");
        let m = mul(above, own);
        if !unseen {
            let before = pen.segs.len();
            pen.m = m;
            pen.element(&name, e);
            shapes += (pen.segs.len() > before) as usize;
            if pen.segs.len() > 20_000 {
                bail!("this drawing is too detailed for a SwiftUI shape (over 20,000 segments)");
            }
        }
        if open {
            if stack.len() >= 256 {
                bail!("can't read this SVG: it is nested too deeply");
            }
            stack.push((m, unseen));
        }
    }
    if !root {
        bail!("not an SVG file");
    }
    if pen.segs.is_empty() {
        bail!("nothing in this SVG can be drawn as a shape");
    }
    // no size given: the drawing is as big as what is in it
    let view = view.filter(|v| v[2] > 0.0 && v[3] > 0.0).unwrap_or_else(|| {
        let pts: Vec<[f64; 2]> = pen
            .segs
            .iter()
            .flat_map(|s| match *s {
                Seg::Move(p) | Seg::Line(p) => vec![p],
                Seg::Cubic(v) => vec![[v[0], v[1]], [v[2], v[3]], [v[4], v[5]]],
                Seg::Quad(v) => vec![[v[0], v[1]], [v[2], v[3]]],
                Seg::Close => vec![],
            })
            .collect();
        let (lo, hi) = pts.iter().fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]));
        // curve arithmetic leaves 1e-16 where it meant 0
        [lo[0], lo[1], (hi[0] - lo[0]).max(1.0), (hi[1] - lo[1]).max(1.0)].map(|v| (v * 1e6).round() / 1e6 + 0.0)
    });
    Ok(Drawing { segs: pen.segs, shapes, view, skipped })
}

/// Three decimals is finer than a pixel at any size an icon is shown.
fn num(v: f64) -> String {
    let r = (v.clamp(-1e9, 1e9) * 1000.0).round() / 1000.0;
    if r == 0.0 || !r.is_finite() { "0".into() } else { format!("{r}") }
}

pub(crate) fn shape_source(d: &Drawing, name: &str, from: &str) -> String {
    let p = |x: f64, y: f64| format!("p({}, {})", num(x), num(y));
    let [x, y, w, h] = d.view;
    let from: String = from.chars().filter(|c| !c.is_control() && !matches!(c, '\u{2028}' | '\u{2029}')).take(120).collect();
    let mut o = format!(
        "import SwiftUI\n\n/// Drawn from {from}. Use it like any shape: {name}().fill(.primary).frame(width: 64, height: 64)\nstruct {name}: Shape {{\n    func path(in rect: CGRect) -> Path {{\n        // the drawing's own box is {w} × {h}; it is fitted inside rect and centred\n        let s = min(rect.width / {w}, rect.height / {h})\n        let ox = rect.midX - {cx} * s\n        let oy = rect.midY - {cy} * s\n        func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint {{ CGPoint(x: ox + x * s, y: oy + y * s) }}\n        var path = Path()\n",
        w = num(w),
        h = num(h),
        cx = num(x + w / 2.0),
        cy = num(y + h / 2.0)
    );
    for s in &d.segs {
        o.push_str(&match *s {
            Seg::Move([x, y]) => format!("        path.move(to: {})\n", p(x, y)),
            Seg::Line([x, y]) => format!("        path.addLine(to: {})\n", p(x, y)),
            Seg::Cubic(v) => format!("        path.addCurve(to: {}, control1: {}, control2: {})\n", p(v[4], v[5]), p(v[0], v[1]), p(v[2], v[3])),
            Seg::Quad(v) => format!("        path.addQuadCurve(to: {}, control: {})\n", p(v[2], v[3]), p(v[0], v[1])),
            Seg::Close => "        path.closeSubpath()\n".to_string(),
        });
    }
    o + "        return path\n    }\n}\n"
}

fn swiftui(src: &Path) -> Result<Vec<PathBuf>> {
    let text = String::from_utf8(crate::engines::files::read_capped(src, 20)?).map_err(|_| anyhow!("not an SVG file"))?;
    let d = drawing(text.trim_start_matches('\u{feff}'))?;
    let base = pascal(&words(&stem(src)));
    let name = format!("{}Shape", if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit()) { format!("Icon{base}") } else { base });
    let code = shape_source(&d, &name, &name_of(src));
    let out = out_path(src, Some("Shape"), "swift");
    std::fs::write(&out, &code)?;
    let copied = if clip(&code)? { ", copied" } else { "" };
    let skipped = if d.skipped.is_empty() { String::new() } else { format!("; skipped {}", d.skipped.iter().map(|(k, n)| format!("{n} {k}")).collect::<Vec<_>>().join(", ")) };
    note(&format!("{name}: {} shapes{copied}{skipped}", d.shapes));
    Ok(vec![out])
}

// ---------------------------------------------------------------- what is inside a package

pub(crate) struct Item {
    name: String,
    packed: u64,
    size: u64,
    /// where its line in the list is, and where the list says its data starts
    listed: u64,
    data: u64,
}

fn le(b: &[u8], at: usize, n: usize) -> Option<u64> {
    Some(b.get(at..at.checked_add(n)?)?.iter().rev().fold(0u64, |v, x| v << 8 | *x as u64))
}

/// The list at the end of a zip: names and the sizes it claims, without touching the contents.
/// Returns where that list starts as well; Android keeps its signature just before it.
pub(crate) fn central(f: &mut (impl Read + Seek)) -> Option<(Vec<Item>, u64)> {
    let len = f.seek(SeekFrom::End(0)).ok()?;
    let tail_at = len.saturating_sub(66_000);
    let mut tail = Vec::new();
    f.seek(SeekFrom::Start(tail_at)).ok()?;
    f.by_ref().take(66_000).read_to_end(&mut tail).ok()?;
    let end = (0..=tail.len().checked_sub(22)?).rev().find(|i| tail.get(*i..*i + 4) == Some(b"PK\x05\x06"))?;
    let (mut n, mut cd_len, mut cd_at) = (le(&tail, end + 10, 2)?, le(&tail, end + 12, 4)?, le(&tail, end + 16, 4)?);
    // archives past 4 GB or 65,535 entries keep the real numbers in a second record
    if n == 0xFFFF || cd_len == 0xFFFF_FFFF || cd_at == 0xFFFF_FFFF {
        let loc = end.checked_sub(20).filter(|l| tail.get(*l..*l + 4) == Some(b"PK\x06\x07"))?;
        let mut big = [0u8; 56];
        f.seek(SeekFrom::Start(le(&tail, loc + 8, 8)?)).ok()?;
        f.read_exact(&mut big).ok()?;
        if !big.starts_with(b"PK\x06\x06") {
            return None;
        }
        (n, cd_len, cd_at) = (le(&big, 32, 8)?, le(&big, 40, 8)?, le(&big, 48, 8)?);
    }
    if cd_len > 256 << 20 || cd_at.checked_add(cd_len)? > len {
        return None;
    }
    let mut cd = Vec::new();
    f.seek(SeekFrom::Start(cd_at)).ok()?;
    f.by_ref().take(cd_len).read_to_end(&mut cd).ok()?;
    let (mut items, mut at) = (Vec::new(), 0usize);
    while (items.len() as u64) < n.min(2_000_000) && cd.get(at..at + 4) == Some(b"PK\x01\x02") {
        let (mut packed, mut size) = (le(&cd, at + 20, 4)?, le(&cd, at + 24, 4)?);
        let (name_len, extra_len, note_len) = (le(&cd, at + 28, 2)? as usize, le(&cd, at + 30, 2)? as usize, le(&cd, at + 32, 2)? as usize);
        let name = String::from_utf8_lossy(cd.get(at + 46..at + 46 + name_len)?).into_owned();
        let extra = cd.get(at + 46 + name_len..at + 46 + name_len + extra_len)?;
        let mut x = 0;
        while let (Some(id), Some(l)) = (le(extra, x, 2), le(extra, x + 2, 2)) {
            if id == 1 {
                let mut q = x + 4;
                if size == 0xFFFF_FFFF {
                    size = le(extra, q, 8).unwrap_or(size);
                    q += 8;
                }
                if packed == 0xFFFF_FFFF {
                    packed = le(extra, q, 8).unwrap_or(packed);
                }
            }
            x += 4 + l as usize;
        }
        items.push(Item { name, packed, size, listed: cd_at + at as u64, data: le(&cd, at + 42, 4)? });
        at += 46 + name_len + extra_len + note_len;
    }
    Some((items, cd_at))
}

/// The top-level fields of an Info.plist, binary or XML, as plutil reads them.
fn plist(src: &Path, name: &str) -> Result<Value> {
    let tmp = temp_file("plist");
    let read = (|| -> Result<Value> {
        std::fs::write(&tmp, entry(src, name, 4)?)?;
        let o = crate::engines::tools::output_within(Command::new("/usr/bin/plutil").args(["-convert", "json", "-o", "-", "--"]).arg(&tmp), Duration::from_secs(30))?;
        if !o.status.success() {
            bail!("plutil could not read it");
        }
        Ok(serde_json::from_slice(&o.stdout)?)
    })();
    let _ = std::fs::remove_file(&tmp);
    read
}

fn bundle(src: &Path) -> Result<Vec<PathBuf>> {
    let mut f = std::fs::File::open(src)?;
    let (items, cd_at) = central(&mut f).context("can't read this package: it is not a zip archive, or it is damaged")?;
    let files: Vec<&Item> = items.iter().filter(|i| !i.name.ends_with('/')).collect();
    if files.is_empty() {
        bail!("this package is empty");
    }
    let (packed, full) = files.iter().fold((0u64, 0u64), |a, i| (a.0.saturating_add(i.packed), a.1.saturating_add(i.size)));
    let on_disk = f.metadata()?.len();
    let mut md = format!("# {}\n\n{} files · {} on disk · {} unpacked", code(&name_of(src)), count(files.len() as u64), size(on_disk), size(full));
    if full > 0 {
        md.push_str(&format!(" · packed to {}%", (packed as f64 / full as f64 * 100.0).round()));
    }
    md.push_str("\n\nSizes are what the archive says about itself; nothing was unpacked.\n");
    let listed = |title: &str, all: &mut Vec<String>| {
        all.sort();
        all.dedup();
        let shown: Vec<String> = all.iter().take(60).map(|n| code(n)).collect();
        format!("- {title} ({}): {}{}\n", all.len(), if shown.is_empty() { "none".to_string() } else { shown.join(", ") }, if all.len() > 60 { ", …" } else { "" })
    };
    let kind = ext(src);
    if kind == "ipa" {
        md.push_str("\n## App\n\n");
        let parts = |i: &&Item| i.name.split('/').map(str::to_string).collect::<Vec<_>>();
        let info = files.iter().find(|i| matches!(parts(i).as_slice(), [p, app, last] if p == "Payload" && app.ends_with(".app") && last == "Info.plist"));
        match info.map(|i| plist(src, &i.name)) {
            Some(Ok(p)) => {
                let s = |k: &str| p.get(k).and_then(Value::as_str).map(code);
                for (label, v) in [
                    ("Name", s("CFBundleDisplayName").or(s("CFBundleName"))),
                    ("Bundle ID", s("CFBundleIdentifier")),
                    ("Version", s("CFBundleShortVersionString").map(|v| format!("{v} (build {})", s("CFBundleVersion").unwrap_or_else(|| "?".into())))),
                    ("Minimum iOS", s("MinimumOSVersion")),
                    ("Built with", s("DTXcode").map(|x| format!("Xcode {x}, SDK {}", s("DTSDKName").unwrap_or_else(|| "?".into())))),
                ] {
                    if let Some(v) = v {
                        md.push_str(&format!("- {label}: {v}\n"));
                    }
                }
                let asks: Vec<(&String, &str)> = p.as_object().into_iter().flatten().filter(|(k, _)| k.starts_with("NS") && k.ends_with("UsageDescription")).filter_map(|(k, v)| Some((k, v.as_str()?))).collect();
                md.push_str(&format!("- Permissions it asks for ({}){}\n", asks.len(), if asks.is_empty() { ": none" } else { ":" }));
                for (k, why) in asks.iter().take(80) {
                    md.push_str(&format!("  - {}: {}\n", code(k.trim_start_matches("NS").trim_end_matches("UsageDescription")), code(why)));
                }
            }
            Some(Err(_)) => md.push_str("- Info.plist is there but couldn't be read\n"),
            None => md.push_str("- No Payload/…app/Info.plist: this may not be an iOS app\n"),
        }
        let inside = |folder: &str, tail: &str| -> Vec<String> {
            let found = |i: &&Item| -> Option<String> {
                let p = parts(i);
                let at = p.iter().position(|x| x == folder)?;
                p.get(at + 1).filter(|n| n.ends_with(tail) && p.len() > at + 2 || tail == ".dylib" && n.ends_with(tail)).cloned()
            };
            files.iter().filter_map(found).collect()
        };
        let mut frameworks = inside("Frameworks", ".framework");
        frameworks.extend(inside("Frameworks", ".dylib"));
        md.push_str(&listed("Frameworks", &mut frameworks));
        md.push_str(&listed("App extensions", &mut inside("PlugIns", ".appex")));
    }
    if matches!(kind.as_str(), "apk" | "aab" | "jar" | "aar" | "war") {
        md.push_str(&format!("\n## {}\n\n", if matches!(kind.as_str(), "apk" | "aab") { "Android" } else { "Java" }));
        let ends = |tail: &str| files.iter().filter(|i| i.name.ends_with(tail)).count();
        if matches!(kind.as_str(), "apk" | "aab") {
            md.push_str(&format!("- Dex files: {}\n", ends(".dex")));
            let mut abis: Vec<String> = files.iter().filter(|i| i.name.ends_with(".so")).filter_map(|i| i.name.split('/').rev().nth(1).filter(|_| i.name.split('/').rev().nth(2) == Some("lib")).map(str::to_string)).collect();
            md.push_str(&listed("Native code for", &mut abis));
        } else {
            md.push_str(&format!("- Classes: {}\n", count(ends(".class") as u64)));
        }
        let v1 = files.iter().any(|i| i.name.strip_prefix("META-INF/").is_some_and(|n| !n.contains('/') && [".RSA", ".EC", ".DSA"].iter().any(|e| n.ends_with(e))));
        // the newer schemes sign the whole file and leave a marked block right before the list
        let mut mark = [0u8; 16];
        let block = cd_at >= 16 && f.seek(SeekFrom::Start(cd_at - 16)).is_ok() && f.read_exact(&mut mark).is_ok() && &mark == b"APK Sig Block 42";
        md.push_str(&format!(
            "- Signature: {}\n",
            match (v1, block, kind.as_str()) {
                (true, true, _) => "signed (JAR signature in META-INF, and an APK signing block: scheme v2 or later)",
                (true, false, _) => "signed the old way (JAR signature in META-INF, scheme v1)",
                (false, true, _) => "an APK signing block is present (scheme v2 or later), no v1 signature",
                (false, false, "apk") => "no signature found: this APK is unsigned and a phone will refuse it",
                _ => "no JAR signature found in META-INF",
            }
        ));
        md.push_str("- Only the presence of a signature is checked here, not whether it is valid or whose it is\n");
    }
    md.push_str("\n## Largest\n\n");
    let mut big = files.clone();
    big.sort_by(|a, b| b.size.cmp(&a.size).then(a.name.cmp(&b.name)));
    for i in big.iter().take(20) {
        md.push_str(&format!("- {} — {} ({} packed)\n", code(&i.name), size(i.size), size(i.packed)));
    }
    md.push_str("\n## By top-level folder\n\n");
    let mut tops: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for i in &files {
        let t = tops.entry(i.name.split_once('/').map_or("(top level)", |p| p.0)).or_default();
        *t = (t.0 + 1, t.1.saturating_add(i.size));
    }
    let mut tops: Vec<(&str, (u64, u64))> = tops.into_iter().collect();
    tops.sort_by_key(|t| std::cmp::Reverse(t.1 .1));
    for (name, (n, bytes)) in tops.iter().take(40) {
        md.push_str(&format!("- {} — {} files, {}\n", code(name), count(*n), size(*bytes)));
    }
    if tops.len() > 40 {
        md.push_str(&format!("- and {} more folders\n", tops.len() - 40));
    }
    let out = out_path(src, Some("Contents"), "md");
    std::fs::write(&out, md)?;
    note(&format!("{} files, {} on disk, {} unpacked", count(files.len() as u64), size(on_disk), size(full)));
    Ok(vec![out])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Built at run time so that no scanner watching this repository sees a token in the source.
    fn fake(prefix: &str, n: usize) -> String {
        format!("{prefix}{}", "aB3dE5gH7jK9mN1pQ2sT4vW6xY8zC0fR".chars().cycle().take(n).collect::<String>())
    }

    fn kinds(text: &str) -> Vec<&'static str> {
        scan(text, false).iter().map(|h| h.kind).collect()
    }

    #[test]
    fn tokens_by_shape() {
        let upper = |p: &str, n: usize| format!("{p}{}", "Q7ZK2MX9PL4TW8RB".chars().cycle().take(n).collect::<String>());
        for (kind, hit, miss) in [
            ("Private key", "-----BEGIN RSA PRIVATE KEY-----".to_string(), "-----BEGIN PUBLIC KEY-----".to_string()),
            ("Private key", "-----BEGIN OPENSSH PRIVATE KEY-----".into(), "-----BEGIN CERTIFICATE-----".into()),
            (AWS_ID, format!("key = {}", upper("AKIA", 16)), format!("key = {}", upper("AKIA", 15))),
            (AWS_ID, upper("ASIA", 16), "AKIAIOSFODNN7EXAMPLE".into()),
            ("GitHub token", fake("ghp_", 36), fake("ghp_", 20)),
            ("GitHub token", fake("github_pat_", 82), fake("github_pat_", 30)),
            ("GitLab token", fake("glpat-", 20), fake("glpat-", 10)),
            ("Slack token", format!("xoxb-{}", fake("1234567890-", 24)), "xoxb-your-token".into()),
            ("Stripe live key", fake("sk_live_", 24), fake("sk_test_", 24)),
            ("Stripe live key", fake("rk_live_", 24), fake("pk_live_", 24)),
            ("Google API key", fake("AIza", 35), fake("AIza", 30)),
            ("OpenAI-style API key", fake("sk-", 48), "class=\"sk-spinner-wrapper-with-a-very-long-name-here\"".into()),
            ("OpenAI-style API key", fake("sk-proj-", 60), fake("task-", 60)),
            ("Anthropic API key", fake("sk-ant-api03-", 95), "sk-ant-api03-short".into()),
            ("JSON Web Token", format!("{}.{}.{}", fake("eyJ", 33), fake("eyJ", 60), fake("", 43)), format!("{}.{}", fake("eyJ", 33), fake("eyJ", 60))),
        ] {
            assert_eq!(kinds(&hit), vec![kind], "{hit}");
            assert_eq!(kinds(&miss), Vec::<&str>::new(), "{miss}");
            // what is shown is never enough to use
            let shown = &scan(&hit, false)[0].shown;
            assert!(shown.chars().count() <= 11 && !hit.contains(shown.as_str()), "{shown}");
        }
        // a token in the middle of a longer word is not one
        assert!(kinds(&format!("x{}", fake("ghp_", 36))).is_empty());
    }

    #[test]
    fn assignments_and_placeholders() {
        for hit in [
            "password = \"hunter2-correct\"",
            "  \"api_key\": \"q8Zk2mX9pL4tW8rB\",",
            "const apiKey = 'q8Zk2mX9pL4tW8rB';",
            "DB_PASSWORD: \"s3cr3t-value\"",
            "client_secret => 'q8Zk2mX9pL4tW8rB'",
            "auth_token := `q8Zk2mX9pL4tW8rB`",
        ] {
            assert_eq!(kinds(hit), vec![ASSIGNED], "{hit}");
        }
        for miss in [
            "password = \"\"",
            "password = \"xxx\"",
            "password = \"changeme\"",
            "password = \"${DB_PASSWORD}\"",
            "password = \"<your password>\"",
            "password = process.env.PASSWORD",
            "token = os.environ[\"TOKEN\"]",
            "token_type = \"Bearer-JWT\"",
            "if (password == \"hunter2-correct\") {",
            "password: \"Enter your password\"",
            "secret = \"your_secret_here\"",
            "<input type=\"password\" name=\"password\">",
            "tokens = \"q8Zk2mX9pL4tW8rB\"",
            "PASSWORD=hunter2-correct",
        ] {
            assert!(kinds(miss).is_empty(), "{miss}");
        }
        // .env files write values bare
        assert_eq!(scan("PASSWORD=hunter2-correct # prod\nTOKEN=\nAPI_KEY=${KEY}\n", true).iter().map(|h| (h.line, h.shown.as_str())).collect::<Vec<_>>(), vec![(1, "hunt…")]);
        assert_eq!(mask("hunter2"), "hu…");
        assert_eq!(mask("abc"), "a…");
    }

    #[test]
    fn aws_secret_only_near_its_id() {
        let secret = fake("", 40);
        let id = format!("AKIA{}", "Q7ZK2MX9PL4TW8RB");
        assert_eq!(kinds(&format!("id: {id}\nkey: {secret}\n")), vec![AWS_ID, AWS_SECRET]);
        assert_eq!(kinds(&format!("{secret}\n\n\n{id}\n")), vec![AWS_SECRET, AWS_ID]);
        assert_eq!(kinds(&format!("aws_secret_access_key = \"{secret}\"\n")), vec![AWS_SECRET]);
        // forty characters far from any key ID are a hash, or anything
        assert!(kinds(&format!("{id}\n\n\n\n\nchecksum {secret}\n")) == vec![AWS_ID]);
        assert!(kinds("commit 3f786850e387550fdab836ed7e6dc881de23001b\n").is_empty());
    }

    #[test]
    fn log_lines_to_templates() {
        let t = |s: &str| template(s).0;
        assert_eq!(template("2026-11-13T04:36:00.123Z ERROR db timeout after 30s host=10.0.0.5:5432"), ("<time> ERROR db timeout after <n>s host=<ip>".to_string(), Some("2026-11-13T04:36:00.123Z")));
        assert_eq!(template("Nov  3 04:36:00 mac kernel[0]: x").1, Some("Nov  3 04:36:00"));
        assert_eq!(template("1.2.3.4 - - [13/Nov/2026:04:36:00 +0700] \"GET /a/b HTTP/1.1\" 200 512").0, "<ip> - - [<time>] <str> <n> <n>");
        assert_eq!(t("user 7f3c9a2e-1b2c-4d5e-8f90-123456789abc opened /Users/an/Tài liệu/a.txt:12"), "user <uuid> opened <path> liệu/a.txt:<n>");
        assert_eq!(t("req=8f3a2b1c ptr=0xDEADBEEF id=abc12345xyz took 12.5ms"), "req=<hex> ptr=<hex> id=<id> took <n>ms");
        assert_eq!(t("{\"level\":\"error\",\"msg\":\"can't open file 12\",\"n\":3}"), "{\"level\":\"error\",\"msg\":<str>,\"n\":<n>}");
        assert_eq!(t("GET /health and/or utf8 it's fine"), "GET /health and/or utf8 it's fine");
        assert_eq!(t("fetch https://example.com/a?b=1 failed: 'no route'"), "fetch <url> failed: <str>");
        assert_eq!(t("Lỗi kết nối 5 lần"), "Lỗi kết nối <n> lần");
        assert!(t(&"word ".repeat(200)).chars().count() < 250);
        // never panics on bytes that end mid-way through anything
        for s in ["\"", "'", "/", "0x", "12:", "2026-", "~/", "é\"é", "1.2.3.", "a\u{0}b"] {
            let _ = template(s);
        }
    }

    #[test]
    fn log_levels() {
        assert_eq!(severity("<time> ERROR db timeout"), 2);
        assert_eq!(severity("<time> INFO no error found"), 0);
        assert_eq!(severity("java.lang.NullPointerException at <path>"), 2);
        assert_eq!(severity("thread 'main' panicked at <path>"), 2);
        assert_eq!(severity("{\"level\":\"warn\",\"msg\":<str>}"), 1);
        assert_eq!(severity("<time> WARN retry <n> after error"), 1);
        assert_eq!(severity("terror level was high"), 0);
        assert_eq!(severity("request served"), 0);
    }

    #[test]
    fn lines_of_any_length() {
        let mut got = Vec::new();
        let long = "x".repeat(200_000);
        each_line(format!("a\n\n{long}\nlast").as_bytes(), 8, |l| {
            got.push(String::from_utf8_lossy(l).into_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!(got, vec!["a", "", "xxxxxxxx", "last"]);
    }

    #[test]
    fn names_from_keys() {
        assert_eq!(words("user_id"), ["user", "id"]);
        assert_eq!(words("userID"), ["user", "id"]);
        assert_eq!(words("HTTPServer2Go"), ["http", "server2", "go"]);
        assert_eq!(words("Họ và tên"), ["họ", "và", "tên"]);
        assert_eq!(words("日本"), Vec::<String>::new());
        assert_eq!(swift_name(&words("first-name")), "firstName");
        assert_eq!(swift_name(&words("2fa")), "_2fa");
        assert_eq!(rust_name(&words("firstName")), "first_name");
        assert_eq!(go_name(&words("first name")), "FirstName");
        assert_eq!(go_name(&words("2fa")), "X2fa");
    }

    fn shapes(text: &str) -> (Shapes, Ty) {
        let v: Value = serde_json::from_str(text).unwrap();
        let mut s = Shapes::default();
        let top = s.of("order", &[&v]).unwrap();
        (s, top)
    }

    const SAMPLE: &str = r#"{"id":1,"class":"a","first-name":"x","total":1.5,"tags":["a"],"owner":{"name":"x","age":null},"items":[{"sku":"a","qty":1,"price":2},{"sku":"b","price":2.5,"note":null}],"mixed":[1,"a"],"none":[]}"#;

    #[test]
    fn swift_from_json() {
        let (s, top) = shapes(SAMPLE);
        let out = swift(&s, &top, "OrderFile");
        assert!(out.contains("struct Order: Codable {\n    var id: Int\n    var `class`: String\n    var firstName: String\n    var total: Double\n    var tags: [String]\n    var owner: Owner\n    var items: [Item]\n    var mixed: [JSONValue]\n    var none: [JSONValue]\n"), "{out}");
        assert!(out.contains("        case `class`\n        case firstName = \"first-name\"\n"), "{out}");
        assert!(out.contains("struct Item: Codable {\n    var sku: String\n    var qty: Int?\n    var price: Double\n    var note: JSONValue?\n}"), "{out}");
        assert!(out.contains("struct Owner: Codable {\n    var name: String\n    var age: JSONValue\n}"), "{out}");
        assert!(out.contains("enum JSONValue: Codable") && !out.contains("typealias"));
        assert!(swift(&shapes("[{\"a\":1}]").0, &shapes("[{\"a\":1}]").1, "OrderFile").contains("typealias OrderFile = [Order]\n"));
        assert_eq!(swift_str("a\"b\\c\nd"), "\"a\\\"b\\\\c\\u{A}d\"");
    }

    #[test]
    fn rust_go_and_schema_from_json() {
        let (s, top) = shapes(SAMPLE);
        let r = rust(&s, &top, "OrderFile");
        assert!(r.contains("pub struct Order {\n    pub id: i64,\n    pub class: String,\n    #[serde(rename = \"first-name\")]\n    pub first_name: String,\n    pub total: f64,\n    pub tags: Vec<String>,\n    pub owner: Owner,\n    pub items: Vec<Item>,\n    pub mixed: Vec<serde_json::Value>,"), "{r}");
        assert!(r.contains("    #[serde(default, skip_serializing_if = \"Option::is_none\")]\n    pub qty: Option<i64>,\n    pub price: f64,"), "{r}");
        let g = go(&s, &top, "OrderFile");
        assert!(g.contains("type Order struct {\n\tId int64 `json:\"id\"`\n\tClass string `json:\"class\"`\n\tFirstName string `json:\"first-name\"`\n\tTotal float64 `json:\"total\"`\n\tTags []string `json:\"tags\"`\n\tOwner Owner `json:\"owner\"`\n\tItems []Item `json:\"items\"`\n\tMixed []any `json:\"mixed\"`"), "{g}");
        assert!(g.contains("\tQty *int64 `json:\"qty,omitempty\"`\n"), "{g}");
        let j = schema_of(&top, &s);
        assert_eq!(j["properties"]["items"]["items"]["required"], serde_json::json!(["sku", "price"]));
        assert_eq!(j["properties"]["items"]["items"]["properties"]["price"], serde_json::json!({"type": "number"}));
        assert_eq!(j["properties"]["id"], serde_json::json!({"type": "integer"}));
        let (s, top) = shapes(r#"[{"a":"x","b":{"c":1}},{"a":null,"b":null}]"#);
        let j = schema_of(&top, &s);
        assert_eq!(j["items"]["properties"]["a"], serde_json::json!({"type": ["string", "null"]}));
        assert_eq!(j["items"]["properties"]["b"]["type"], serde_json::json!(["object", "null"]));
        assert!(rust(&s, &top, "F").contains("pub b: Option<B>,") && swift(&s, &top, "F").contains("var b: B?") && go(&s, &top, "F").contains("B *B `json:\"b\"`"));
        // names that collide after respelling, and names a language keeps for itself
        let (s, top) = shapes(r#"{"a_b":1,"aB":2,"self":3,"type":4,"":5,"String":{"x":1}}"#);
        assert!(rust(&s, &top, "F").contains("pub a_b: i64,\n    #[serde(rename = \"aB\")]\n    pub a_b2: i64,\n    #[serde(rename = \"self\")]\n    pub self_: i64,\n    pub r#type: i64,\n    #[serde(rename = \"\")]\n    pub field: i64,\n    #[serde(rename = \"String\")]\n    pub string: StringItem,"), "{}", rust(&s, &top, "F"));
        assert!(swift(&s, &top, "F").contains("case self_ = \"self\"\n        case type_ = \"type\"\n        case field = \"\"\n        case string = \"String\""), "{}", swift(&s, &top, "F"));
        assert!(go(&s, &top, "F").contains("\tField int64 `json:\"-\"` // this key"), "{}", go(&s, &top, "F"));
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn pen() -> Pen {
        Pen { m: SAME, segs: Vec::new(), cur: (0.0, 0.0), start: (0.0, 0.0), down: false }
    }

    #[test]
    fn path_commands() {
        let mut p = pen();
        p.path("M10 10 20 10h10v10H10zm5,5l1-1");
        assert_eq!(p.segs, vec![Seg::Move([10.0, 10.0]), Seg::Line([20.0, 10.0]), Seg::Line([30.0, 10.0]), Seg::Line([30.0, 20.0]), Seg::Line([10.0, 20.0]), Seg::Close, Seg::Move([15.0, 15.0]), Seg::Line([16.0, 14.0])]);
        let mut p = pen();
        p.path("M0 0C1 1 2 1 3 0S5-1 6 0Q7 1 8 0T10 0 .5.5-1e1");
        assert_eq!(p.segs[1..5], [Seg::Cubic([1.0, 1.0, 2.0, 1.0, 3.0, 0.0]), Seg::Cubic([4.0, -1.0, 5.0, -1.0, 6.0, 0.0]), Seg::Quad([7.0, 1.0, 8.0, 0.0]), Seg::Quad([9.0, -1.0, 10.0, 0.0])]);
        assert_eq!(p.segs[5], Seg::Quad([11.0, 1.0, 0.5, 0.5]));
        // relative curves, and garbage ends the path without losing what came before
        let mut p = pen();
        p.path("m1 1c1 0 1 1 2 2 L 5 # 9");
        assert_eq!(p.segs, vec![Seg::Move([1.0, 1.0]), Seg::Cubic([2.0, 1.0, 2.0, 2.0, 3.0, 3.0])]);
        for d in ["", "Z", "5 5", "M", "M1", "A1 1 0 0", "Mz1e999", "M0 0A0 0 0 0 0 5 5", "M0 0a1 1 0 2 0 1 1", "M0 0Z1 1"] {
            pen().path(d);
        }
    }

    #[test]
    fn arcs_become_curves() {
        // half a unit circle, clockwise on screen: through the bottom
        let mut p = pen();
        p.path("M1 0A1 1 0 0 1 -1 0");
        let k = 4.0 / 3.0 * (std::f64::consts::FRAC_PI_8).tan();
        let [Seg::Move(_), Seg::Cubic(a), Seg::Cubic(b)] = p.segs.as_slice() else { panic!("{:?}", p.segs) };
        for (got, want) in a.iter().zip([1.0, k, k, 1.0, 0.0, 1.0]).chain(b.iter().zip([-k, 1.0, -1.0, k, -1.0, 0.0])) {
            assert!(close(*got, want), "{a:?} {b:?}");
        }
        // flags packed against the numbers, and the other way round goes over the top
        let mut p = pen();
        p.path("M1 0a1 1 0 00-2 0");
        let [_, Seg::Cubic(a), Seg::Cubic(_)] = p.segs.as_slice() else { panic!("{:?}", p.segs) };
        assert!(close(a[4], 0.0) && close(a[5], -1.0), "{a:?}");
        // radii too short to span the gap are stretched, not dropped
        let mut p = pen();
        p.path("M0 0A1 1 0 0 1 10 0");
        assert!(matches!(p.segs.last(), Some(Seg::Cubic(v)) if close(v[4], 10.0) && close(v[5], 0.0)));
        // every point of a circle stays on it
        let mut p = pen();
        p.ellipse(5.0, 5.0, 3.0, 3.0);
        assert_eq!(p.segs.len(), 6);
        for s in &p.segs {
            if let Seg::Cubic(v) = s {
                assert!(close((v[4] - 5.0).hypot(v[5] - 5.0), 3.0));
            }
        }
    }

    #[test]
    fn transforms_and_shapes() {
        assert_eq!(transform("translate(10,20) scale(2)"), ([2.0, 0.0, 0.0, 2.0, 10.0, 20.0], true));
        assert_eq!(transform("matrix(1 2 3 4 5 6)").0, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let (r, ok) = transform("rotate(90)");
        assert!(ok && close(r[0], 0.0) && close(r[1], 1.0) && close(r[2], -1.0));
        assert!(!transform("perspective(3)").1 && !transform("translate(1").1);
        let d = drawing(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><defs><linearGradient id="g"/><rect width="5" height="5"/></defs><g transform="translate(2 2)"><g transform="scale(2)"><rect x="1" y="1" width="2" height="3"/></g><line x1="0" y1="0" x2="1" y2="1"/></g><text>hi</text><polygon points="0,0 4,0 4,4"/></svg>"#).unwrap();
        assert_eq!(d.segs[..5], [Seg::Move([4.0, 4.0]), Seg::Line([8.0, 4.0]), Seg::Line([8.0, 10.0]), Seg::Line([4.0, 10.0]), Seg::Close]);
        assert_eq!(d.segs[5..7], [Seg::Move([2.0, 2.0]), Seg::Line([3.0, 3.0])]);
        assert_eq!((d.shapes, d.view), (3, [0.0, 0.0, 24.0, 24.0]));
        assert_eq!(d.skipped.iter().map(|(k, n)| (*k, *n)).collect::<Vec<_>>(), vec![("gradients", 1), ("text", 1)]);
        let rounded = drawing(r#"<svg width="10" height="10"><rect width="10" height="10" rx="2"/></svg>"#).unwrap();
        assert_eq!(rounded.segs.iter().filter(|s| matches!(s, Seg::Cubic(_))).count(), 4);
        let src = shape_source(&d, "IconShape", "a\nb.svg");
        assert!(src.contains("/// Drawn from ab.svg.") && src.contains("let s = min(rect.width / 24, rect.height / 24)") && src.contains("path.move(to: p(4, 4))\n        path.addLine(to: p(8, 4))"), "{src}");
        // no size anywhere: the box is what was drawn
        assert_eq!(drawing(r#"<svg><circle cx="5" cy="5" r="5"/></svg>"#).unwrap().view, [0.0, 0.0, 10.0, 10.0]);
        assert!(drawing("<html><body/></html>").is_err() && drawing("<svg><text>x</text></svg>").is_err() && drawing("<svg><g></svg>").is_err() && drawing("").is_err());
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(1.23456), "1.235");
    }

    /// The smallest zip that lists `names`, each claiming `size` bytes.
    fn listing(names: &[&str], size: u32) -> Vec<u8> {
        let mut cd = Vec::new();
        for n in names {
            cd.extend_from_slice(b"PK\x01\x02");
            cd.extend_from_slice(&[0; 16]);
            cd.extend_from_slice(&7u32.to_le_bytes());
            cd.extend_from_slice(&size.to_le_bytes());
            cd.extend_from_slice(&(n.len() as u16).to_le_bytes());
            cd.extend_from_slice(&[0; 16]);
            cd.extend_from_slice(n.as_bytes());
        }
        let mut z = b"junk before".to_vec();
        let at = z.len() as u32;
        z.extend_from_slice(&cd);
        z.extend_from_slice(b"PK\x05\x06\0\0\0\0");
        z.extend_from_slice(&(names.len() as u16).to_le_bytes());
        z.extend_from_slice(&(names.len() as u16).to_le_bytes());
        z.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        z.extend_from_slice(&at.to_le_bytes());
        z.extend_from_slice(&[0, 0]);
        z
    }

    #[test]
    fn zip_listing_without_unpacking() {
        let z = listing(&["a.txt", "../../evil", "dir/b\nc"], 4_000_000_000);
        let (items, at) = central(&mut std::io::Cursor::new(&z)).unwrap();
        assert_eq!(items.iter().map(|i| (i.name.as_str(), i.packed, i.size)).collect::<Vec<_>>(), vec![("a.txt", 7, 4_000_000_000), ("../../evil", 7, 4_000_000_000), ("dir/b\nc", 7, 4_000_000_000)]);
        assert_eq!(at, 11);
        // cut anywhere, or pointing past its own end, it is refused rather than trusted
        for cut in 0..z.len() {
            let _ = central(&mut std::io::Cursor::new(&z[..cut]));
        }
        let mut lying = z.clone();
        let n = lying.len();
        lying[n - 6..n - 2].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        assert!(central(&mut std::io::Cursor::new(&lying)).is_none());
        assert!(central(&mut std::io::Cursor::new(b"not a zip at all, just some text".to_vec())).is_none());
    }

    #[test]
    fn small_things() {
        assert_eq!(count(1234567), "1,234,567");
        assert_eq!(count(12), "12");
        assert_eq!(size(999), "999 bytes");
        assert_eq!(size(1_500_000), "1.5 MB");
        assert_eq!(code("a`b\nc\u{202e}d"), "`a'b c d`");
        assert_eq!(code("\n"), "`(no name)`");
        assert_eq!(unpacked(".git", false), Some(".git"));
        assert_eq!(unpacked("build", false), None);
        assert_eq!(unpacked("build", true), Some("build"));
        assert_eq!(unpacked(".env.local", false), Some(".env files"));
        assert_eq!(unpacked("server.key", false), Some("key files"));
        assert_eq!(unpacked("keys", true), None);
        assert!(!is_group("kit", "pack"));
        assert!(run(&[PathBuf::from("/x")], "sheet", "xlsx").is_none() && run(&[PathBuf::from("/x")], "kit", "to=yaml").is_none());
    }
}
