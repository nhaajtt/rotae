//! The Windows codecs' counterparts, backed by ImageIO and Vision.
use anyhow::Result;
use std::path::Path;

pub fn decode_to_png(path: &Path) -> Result<Vec<u8>> {
    crate::native::decode(path)
}

pub fn ocr(path: &Path) -> Result<String> {
    crate::native::ocr(path)
}

pub fn ocr_max_side() -> u32 {
    8192
}
