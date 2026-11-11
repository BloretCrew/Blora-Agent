// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Durable storage for tool and user attachments.
//!
//! Bytes live under `.blora/attachments/<session>/<sha256>.<ext>`; the event log
//! keeps only an [`AttachmentRef`]. Resolving a reference back to bytes is an
//! explicit step so event replay never reads the filesystem.

use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use blora_content::{AttachmentRef, AttachmentResolver, ContentPart, ImageData};
use blora_types::{BloraError, Result};
use sha2::{Digest, Sha256};

const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct AttachmentStore<'a> {
    workspace: &'a Path,
}

impl<'a> AttachmentStore<'a> {
    #[must_use]
    pub fn new(workspace: &'a Path) -> Self {
        Self { workspace }
    }

    #[must_use]
    pub fn root(&self) -> PathBuf {
        self.workspace.join(".blora/attachments")
    }

    /// Write bytes and return the durable reference.
    pub fn save_bytes(
        &self,
        session_id: &str,
        kind: &str,
        mime: &str,
        name: Option<&str>,
        bytes: &[u8],
    ) -> Result<AttachmentRef> {
        if bytes.is_empty() {
            return Err(BloraError::Other("empty attachment".to_owned()));
        }
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(BloraError::Other(format!(
                "attachment is {} bytes; limit is {MAX_ATTACHMENT_BYTES}",
                bytes.len()
            )));
        }
        if !session_id.is_empty() && !session_id.chars().all(is_safe_component) {
            return Err(BloraError::Policy(
                "invalid session id for attachment".to_owned(),
            ));
        }
        let sha256 = hex_sha256(bytes);
        let extension = extension_for(mime, name);
        let relative = if session_id.is_empty() {
            format!(".blora/attachments/{sha256}.{extension}")
        } else {
            format!(".blora/attachments/{session_id}/{sha256}.{extension}")
        };
        let path = self.workspace.join(&relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(BloraError::exec)?;
        }
        fs::write(&path, bytes).map_err(BloraError::exec)?;
        let (width, height) = image_size(bytes);
        Ok(AttachmentRef {
            id: format!("att_{sha256}"),
            kind: kind.to_owned(),
            mime: mime.to_owned(),
            name: name.map(ToOwned::to_owned),
            path: relative,
            sha256,
            bytes: bytes.len() as u64,
            width,
            height,
        })
    }

    /// Decode a base64 payload and store it.
    pub fn save_base64(
        &self,
        session_id: &str,
        kind: &str,
        mime: &str,
        name: Option<&str>,
        data: &str,
    ) -> Result<AttachmentRef> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data.trim())
            .map_err(|err| BloraError::Other(format!("invalid base64 attachment: {err}")))?;
        self.save_bytes(session_id, kind, mime, name, &bytes)
    }

    /// Read the bytes for a reference, refusing paths outside the workspace.
    pub fn load(&self, reference: &AttachmentRef) -> Result<Vec<u8>> {
        let path = self.resolve(&reference.path)?;
        fs::read(path).map_err(BloraError::exec)
    }

    pub fn resolve(&self, relative: &str) -> Result<PathBuf> {
        let candidate = self.workspace.join(relative);
        let canonical = candidate.canonicalize().map_err(BloraError::exec)?;
        let root = self.workspace.canonicalize().map_err(BloraError::exec)?;
        if !canonical.starts_with(&root) {
            return Err(BloraError::Policy(format!(
                "attachment escapes workspace: {relative}"
            )));
        }
        Ok(canonical)
    }

    /// Convert path-backed parts into base64 parts for provider adapters.
    pub fn resolve_parts(&self, parts: &[ContentPart]) -> Result<Vec<ContentPart>> {
        parts
            .iter()
            .map(|part| match part {
                ContentPart::Image {
                    media_type,
                    data: ImageData::Path { path, .. },
                    detail,
                    alt,
                } => {
                    let bytes = self.resolve(path)?;
                    let bytes = fs::read(bytes).map_err(BloraError::exec)?;
                    Ok(ContentPart::Image {
                        media_type: media_type.clone(),
                        data: ImageData::Base64 {
                            data: base64::engine::general_purpose::STANDARD.encode(bytes),
                        },
                        detail: *detail,
                        alt: alt.clone(),
                    })
                }
                ContentPart::File {
                    media_type,
                    name,
                    data: ImageData::Path { path, .. },
                } => {
                    let bytes = self.resolve(path)?;
                    let bytes = fs::read(bytes).map_err(BloraError::exec)?;
                    Ok(ContentPart::File {
                        media_type: media_type.clone(),
                        name: name.clone(),
                        data: ImageData::Base64 {
                            data: base64::engine::general_purpose::STANDARD.encode(bytes),
                        },
                    })
                }
                other => Ok(other.clone()),
            })
            .collect()
    }
}

impl AttachmentResolver for AttachmentStore<'_> {
    fn read(&self, reference: &AttachmentRef) -> std::result::Result<Vec<u8>, String> {
        self.load(reference).map_err(|err| err.to_string())
    }
}

fn is_safe_component(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn extension_for(mime: &str, name: Option<&str>) -> String {
    if let Some(name) = name {
        if let Some(extension) = Path::new(name).extension().and_then(|value| value.to_str()) {
            if extension.len() <= 8 && extension.chars().all(|ch| ch.is_ascii_alphanumeric()) {
                return extension.to_ascii_lowercase();
            }
        }
    }
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "audio/mpeg" => "mp3",
        "audio/wav" => "wav",
        "video/mp4" => "mp4",
        _ => "bin",
    }
    .to_owned()
}

fn image_size(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    image::load_from_memory(bytes)
        .map(|image| (Some(image.width()), Some(image.height())))
        .unwrap_or((None, None))
}

/// Guess an image MIME type from a path extension.
#[must_use]
pub fn image_mime_for(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_resolves_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttachmentStore::new(dir.path());
        let reference = store
            .save_bytes("ses_test", "image", "image/png", None, b"not really png")
            .unwrap();
        assert!(reference.path.starts_with(".blora/attachments/ses_test/"));
        assert_eq!(reference.bytes, 14);
        let loaded = store.load(&reference).unwrap();
        assert_eq!(loaded, b"not really png");
    }

    #[test]
    fn rejects_path_escape() {
        let dir = tempfile::tempdir().unwrap();
        let store = AttachmentStore::new(dir.path());
        let reference = AttachmentRef {
            id: "x".to_owned(),
            kind: "file".to_owned(),
            mime: "text/plain".to_owned(),
            name: None,
            path: "../outside.txt".to_owned(),
            sha256: String::new(),
            bytes: 0,
            width: None,
            height: None,
        };
        assert!(store.load(&reference).is_err());
    }
}
