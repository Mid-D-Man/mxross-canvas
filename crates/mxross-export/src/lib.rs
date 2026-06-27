// crates/mxross-export/src/lib.rs
//! Encodes raw pixel buffers into output file formats. Currently just
//! PNG (the obvious first format, and the only one that needs to exist
//! for "export a transparent image" to work end to end) — MidManStudio's
//! own MPX raster format is a natural future addition here, alongside
//! PNG rather than instead of it, once there's a reason to reach for it
//! specifically (smaller files, round-tripping through other MidManStudio
//! tools, etc.).
//!
//! Deliberately knows nothing about wgpu, Android, or any other
//! platform/rendering concern — it only ever sees plain bytes plus
//! width/height. Whatever GPU-side readback produced those bytes
//! (`mxross-render-gpu`'s `PaintCanvas::read_pixels`, currently the only
//! caller) is a separate concern this crate has no dependency on.

/// Encodes `rgba` (tightly packed, 4 bytes per pixel, row-major,
/// `width * height * 4` bytes total) as a PNG with a full alpha channel.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let expected_len = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected_len {
        return Err(format!(
            "encode_png: expected {expected_len} bytes for a {width}x{height} RGBA8 image, got {}",
            rgba.len()
        ));
    }

    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("PNG header write failed: {e}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| format!("PNG data write failed: {e}"))?;
    }
    Ok(bytes)
      }
