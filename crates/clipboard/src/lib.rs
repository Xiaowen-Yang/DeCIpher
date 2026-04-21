//! Clipboard image paste support.
//!
//! Uses the `arboard` crate to read images from the system clipboard,
//! stages them as PNG files under `/tmp/decipher-clipboard`, and returns
//! lightweight image references for inclusion in the JSON protocol message.

use arboard::Clipboard;
use image::ImageEncoder;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use decipher_protocol::ImageData;

/// Try to read an image from the clipboard.
/// Returns `Some(ImageData)` with a staged PNG path if an image is found,
/// or `None` if the clipboard doesn't contain an image.
pub fn paste_image() -> Option<ImageData> {
    let mut clipboard = Clipboard::new().ok()?;
    let img = clipboard.get_image().ok()?;

    stage_png_image(img.bytes.as_ref(), img.width as u32, img.height as u32).ok()
}

fn stage_png_image(rgba: &[u8], width: u32, height: u32) -> std::io::Result<ImageData> {
    // Convert RGBA pixels into PNG bytes.
    let mut png_bytes = Vec::new();
    let cursor = Cursor::new(&mut png_bytes);
    let encoder = image::codecs::png::PngEncoder::new(cursor);
    encoder
        .write_image(
            rgba,
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(std::io::Error::other)?;

    let path = next_staged_image_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, png_bytes)?;

    Ok(ImageData {
        data: String::new(),
        path: Some(path.to_string_lossy().to_string()),
        mime: "image/png".to_string(),
    })
}

fn next_staged_image_path() -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    PathBuf::from(format!("/tmp/decipher-clipboard/paste-{pid}-{now}.png"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_png_image_writes_temp_file_and_returns_path() {
        let img = stage_png_image(&[255, 0, 0, 255], 1, 1).expect("staged image");
        let path = img.path.expect("image path");
        assert!(path.starts_with("/tmp/decipher-clipboard/"));
        assert!(std::path::Path::new(&path).exists());
        assert!(img.data.is_empty());
        let _ = std::fs::remove_file(path);
    }
}
