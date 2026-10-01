// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Generate images through an OpenAI-compatible `/images/generations` endpoint
//! and store the bytes under `.blora/images/<session>/`.

use std::path::{Path, PathBuf};

use blora_types::{BloraError, Result};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug)]
pub struct ImageEndpoint {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

#[derive(Clone, Debug)]
pub struct ImageRequest {
    pub prompt: String,
    pub aspect_ratio: String,
    pub count: u32,
}

#[derive(Clone, Debug)]
pub struct DecodedImage {
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct SavedImage {
    pub relative_path: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
}

impl ImageEndpoint {
    /// Credentials from the process environment. `BLORA_IMAGE_BASE` overrides
    /// the chat base URL. Missing keys are an error, not a fake picture.
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("BLORA_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .map_err(|_| {
                BloraError::provider("set BLORA_API_KEY or OPENAI_API_KEY to generate images")
            })?;
        let api_key = api_key.trim().to_owned();
        if api_key.is_empty() {
            return Err(BloraError::provider(
                "set BLORA_API_KEY or OPENAI_API_KEY to generate images",
            ));
        }
        let base_url = std::env::var("BLORA_IMAGE_BASE")
            .or_else(|_| std::env::var("BLORA_API_BASE"))
            .or_else(|_| std::env::var("OPENAI_BASE_URL"))
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned());
        let model = std::env::var("BLORA_IMAGE_MODEL").unwrap_or_else(|_| "dall-e-3".to_owned());
        Ok(Self {
            base_url: base_url.trim().trim_end_matches('/').to_owned(),
            api_key,
            model: model.trim().to_owned(),
        })
    }
}

impl ImageRequest {
    /// Build a request from the user's message, including lines like `画幅 16:9，张数 2`.
    #[must_use]
    pub fn from_user_text(text: &str) -> Self {
        let aspect_ratio = ["9:16", "16:9", "4:3", "3:4", "1:1"]
            .into_iter()
            .find(|ratio| text.contains(ratio))
            .unwrap_or("1:1");
        let count = text
            .split("张数")
            .nth(1)
            .and_then(|rest| rest.trim().chars().next())
            .and_then(|ch| ch.to_digit(10))
            .filter(|count| (1..=4).contains(count))
            .unwrap_or(1);
        let prompt = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with("画幅") && !line.starts_with("张数"))
            .unwrap_or("image");
        Self {
            prompt: prompt.to_owned(),
            aspect_ratio: aspect_ratio.to_owned(),
            count,
        }
    }

    pub fn parse(prompt: &str, aspect_ratio: Option<&str>, count: Option<u64>) -> Result<Self> {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err(BloraError::Other(
                "generate_image needs a prompt".to_owned(),
            ));
        }
        let aspect_ratio = aspect_ratio.unwrap_or("1:1").trim();
        if size_for(aspect_ratio).is_none() {
            return Err(BloraError::Other(
                "aspect_ratio must be 1:1, 16:9, 9:16, 4:3, or 3:4".to_owned(),
            ));
        }
        let count = count.unwrap_or(1);
        if !(1..=4).contains(&count) {
            return Err(BloraError::Other(
                "generate_image n must be from 1 to 4".to_owned(),
            ));
        }
        Ok(Self {
            prompt: prompt.to_owned(),
            aspect_ratio: aspect_ratio.to_owned(),
            count: u32::try_from(count).unwrap_or(1),
        })
    }
}

#[must_use]
pub fn size_for(aspect_ratio: &str) -> Option<&'static str> {
    match aspect_ratio {
        "1:1" => Some("1024x1024"),
        "16:9" => Some("1792x1024"),
        "9:16" => Some("1024x1792"),
        "4:3" => Some("1024x768"),
        "3:4" => Some("768x1024"),
        _ => None,
    }
}

/// Call the provider and return decoded images. Does not touch the workspace.
pub fn fetch_images(endpoint: &ImageEndpoint, request: &ImageRequest) -> Result<Vec<DecodedImage>> {
    let size = size_for(&request.aspect_ratio).ok_or_else(|| {
        BloraError::Other(format!("unsupported aspect_ratio {}", request.aspect_ratio))
    })?;
    let url = format!("{}/images/generations", endpoint.base_url);
    let body = json!({
        "model": endpoint.model,
        "prompt": request.prompt,
        "n": request.count,
        "size": size,
        "response_format": "b64_json",
    });
    let response = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", endpoint.api_key))
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(map_ureq)?;
    let payload: Value = response
        .into_json()
        .map_err(|err| BloraError::provider(format!("image response was not JSON: {err}")))?;
    decode_openai_images(&payload)
}

pub fn decode_openai_images(payload: &Value) -> Result<Vec<DecodedImage>> {
    let data = payload
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| BloraError::provider("image response has no data array"))?;
    if data.is_empty() {
        return Err(BloraError::provider("image response contained no images"));
    }
    let mut out = Vec::new();
    for item in data {
        if let Some(encoded) = item.get("b64_json").and_then(Value::as_str) {
            let bytes = decode_b64(encoded).map_err(|err| {
                BloraError::provider(format!("image bytes were not valid base64: {err}"))
            })?;
            out.push(DecodedImage {
                mime: sniff_mime(&bytes).to_owned(),
                bytes,
            });
        } else if item.get("url").and_then(Value::as_str).is_some() {
            return Err(BloraError::provider(
                "provider returned an image URL instead of b64_json; set response_format to b64_json",
            ));
        }
    }
    if out.is_empty() {
        return Err(BloraError::provider(
            "image response did not include b64_json data",
        ));
    }
    Ok(out)
}

/// Write images under `.blora/images/<session-id>/`. Names are the content hash.
pub fn write_images(
    workspace: &Path,
    session_id: &str,
    images: &[DecodedImage],
) -> Result<Vec<SavedImage>> {
    let session_id = safe_session_id(session_id)?;
    let dir = workspace.join(".blora").join("images").join(&session_id);
    std::fs::create_dir_all(&dir).map_err(|err| BloraError::Exec(err.to_string()))?;
    let mut saved = Vec::new();
    for image in images {
        let ext = ext_for(&image.mime);
        let name = format!("{}.{}", hex_hash(&image.bytes), ext);
        let path = dir.join(&name);
        std::fs::write(&path, &image.bytes).map_err(|err| BloraError::Exec(err.to_string()))?;
        let (width, height) = dimensions(&image.bytes).unwrap_or((0, 0));
        saved.push(SavedImage {
            relative_path: format!(".blora/images/{session_id}/{name}"),
            mime: image.mime.clone(),
            width,
            height,
        });
    }
    Ok(saved)
}

#[must_use]
pub fn format_saved(images: &[SavedImage]) -> String {
    let mut lines = vec![format!("saved {} image(s)", images.len())];
    for image in images {
        lines.push(format!(
            "{} {} {}x{}",
            image.relative_path, image.mime, image.width, image.height
        ));
    }
    lines.join("\n")
}

/// Workspace path the permission check should see before any network call.
#[must_use]
pub fn permission_path(session_id: &str) -> PathBuf {
    PathBuf::from(format!(".blora/images/{session_id}/pending.png"))
}

fn safe_session_id(session_id: &str) -> Result<String> {
    let ok = !session_id.is_empty()
        && session_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if ok {
        Ok(session_id.to_owned())
    } else {
        Err(BloraError::Other(
            "generate_image needs a session id".to_owned(),
        ))
    }
}

fn ext_for(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => "png",
    }
}

fn hex_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sniff_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "image/png"
    }
}

fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() >= 24 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") && &bytes[12..16] == b"IHDR" {
        let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
        let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
        return Some((width, height));
    }
    None
}

fn map_ureq(err: ureq::Error) -> BloraError {
    match err {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            let body: String = body.chars().take(400).collect();
            BloraError::provider(format!("image API HTTP {code}: {}", body.trim()))
        }
        ureq::Error::Transport(transport) => {
            BloraError::provider(format!("image API transport: {transport}"))
        }
    }
}

fn decode_b64(input: &str) -> std::result::Result<Vec<u8>, &'static str> {
    fn value(byte: u8) -> std::result::Result<u8, &'static str> {
        match byte {
            b'A'..=b'Z' => Ok(byte - b'A'),
            b'a'..=b'z' => Ok(byte - b'a' + 26),
            b'0'..=b'9' => Ok(byte - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err("invalid base64"),
        }
    }
    let cleaned: Vec<u8> = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if cleaned.len() % 4 != 0 {
        return Err("invalid base64 length");
    }
    let mut out = Vec::new();
    for chunk in cleaned.chunks(4) {
        let pad = chunk.iter().filter(|byte| **byte == b'=').count();
        if pad > 2 || chunk[..4 - pad].contains(&b'=') {
            return Err("invalid base64 padding");
        }
        let mut bits = [0u8; 4];
        for (index, byte) in chunk.iter().take(4 - pad).enumerate() {
            bits[index] = value(*byte)?;
        }
        out.push((bits[0] << 2) | (bits[1] >> 4));
        if pad < 2 {
            out.push((bits[1] << 4) | (bits[2] >> 2));
        }
        if pad < 1 {
            out.push((bits[2] << 6) | bits[3]);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90wS\xde";

    #[test]
    fn decodes_b64_png_and_writes_hashed_file() {
        let payload = json!({
            "data": [{ "b64_json": base64_of_png() }]
        });
        let images = decode_openai_images(&payload).unwrap();
        assert_eq!(images[0].mime, "image/png");
        let dir = std::env::temp_dir().join(format!("blora-imagine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let saved = write_images(&dir, "sess-1", &images).unwrap();
        assert!(saved[0].relative_path.starts_with(".blora/images/sess-1/"));
        assert!(saved[0].relative_path.ends_with(".png"));
        assert_eq!((saved[0].width, saved[0].height), (1, 1));
        let bytes = std::fs::read(dir.join(&saved[0].relative_path)).unwrap();
        assert_eq!(bytes, PNG);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_url_only_payload_and_bad_ratio() {
        let payload = json!({ "data": [{ "url": "https://example.test/a.png" }] });
        assert!(decode_openai_images(&payload).is_err());
        assert!(ImageRequest::parse("a cat", Some("2:1"), Some(1)).is_err());
        assert!(ImageRequest::parse("a cat", Some("1:1"), Some(5)).is_err());
        assert_eq!(size_for("16:9"), Some("1792x1024"));
    }

    #[test]
    fn reads_aspect_and_count_from_the_user_message() {
        let request = ImageRequest::from_user_text("一张珊瑚壁纸\n画幅 16:9，张数 2");
        assert_eq!(request.prompt, "一张珊瑚壁纸");
        assert_eq!(request.aspect_ratio, "16:9");
        assert_eq!(request.count, 2);
    }

    fn base64_of_png() -> String {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        let mut index = 0;
        while index < PNG.len() {
            let b0 = PNG[index];
            let b1 = PNG.get(index + 1).copied();
            let b2 = PNG.get(index + 2).copied();
            let triple = [
                b0 >> 2,
                ((b0 & 0x03) << 4) | (b1.unwrap_or(0) >> 4),
                ((b1.unwrap_or(0) & 0x0f) << 2) | (b2.unwrap_or(0) >> 6),
                b2.unwrap_or(0) & 0x3f,
            ];
            for (offset, value) in triple.iter().enumerate() {
                if (offset == 2 && b1.is_none()) || (offset == 3 && b2.is_none()) {
                    out.push('=');
                } else {
                    out.push(TABLE[*value as usize] as char);
                }
            }
            index += 3;
        }
        out
    }
}
