//! JSON lines between the app and this process. stdin: one request. stdout: events.
use crate::engines::{self, Action};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::os::fd::FromRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const VERSION: u32 = 1;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    v: u32,
    #[serde(default)]
    paths: Vec<PathBuf>,
    /// Absent for `verb` requests.
    action: Option<Action>,
    #[serde(default)]
    verb: String,
    #[serde(default)]
    unattended: bool,
    bin_dir: Option<PathBuf>,
    #[serde(default)]
    secrets: HashMap<String, String>,
    #[serde(default)]
    settings: HashMap<String, String>,
}

static OUT: Mutex<Option<std::fs::File>> = Mutex::new(None);
static CLAIMS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static WAKE: OnceLock<i32> = OnceLock::new();
/// Set the moment a stop begins: helpers dying then is the stop, not a failure to report.
static STOPPING: AtomicBool = AtomicBool::new(false);

pub fn emit(v: Value) {
    if let Some(f) = OUT.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let mut line = v.to_string();
        line.push('\n');
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }
}

/// Told before the file is written, so the app can clean up even after `kill -9`.
pub fn claimed(p: &Path) {
    if p.starts_with(std::env::temp_dir()) {
        return;
    }
    CLAIMS.lock().unwrap_or_else(|e| e.into_inner()).push(p.to_path_buf());
    emit(json!({"event": "claim", "path": p}));
}

pub fn clipboard(text: &str) {
    emit(json!({"event": "clipboard", "text": text}));
}

extern "C" fn on_term(_: i32) {
    if let Some(&fd) = WAKE.get() {
        unsafe { libc::write(fd, b"x".as_ptr() as *const _, 1) };
    }
}

/// Stops the helpers, removes what this unit was writing, reports, and leaves.
fn teardown() -> ! {
    STOPPING.store(true, Ordering::SeqCst);
    unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
        libc::killpg(0, libc::SIGTERM);
    }
    std::thread::sleep(Duration::from_millis(250));
    crate::naming::discard_running();
    remove_empty_claims(&[]);
    emit(json!({"event": "cancelled"}));
    unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_DFL);
        libc::killpg(0, libc::SIGKILL);
        libc::_exit(130)
    }
}

/// A name is reserved by creating an empty file; drop the ones that ended up unused.
fn remove_empty_claims(keep: &[PathBuf]) {
    for p in CLAIMS.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        if !keep.contains(p) && std::fs::metadata(p).map(|m| m.is_file() && m.len() == 0).unwrap_or(false) {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn setup() {
    unsafe {
        // a private copy of stdout; anything else printing to fd 1 lands in stderr
        let fd = libc::dup(1);
        libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        libc::dup2(2, 1);
        *OUT.lock().unwrap() = Some(std::fs::File::from_raw_fd(fd));
        libc::setpgid(0, 0);
        let mut p = [0i32; 2];
        libc::pipe(p.as_mut_ptr());
        libc::fcntl(p[0], libc::F_SETFD, libc::FD_CLOEXEC);
        libc::fcntl(p[1], libc::F_SETFD, libc::FD_CLOEXEC);
        let _ = WAKE.set(p[1]);
        // a real handler, not SIG_IGN: dispositions that are ignored would pass on to ffmpeg
        libc::signal(libc::SIGTERM, on_term as extern "C" fn(i32) as libc::sighandler_t);
        let rd = p[0];
        std::thread::spawn(move || {
            let mut b = [0u8; 1];
            libc::read(rd, b.as_mut_ptr() as *mut _, 1);
            teardown();
        });
    }
}

pub fn serve() -> i32 {
    setup();
    emit(json!({"event": "hello", "pid": std::process::id(), "proto": VERSION}));
    let mut stdin = std::io::BufReader::new(std::io::stdin());
    let mut line = String::new();
    if stdin.read_line(&mut line).unwrap_or(0) == 0 {
        return 2;
    }
    let req: Request = match serde_json::from_str(&line) {
        Ok(r) => r,
        Err(e) => {
            emit(json!({"event": "error", "code": "bad_request", "detail": e.to_string()}));
            return 2;
        }
    };
    if req.v != VERSION {
        emit(json!({"event": "error", "code": "bad_request", "detail": "protocol version"}));
        return 2;
    }
    // the app closing our stdin means it has gone away
    std::thread::spawn(move || {
        let mut sink = [0u8; 64];
        while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
        teardown();
    });
    if let Some(d) = &req.bin_dir {
        engines::tools::set_bin_dir(d.clone());
    }
    crate::settings::init(req.secrets, req.settings);
    match req.verb.as_str() {
        "share_lan" => share_lan(req.paths),
        "share_r2" => share_r2(req.paths),
        "disguise" => disguise(req.paths),
        _ => run_action(req.paths, req.action, req.unattended),
    }
}

fn run_action(paths: Vec<PathBuf>, action: Option<Action>, unattended: bool) -> i32 {
    let Some(action) = action else {
        emit(json!({"event": "error", "code": "bad_request", "detail": "no action"}));
        return 2;
    };
    if paths.is_empty() {
        emit(json!({"event": "error", "code": "bad_request", "detail": "no files"}));
        return 2;
    }
    if unattended && !unattended_ok(&action) {
        emit(json!({"event": "error", "code": "not_allowed_unattended", "detail": ""}));
        return 3;
    }
    crate::naming::begin();
    let last = Mutex::new(Instant::now() - Duration::from_secs(1));
    let progress = |f: f32| {
        let mut l = last.lock().unwrap_or_else(|e| e.into_inner());
        if l.elapsed() >= Duration::from_millis(50) {
            *l = Instant::now();
            emit(json!({"event": "progress", "f": f}));
        }
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engines::run(&paths, &action, &progress)));
    // ffmpeg can be killed a moment before the flag is seen; let the signal thread get there
    if r.as_ref().map(|x| x.is_err()).unwrap_or(true) {
        std::thread::sleep(Duration::from_millis(30));
    }
    while STOPPING.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_secs(1)); // teardown ends the process
    }
    match r {
        Ok(Ok(outputs)) => {
            crate::naming::finish();
            remove_empty_claims(&outputs);
            emit(json!({"event": "done", "outputs": outputs, "notes": engines::take_notes()}));
            0
        }
        Ok(Err(e)) => {
            crate::naming::discard();
            let detail = format!("{e:#}");
            emit(json!({"event": "error", "code": crate::errors::code(&detail), "detail": detail}));
            1
        }
        Err(_) => {
            crate::naming::discard();
            emit(json!({"event": "error", "code": "crashed", "detail": "the work stopped unexpectedly; nothing was changed"}));
            1
        }
    }
}

fn share_lan(paths: Vec<PathBuf>) -> i32 {
    let list = paths.iter().map(|p| p.display().to_string()).collect();
    match tauri::async_runtime::block_on(crate::share::share_lan(list)) {
        Ok(s) => {
            emit(json!({"event": "share", "lan": s}));
            // the server lives on its own threads; the app ends sharing by ending this process
            std::thread::sleep(Duration::from_secs(11 * 60));
            0
        }
        Err(e) => {
            emit(json!({"event": "error", "code": crate::errors::code(&e), "detail": e}));
            1
        }
    }
}

fn share_r2(paths: Vec<PathBuf>) -> i32 {
    let list = paths.iter().map(|p| p.display().to_string()).collect();
    match tauri::async_runtime::block_on(crate::share::share_r2(list)) {
        Ok(links) => {
            emit(json!({"event": "done", "outputs": [], "notes": [], "links": links}));
            0
        }
        Err(e) => {
            emit(json!({"event": "error", "code": crate::errors::code(&e), "detail": e}));
            1
        }
    }
}

fn disguise(paths: Vec<PathBuf>) -> i32 {
    let w: Vec<String> = paths.iter().take(50).filter_map(|p| engines::security::disguise(p)).collect();
    emit(json!({"event": "done", "outputs": [], "notes": w}));
    0
}

/// Every action is classified here on purpose: no catch-all arm, so a new one won't compile
/// until someone decides whether it may run with nobody watching.
pub fn unattended_ok(a: &Action) -> bool {
    use Action::*;
    match a {
        Chain { steps } => steps.iter().all(unattended_ok),
        Convert { .. } | Compress { .. } | Resize { .. } | Rotate { .. } | StripMetadata | ExtractAudio { .. } | Mute | Crop { .. } | AddBg { .. }
        | Trim { .. } | FormatJson { .. } | Zip | Hash | Hashes { .. } | Identify | TargetSize { .. } | Speed { .. } | Flip { .. } | Thumbnail
        | AudioFix { .. } | Ocr { .. } | QrRead | IconSet { .. } | Palette | Watermark { .. } | Vectorize | RemoveBg | BlurFaces | Upscale { .. }
        | PageNumbers | PdfText | ExtractImages | Native { .. } => true,
        Merge | Split | Redact { .. } | Extract | CopyPath | CopyText | Strings | PeInfo | VerifySignature | Sandbox | Encrypt { .. } | Decrypt { .. }
        | Shred | Locks | VirusTotal | Tool { .. } | Base64 { .. } | CopyPathAs { .. } | Diff | Frames { .. } | Slideshow { .. } | Transcribe { .. }
        | QrMake | ContactSheet | Pages { .. } | Stamp { .. } | Protect { .. } | Unprotect { .. } | Custom { .. } | AskClaude { .. } | Pentest { .. } => false,
    }
}
