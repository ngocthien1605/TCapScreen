//! Thin wrapper over `arboard` for putting a captured image (or a path string) on the
//! Windows clipboard. `arboard` is pinned with `default-features = false` in the workspace, so
//! the `image-data` feature is re-enabled on this crate's dependency to get `set_image`.

use anyhow::{Context, Result};
use arboard::{Clipboard, ImageData};
use imaging::Rgba;

/// Copy an RGBA image to the clipboard as a bitmap.
pub fn copy_image(img: &Rgba) -> Result<()> {
    let mut clipboard = Clipboard::new().context("opening clipboard")?;
    let data = ImageData {
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: img.as_raw().as_slice().into(),
    };
    clipboard
        .set_image(data)
        .context("setting clipboard image")?;
    Ok(())
}

/// Copy text (e.g. a saved file's path) to the clipboard.
pub fn copy_text(text: &str) -> Result<()> {
    let mut clipboard = Clipboard::new().context("opening clipboard")?;
    clipboard
        .set_text(text.to_owned())
        .context("setting clipboard text")?;
    Ok(())
}
