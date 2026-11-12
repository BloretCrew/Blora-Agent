// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Image editing, video generation, speech, and vision analysis through
//! OpenAI-compatible HTTP endpoints.

use std::io::Read;
use std::time::Duration;

use base64::Engine as _;
use blora_types::{BloraError, Result};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct MediaEndpoint {
    pub base: String,
    pub key: String,
    pub image_model: String,
    pub vision_model: String,
    pub video_model: String,
    pub tts_model: String,
    pub tts_voice: String,
}

#[derive(Clone, Debug)]
pub struct MediaFile {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub extension: &'static str,
}

#[derive(Clone, Debug)]
pub enum VideoOutput {
    File(MediaFile),
    Job(String),
}

impl MediaEndpoint {
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let key = std::env::var("BLORA_MEDIA_KEY")
            .or_else(|_| std::env::var("BLORA_API_KEY"))
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .ok()?;
        let base = std::env::var("BLORA_MEDIA_BASE")
            .or_else(|_| std::env::var("BLORA_IMAGE_BASE"))
            .or_else(|_| std::env::var("BLORA_API_BASE"))
            .or_else(|_| std::env::var("OPENAI_BASE_URL"))
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned());
        Some(Self {
            base: base.trim_end_matches('/').to_owned(),
            key,
            image_model: std::env::var("BLORA_IMAGE_MODEL")
                .unwrap_or_else(|_| "gpt-image-1".to_owned()),
            vision_model: std::env::var("BLORA_VISION_MODEL")
                .unwrap_or_else(|_| "gpt-4o".to_owned()),
            video_model: std::env::var("BLORA_VIDEO_MODEL").unwrap_or_else(|_| "sora-2".to_owned()),
            tts_model: std::env::var("BLORA_TTS_MODEL").unwrap_or_else(|_| "tts-1".to_owned()),
            tts_voice: std::env::var("BLORA_TTS_VOICE").unwrap_or_else(|_| "alloy".to_owned()),
        })
    }
}

pub fn edit_image(
    endpoint: &MediaEndpoint,
    image: &[u8],
    media_type: &str,
    mask: Option<(&[u8], &str)>,
    prompt: &str,
    size: Option<&str>,
) -> Result<MediaFile> {
    let boundary = format!(
        "blora-{}",
        blora_types::EventId::generate().as_str().replace('_', "")
    );
    let mut body = Vec::new();
    field(&mut body, &boundary, "model", &endpoint.image_model);
    field(&mut body, &boundary, "prompt", prompt);
    if let Some(size) = size {
        field(&mut body, &boundary, "size", size);
    }
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"image\"; filename=\"image.{}\"\r\n",
            extension_for(media_type)
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {media_type}\r\n\r\n").as_bytes());
    body.extend_from_slice(image);
    body.extend_from_slice(b"\r\n");
    if let Some((mask, mask_type)) = mask {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"mask\"; filename=\"mask.{}\"\r\n",
                extension_for(mask_type)
            )
            .as_bytes(),
        );
        body.extend_from_slice(format!("Content-Type: {mask_type}\r\n\r\n").as_bytes());
        body.extend_from_slice(mask);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    let response = ureq::post(&format!("{}/images/edits", endpoint.base))
        .set("Authorization", &format!("Bearer {}", endpoint.key))
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .timeout(Duration::from_secs(120))
        .send_bytes(&body)
        .map_err(map_ureq)?;
    let value: Value = response
        .into_json()
        .map_err(|err| BloraError::Exec(err.to_string()))?;
    first_image(&value)
}

pub fn generate_video(
    endpoint: &MediaEndpoint,
    prompt: &str,
    image: Option<&[u8]>,
    duration_seconds: Option<u32>,
    aspect_ratio: Option<&str>,
) -> Result<VideoOutput> {
    let mut body = json!({"model": endpoint.video_model, "prompt": prompt});
    if let Some(duration) = duration_seconds {
        body["duration_seconds"] = json!(duration);
    }
    if let Some(ratio) = aspect_ratio {
        body["aspect_ratio"] = json!(ratio);
    }
    if let Some(image) = image {
        body["image"] = json!(base64::engine::general_purpose::STANDARD.encode(image));
    }
    let response = ureq::post(&format!("{}/videos/generations", endpoint.base))
        .set("Authorization", &format!("Bearer {}", endpoint.key))
        .timeout(Duration::from_secs(300))
        .send_json(body)
        .map_err(map_ureq)?;
    let value: Value = response
        .into_json()
        .map_err(|err| BloraError::Exec(err.to_string()))?;
    if let Some(id) = value.get("id").and_then(Value::as_str) {
        return Ok(VideoOutput::Job(id.to_owned()));
    }
    first_media(&value, "video/mp4", "mp4").map(VideoOutput::File)
}

pub fn text_to_speech(
    endpoint: &MediaEndpoint,
    text: &str,
    voice: Option<&str>,
    format: Option<&str>,
) -> Result<MediaFile> {
    let format = format.unwrap_or("mp3");
    let response = ureq::post(&format!("{}/audio/speech", endpoint.base))
        .set("Authorization", &format!("Bearer {}", endpoint.key))
        .timeout(Duration::from_secs(120))
        .send_json(json!({
            "model": endpoint.tts_model,
            "input": text,
            "voice": voice.unwrap_or(&endpoint.tts_voice),
            "response_format": format,
        }))
        .map_err(map_ureq)?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|err| BloraError::Exec(err.to_string()))?;
    Ok(MediaFile {
        bytes,
        mime: match format {
            "wav" => "audio/wav",
            "opus" => "audio/opus",
            "aac" => "audio/aac",
            "flac" => "audio/flac",
            _ => "audio/mpeg",
        }
        .to_owned(),
        extension: match format {
            "wav" => "wav",
            "opus" => "opus",
            "aac" => "aac",
            "flac" => "flac",
            _ => "mp3",
        },
    })
}

pub fn vision_analyze(
    endpoint: &MediaEndpoint,
    image: &[u8],
    media_type: &str,
    prompt: &str,
) -> Result<String> {
    let data = base64::engine::general_purpose::STANDARD.encode(image);
    let response = ureq::post(&format!("{}/chat/completions", endpoint.base))
        .set("Authorization", &format!("Bearer {}", endpoint.key))
        .timeout(Duration::from_secs(120))
        .send_json(json!({
            "model": endpoint.vision_model,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": prompt},
                    {"type": "image_url", "image_url": {"url": format!("data:{media_type};base64,{data}")}}
                ]
            }]
        }))
        .map_err(map_ureq)?;
    let value: Value = response
        .into_json()
        .map_err(|err| BloraError::Exec(err.to_string()))?;
    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| BloraError::Other("vision response had no text".to_owned()))
}

fn field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(value.as_bytes());
    body.extend_from_slice(b"\r\n");
}

fn first_image(value: &Value) -> Result<MediaFile> {
    first_media(value, "image/png", "png")
}

fn first_media(value: &Value, mime: &str, extension: &'static str) -> Result<MediaFile> {
    let item = value
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or_else(|| BloraError::Other("media response had no data".to_owned()))?;
    if let Some(encoded) = item.get("b64_json").and_then(Value::as_str) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|err| BloraError::Other(format!("media base64: {err}")))?;
        return Ok(MediaFile {
            bytes,
            mime: mime.to_owned(),
            extension,
        });
    }
    if let Some(url) = item.get("url").and_then(Value::as_str) {
        let mut bytes = Vec::new();
        ureq::get(url)
            .timeout(Duration::from_secs(120))
            .call()
            .map_err(map_ureq)?
            .into_reader()
            .read_to_end(&mut bytes)
            .map_err(|err| BloraError::Exec(err.to_string()))?;
        return Ok(MediaFile {
            bytes,
            mime: mime.to_owned(),
            extension,
        });
    }
    Err(BloraError::Other(
        "media response had neither b64_json nor url".to_owned(),
    ))
}

fn extension_for(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    }
}

fn map_ureq(error: ureq::Error) -> BloraError {
    BloraError::Other(error.to_string())
}
