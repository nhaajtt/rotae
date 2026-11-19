use anyhow::Result;

/// The app owns the pasteboard; this process only says what goes on it.
pub fn set(text: &str) -> Result<()> {
    crate::proto::clipboard(&text.replace("\r\n", "\n"));
    Ok(())
}
