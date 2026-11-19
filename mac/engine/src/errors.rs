//! Engine messages are plain English; the app needs stable codes to show a localized fix.
const RULES: &[(&str, &str)] = &[
    // nothing was wrong with the file: there was simply nothing to do
    ("already has text", "already_text"),
    ("nothing private found", "nothing_private"),
    ("on-device model is not ready", "need_model"),
    ("on-device model needs", "need_model"),
    ("does not support this language", "model_language"),
    ("model declined", "model_declined"),
    ("there was no question", "no_question"),
    ("no text found", "no_text"),
    ("no faces found", "no_faces"),
    ("already as small", "already_small"),
    ("already small", "already_small"),
    ("too small for app icons", "too_small"),
    ("drop a folder", "need_folder"),
    ("not an SVG", "wrong_kind"),
    ("can't read this SVG", "bad_document"),
    ("can't read this package", "bad_document"),
    ("this works on Markdown", "wrong_kind"),
    ("is not a document this can compare", "wrong_kind"),
    ("choose exactly two documents", "need_two_docs"),
    ("have the same text", "same_text"),
    ("only one page", "one_page"),
    ("no pictures found", "no_pictures"),
    ("no columns of numbers", "no_numbers"),
    ("checksum does not match", "checksum_mismatch"),
    ("no checksum on the clipboard", "no_checksum"),
    ("this works on .csv", "wrong_kind"),
    ("this works on .xml", "wrong_kind"),
    ("this works on photos", "wrong_kind"),
    ("this works on .pdf bank", "wrong_kind"),
    ("nothing to tidy", "already_clean"),
    ("choose exactly two tables", "need_two_tables"),
    ("no amount column", "no_numbers"),
    ("isn't a Vietnamese e-invoice", "not_einvoice"),
    ("choose one .docx template", "need_template"),
    ("no placeholders found", "no_placeholders"),
    ("none of the placeholders", "no_placeholders"),
    ("open in another app", "file_open"),
    ("no dated lines", "not_statement"),
    ("no column named", "no_column"),
    ("this works on .", "wrong_document"),
    ("can't read this document", "bad_document"),
    ("too big for an Excel sheet", "too_big"),
    ("FFmpeg add-on", "need_ffmpeg"),
    ("FFmpeg failed", "ffmpeg_failed"),
    ("stopped responding", "timed_out"),
    ("longer than", "timed_out"),
    ("wrong password", "wrong_password"),
    ("password", "password_problem"),
    ("not installed", "need_addon"),
    ("Add-ons", "need_addon"),
    ("LibreOffice", "need_libreoffice"),
    ("Claude API key", "need_claude_key"),
    ("R2", "need_r2"),
    ("private local network", "not_private_network"),
    ("animated", "animated_unsupported"),
    ("too big", "too_big"),
    ("too large", "too_big"),
    ("too long", "too_big"),
    ("can't open PDF", "bad_pdf"),
    ("can't read this image", "bad_image"),
    ("can't extract", "bad_archive"),
    ("not a text file", "not_text"),
    ("not valid JSON", "bad_data"),
    ("hard links", "shred_links"),
    ("is a link or folder", "shred_links"),
    ("Operation not permitted", "no_permission"),
    ("Permission denied", "no_permission"),
    ("No space left", "disk_full"),
    ("unsafe in a command line", "unsafe_name"),
];

pub fn code(message: &str) -> &'static str {
    RULES.iter().find(|(m, _)| message.contains(m)).map(|(_, c)| *c).unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    #[test]
    fn maps_and_falls_back() {
        assert_eq!(super::code("Video and audio need the FFmpeg add-on: Settings › Add-ons"), "need_ffmpeg");
        assert_eq!(super::code("something new"), "unknown");
        assert_eq!(super::code("this PDF is already as small as it gets"), "already_small");
        assert_eq!(super::code("no faces found in this picture"), "no_faces");
    }
}
