use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Default)]
pub struct Settings {
    pub vt_key: String,
    pub yara_rules: String,
    pub anthropic_key: String,
    pub r2_key: String,
    pub r2_secret: String,
    pub r2_account: String,
    pub r2_bucket: String,
    pub r2_public: String,
    /// Pentest user mode is Linux-only (it needs bubblewrap), so it stays off here
    pub pentest: bool,
    pub pentest_consented_at: u64,
}

static CURRENT: OnceLock<Settings> = OnceLock::new();

/// Handed over on stdin for this one unit of work; never read from disk or the environment.
pub fn init(secrets: HashMap<String, String>, plain: HashMap<String, String>) {
    let s = |k: &str| secrets.get(k).cloned().unwrap_or_default();
    let p = |k: &str| plain.get(k).cloned().unwrap_or_default();
    let _ = CURRENT.set(Settings {
        vt_key: s("vt_key"),
        anthropic_key: s("anthropic_key"),
        r2_key: s("r2_key"),
        r2_secret: s("r2_secret"),
        yara_rules: p("yara_rules"),
        r2_account: p("r2_account"),
        r2_bucket: p("r2_bucket"),
        r2_public: p("r2_public"),
        ..Default::default()
    });
}

pub fn current() -> Settings {
    CURRENT.get().cloned().unwrap_or_default()
}
