//! Clipboard image paste support.
//!
//! Uses the `arboard` crate to read images from the system clipboard,
//! encodes them as PNG base64, and returns them for inclusion in the
//! JSON protocol message to the Node.js agent.

use arboard::Clipboard;
use base64::Engine;
use image::ImageEncoder;
use std::io::Cursor;

use crate::protocol::ImageData;

/// Try to read an image from the clipboard.
/// Returns `Some(ImageData)` with base64-encoded PNG if an image is found,
/// or `None` if the clipboard doesn't contain an image.
pub fn paste_image() -> Option<ImageData> {
    let mut clipboard = Clipboard::new().ok()?;
    let img = clipboard.get_image().ok()?;

    // Convert arboard ImageData to PNG bytes
    let mut png_bytes = Vec::new();
    let cursor = Cursor::new(&mut png_bytes);
    let encoder = image::codecs::png::PngEncoder::new(cursor);
    encoder
        .write_image(
            img.bytes.as_ref(),
            img.width as u32,
            img.height as u32,
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);

    Some(ImageData {
        data: b64,
        mime: "image/png".to_string(),
    })
}
