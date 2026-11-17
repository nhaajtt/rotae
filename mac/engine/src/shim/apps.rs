pub fn launch_id(_id: &str, _args: &str) -> Result<(), String> {
    Err("not available here".into())
}

#[cfg(test)]
pub struct App {
    pub id: &'static str,
    pub target: std::path::PathBuf,
}

/// Mac app entries arrive with the Open In feature.
#[cfg(test)]
pub fn all() -> &'static [App] {
    &[]
}
