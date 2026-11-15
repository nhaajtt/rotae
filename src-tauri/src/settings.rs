use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::Mutex};
use tauri::{AppHandle, Emitter, Manager};

pub const ACCENTS: [&str; 7] = ["gold", "noir", "oxblood", "navy", "racing", "violet", "graphite"];
pub const THEMES: [&str; 3] = ["system", "light", "dark"];

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    /// Only react to drags that start in File Explorer or on the desktop.
    pub explorer_only: bool,
    pub accent: String,
    pub theme: String,
    /// system | en | vi
    pub lang: String,
    /// VirusTotal API key; empty = only open the public report page
    pub vt_key: String,
    pub watches: Vec<Watch>,
    /// Ctrl+Space previews the file selected in Explorer
    pub peek: bool,
    pub anthropic_key: String,
    pub r2_account: String,
    pub r2_key: String,
    pub r2_secret: String,
    pub r2_bucket: String,
    /// public base URL of the bucket (r2.dev or a custom domain)
    pub r2_public: String,
    /// folder of .yar rules for the YARA scan
    pub yara_rules: String,
    /// Pentest user mode: security tools on the wheel (sandboxed)
    pub pentest: bool,
    /// when the authorized-use notice was accepted (seconds since 1970; 0 = never)
    pub pentest_consented_at: u64,
    /// Linux: where the drop strip waits: right | left | top | off
    pub zone_edge: String,
    /// solid panels instead of glass
    pub reduce_transparency: bool,
}

/// A folder whose new files get a recipe automatically.
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Watch {
    pub folder: String,
    /// only these extensions (empty = all)
    pub exts: Vec<String>,
    pub title: String,
    /// an engines::Action as JSON
    pub action: serde_json::Value,
}

impl Default for Settings {
    fn default() -> Self {
        Self { enabled: true, explorer_only: true, accent: "violet".into(), theme: "system".into(), vt_key: String::new(), watches: Vec::new(), peek: true, ..Self::blank() }
    }
}

impl Settings {
    fn blank() -> Self {
        Self {
            enabled: true,
            explorer_only: true,
            accent: String::new(),
            theme: String::new(),
            lang: "system".into(),
            vt_key: String::new(),
            watches: Vec::new(),
            peek: true,
            anthropic_key: String::new(),
            r2_account: String::new(),
            r2_key: String::new(),
            r2_secret: String::new(),
            r2_bucket: String::new(),
            r2_public: String::new(),
            yara_rules: String::new(),
            pentest: false,
            pentest_consented_at: 0,
            zone_edge: "right".into(),
            reduce_transparency: false,
        }
    }

    /// The API keys and secrets, so they can be sealed on disk.
    fn secrets(&mut self) -> [&mut String; 4] {
        [&mut self.vt_key, &mut self.anthropic_key, &mut self.r2_key, &mut self.r2_secret]
    }
}

/// Secrets are stored encrypted with Windows DPAPI: only this Windows user can read them.
#[cfg(windows)]
mod seal {
    use base64::Engine;
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB},
    };

    fn run(data: &[u8], protect: bool) -> Option<Vec<u8>> {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe {
            if protect {
                CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out)
            } else {
                CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out)
            }
        };
        if ok == 0 {
            return None;
        }
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
        unsafe { LocalFree(out.pbData as _) };
        Some(v)
    }

    /// A value that is still sealed (could not be opened) is written back as it was.
    /// None when Windows couldn't seal it; the key is then never written in the clear.
    pub fn lock(s: &str) -> Option<String> {
        if s.is_empty() || s.starts_with("dpapi:") {
            return Some(s.to_string());
        }
        run(s.as_bytes(), true).map(|v| format!("dpapi:{}", base64::engine::general_purpose::STANDARD.encode(v)))
    }

    /// None when the value was sealed by another Windows account or PC.
    pub fn unlock(s: &str) -> Option<String> {
        let Some(b64) = s.strip_prefix("dpapi:") else { return Some(s.to_string()) };
        base64::engine::general_purpose::STANDARD.decode(b64).ok().and_then(|v| run(&v, false)).and_then(|v| String::from_utf8(v).ok())
    }
}

/// Linux: the desktop keyring (Secret Service, through secret-tool) when there is one; otherwise
/// the value stays in settings.json, which is written readable by this user only.
#[cfg(not(windows))]
mod seal {
    use std::{
        collections::HashMap,
        io::Write,
        process::{Command, Stdio},
        sync::Mutex,
        time::{Duration, Instant},
    };

    /// value → what was stored for it, so each save doesn't ask the keyring again
    static DONE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

    fn id(s: &str) -> String {
        use sha2::Digest;
        sha2::Sha256::digest(s.as_bytes()).iter().take(12).map(|b| format!("{b:02x}")).collect()
    }

    /// Runs secret-tool, giving up after a few seconds (a locked keyring can wait for a prompt).
    fn secret_tool(args: &[&str], input: Option<&str>) -> Option<Vec<u8>> {
        let mut child = Command::new("secret-tool")
            .args(args)
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
            let _ = stdin.write_all(text.as_bytes());
        }
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            match child.try_wait() {
                Ok(Some(st)) if st.success() => {
                    let mut out = Vec::new();
                    use std::io::Read;
                    child.stdout.take()?.read_to_end(&mut out).ok()?;
                    return Some(out);
                }
                Ok(Some(_)) | Err(_) => return None,
                Ok(None) if Instant::now() > until => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    pub fn lock(s: &str) -> Option<String> {
        if s.is_empty() || s.starts_with("secret:") {
            return Some(s.to_string());
        }
        let mut done = DONE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = done.as_ref().and_then(|m| m.get(s)) {
            return Some(v.clone());
        }
        let id = id(s);
        let stored = secret_tool(&["store", "--label", "Rotae key", "application", "rotae", "key", &id], Some(s)).is_some();
        let v = if stored { format!("secret:{id}") } else { s.to_string() };
        done.get_or_insert_with(HashMap::new).insert(s.to_string(), v.clone());
        Some(v)
    }

    /// None when the keyring doesn't have it (another account or PC, or the keyring is locked).
    pub fn unlock(s: &str) -> Option<String> {
        let Some(id) = s.strip_prefix("secret:") else { return Some(s.to_string()) };
        let out = secret_tool(&["lookup", "application", "rotae", "key", id], None)?;
        let v = String::from_utf8(out).ok().filter(|v| !v.is_empty())?;
        DONE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(v.clone(), s.to_string());
        Some(v)
    }
}

/// Writes a file only this user can read (Linux: mode 0600; Windows keeps the per-user folder's rights).
fn write_private(p: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(p)?;
        f.write_all(data)?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(p, fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    fs::write(p, data)
}

/// What the windows see instead of a saved key; sending it back leaves the key unchanged.
const MASK: &str = "••••••••";

/// A copy for the web views: keys are never handed to them, only whether one is set.
fn masked(s: &Settings) -> Settings {
    let mut s = s.clone();
    for v in s.secrets() {
        if !v.is_empty() {
            *v = MASK.into();
        }
    }
    s
}

/// The keys in memory. One that could not be opened stays sealed (so it isn't wiped on the next save)
/// and is treated as missing.
fn usable(v: &str) -> &str {
    if v.starts_with("dpapi:") || v.starts_with("secret:") {
        ""
    } else {
        v
    }
}

pub struct Store(pub Mutex<Settings>);

fn file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("settings.json"))
}

/// True before the settings file has ever been written (first launch).
pub fn first_run(app: &AppHandle) -> bool {
    file(app).is_some_and(|p| !p.exists())
}

/// The settings file exists but couldn't be read (held by another program at start): saving
/// would replace the user's settings with defaults, so it is left alone this session.
static KEEP_FILE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn load(app: &AppHandle) -> Settings {
    let Some(p) = file(app) else { return Settings::default() };
    let mut read = fs::read(&p);
    for _ in 0..10 {
        if read.is_ok() || !p.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        read = fs::read(&p);
    }
    let text = match read {
        // not UTF-8 counts as damaged, like bad JSON below
        Ok(t) => String::from_utf8_lossy(&t).into_owned(),
        Err(_) if !p.exists() => return Settings::default(),
        Err(e) => {
            KEEP_FILE.store(true, std::sync::atomic::Ordering::Relaxed);
            crate::log(&format!("settings: couldn't read the file ({e}); using defaults without overwriting it"));
            return Settings::default();
        }
    };
    match serde_json::from_str::<Settings>(&text) {
        Ok(mut s) => {
            for v in s.secrets() {
                match seal::unlock(v) {
                    Some(open) => *v = open,
                    None => crate::log("settings: a saved key could not be opened on this account; it needs to be entered again"),
                }
            }
            s
        }
        Err(e) => {
            // keep the broken file for the user instead of silently overwriting it with defaults
            let _ = fs::copy(&p, p.with_extension("json.bad"));
            crate::log(&format!("settings: unreadable ({e}); kept a copy as settings.json.bad and started from defaults"));
            Settings::default()
        }
    }
}

static CURRENT: Mutex<Option<Settings>> = Mutex::new(None);

/// Settings for code without an AppHandle (engines).
pub fn current() -> Settings {
    let mut s = CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
    for v in s.secrets() {
        *v = usable(v).to_string();
    }
    s
}

pub fn get(app: &AppHandle) -> Settings {
    app.state::<Store>().0.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn update(app: &AppHandle, f: impl FnOnce(&mut Settings)) {
    let _ = try_update(app, |s| {
        f(s);
        Ok(())
    });
}

/// Changes the settings and saves them while holding the lock, so two changes can't be saved out of order.
fn try_update(app: &AppHandle, f: impl FnOnce(&mut Settings) -> Result<(), String>) -> Result<Settings, String> {
    let store = app.state::<Store>();
    let mut s = store.0.lock().unwrap_or_else(|e| e.into_inner());
    let mut next = s.clone();
    f(&mut next)?;
    // a change that can't be saved is refused, so the window never shows something that is lost on restart
    if KEEP_FILE.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("the settings file was busy when Rotae started; restart Rotae to change settings".into());
    }
    let mut sealed = next.clone();
    for v in sealed.secrets() {
        // writing would either expose the key or erase it
        *v = seal::lock(v).ok_or("Windows couldn't protect the key, so it wasn't saved")?;
    }
    *s = next;
    *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
    #[cfg(windows)]
    crate::win::set_peek(s.enabled && s.peek);
    #[cfg(not(windows))]
    {
        // after the lock is released: parking reads the settings
        let app = app.clone();
        std::thread::spawn(move || crate::windows::zone::park(&app));
    }
    if let Some(p) = file(app) {
        if let Some(dir) = p.parent() {
            let _ = fs::create_dir_all(dir);
        }
        // write a new file and swap it in, so a crash mid-write can't leave half a file
        let tmp = p.with_extension("json.tmp");
        let saved = write_private(&tmp, serde_json::to_string_pretty(&sealed).unwrap_or_default().as_bytes()).and_then(|_| fs::rename(&tmp, &p));
        if let Err(e) = saved {
            crate::log(&format!("settings: could not save: {e}"));
        }
    }
    let out = masked(&s);
    drop(s);
    let _ = app.emit("settings", &out);
    Ok(out)
}

#[tauri::command]
pub async fn get_settings(app: AppHandle) -> Settings {
    masked(&get(&app))
}

/// Applies a partial update from the settings view, e.g. `{"theme": "dark"}`.
#[tauri::command]
pub async fn update_settings(app: AppHandle, patch: serde_json::Value) -> Result<Settings, String> {
    try_update(&app, |s| {
        let keep: Vec<String> = s.secrets().iter().map(|v| v.to_string()).collect();
        let mut v = serde_json::to_value(&*s).map_err(|e| e.to_string())?;
        if let (Some(o), Some(p)) = (v.as_object_mut(), patch.as_object()) {
            for (k, val) in p {
                o.insert(k.clone(), val.clone());
            }
        }
        *s = serde_json::from_value(v).map_err(|e| e.to_string())?;
        // the mask coming back means "unchanged"
        for (v, old) in s.secrets().into_iter().zip(keep) {
            if v.as_str() == MASK {
                *v = old;
            }
        }
        Ok(())
    })
}

#[tauri::command]
pub fn get_autostart(app: AppHandle) -> bool {
    crate::platform::autostart_enabled(&app)
}

#[tauri::command]
pub async fn set_autostart(app: AppHandle, on: bool) -> Result<(), String> {
    crate::platform::set_autostart(&app, on)
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn secrets_round_trip() {
        let locked = super::seal::lock("sk-test-123").unwrap();
        assert!(locked.starts_with("dpapi:") && !locked.contains("sk-test"));
        assert_eq!(super::seal::unlock(&locked).as_deref(), Some("sk-test-123"));
        assert_eq!(super::seal::unlock("plain").as_deref(), Some("plain"));
        // a key sealed elsewhere is kept as it was, not wiped
        assert_eq!(super::seal::unlock("dpapi:AAAA"), None);
        assert_eq!(super::seal::lock("dpapi:AAAA").as_deref(), Some("dpapi:AAAA"));
    }
}
