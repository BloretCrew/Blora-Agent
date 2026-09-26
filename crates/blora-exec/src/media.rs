// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Text descriptions of files that are not valid UTF-8.
//!
//! Tool results stay text. Image bytes are identified and measured here instead
//! of being lossily decoded into the transcript.

/// A short, single-paragraph description of `bytes` for a model.
#[must_use]
pub fn describe_bytes(label: &str, bytes: &[u8]) -> String {
    if let Some((width, height)) = png_size(bytes) {
        return format!(
            "image/png {width}x{height}, {} bytes ({label}). Pixel bytes are not inlined.",
            bytes.len()
        );
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return format!(
            "image/png, {} bytes ({label}). Dimensions were not in the header; pixel bytes are not inlined.",
            bytes.len()
        );
    }
    if let Some((width, height)) = gif_size(bytes) {
        return format!(
            "image/gif {width}x{height}, {} bytes ({label}). Pixel bytes are not inlined.",
            bytes.len()
        );
    }
    if let Some((width, height)) = jpeg_size(bytes) {
        return format!(
            "image/jpeg {width}x{height}, {} bytes ({label}). Pixel bytes are not inlined.",
            bytes.len()
        );
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return format!(
            "image/jpeg, {} bytes ({label}). Dimensions were not found; pixel bytes are not inlined.",
            bytes.len()
        );
    }
    if let Some((width, height)) = webp_size(bytes) {
        return format!(
            "image/webp {width}x{height}, {} bytes ({label}). Pixel bytes are not inlined.",
            bytes.len()
        );
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return format!(
            "image/webp, {} bytes ({label}). Dimensions were not found; pixel bytes are not inlined.",
            bytes.len()
        );
    }
    format!(
        "binary file, {} bytes ({label}). It is not valid UTF-8, so its bytes are not inlined.",
        bytes.len()
    )
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

fn gif_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 10 || !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return None;
    }
    let width = u16::from_le_bytes(bytes[6..8].try_into().ok()?) as u32;
    let height = u16::from_le_bytes(bytes[8..10].try_into().ok()?) as u32;
    Some((width, height))
}

fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut index = 2usize;
    while index + 9 < bytes.len() {
        if bytes[index] != 0xFF {
            index += 1;
            continue;
        }
        let marker = bytes[index + 1];
        if marker == 0xD8 || marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            index += 2;
            continue;
        }
        if index + 4 > bytes.len() {
            return None;
        }
        let segment = u16::from_be_bytes(bytes[index + 2..index + 4].try_into().ok()?) as usize;
        if segment < 2 || index + 2 + segment > bytes.len() {
            return None;
        }
        // Start-of-frame markers that carry dimensions.
        let sof = matches!(
            marker,
            0xC0 | 0xC1
                | 0xC2
                | 0xC3
                | 0xC5
                | 0xC6
                | 0xC7
                | 0xC9
                | 0xCA
                | 0xCB
                | 0xCD
                | 0xCE
                | 0xCF
        );
        if sof && segment >= 7 {
            let height = u16::from_be_bytes(bytes[index + 5..index + 7].try_into().ok()?) as u32;
            let width = u16::from_be_bytes(bytes[index + 7..index + 9].try_into().ok()?) as u32;
            return Some((width, height));
        }
        index += 2 + segment;
    }
    None
}

fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 30 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    if &bytes[12..16] == b"VP8X" {
        let width = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
        let height = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
        return Some((width, height));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_png_dimensions() {
        let mut bytes = vec![0u8; 24];
        bytes[0..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes[12..16].copy_from_slice(b"IHDR");
        bytes[16..20].copy_from_slice(&32u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&16u32.to_be_bytes());
        let text = describe_bytes("icon.png", &bytes);
        assert!(text.contains("image/png 32x16"));
        assert!(!text.contains('\u{fffd}'));
    }
}
