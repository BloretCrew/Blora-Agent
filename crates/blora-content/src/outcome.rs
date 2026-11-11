// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use serde::{Deserialize, Serialize};

use crate::parts::{AttachmentRef, ContentPart};

/// Metadata attached to a tool result without adding another content part.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolMetadata {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spilled_path: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub extra: serde_json::Value,
}

/// The provider-neutral result of one tool call.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolOutcome {
    pub parts: Vec<ContentPart>,
    #[serde(default, skip_serializing_if = "ToolMetadata::is_empty")]
    pub metadata: ToolMetadata,
}

impl ToolMetadata {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.truncated && self.spilled_path.is_none() && self.extra.is_null()
    }
}

impl ToolOutcome {
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            parts: vec![ContentPart::text(text)],
            metadata: ToolMetadata::default(),
        }
    }

    #[must_use]
    pub fn from_parts(parts: Vec<ContentPart>) -> Self {
        Self {
            parts,
            metadata: ToolMetadata::default(),
        }
    }

    #[must_use]
    pub fn with_metadata(mut self, metadata: ToolMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Text concatenation plus a short placeholder for non-text parts.
    ///
    /// This is the compatibility surface for `ToolRegistry::execute_for`.
    #[must_use]
    pub fn as_text(&self) -> String {
        let mut out = String::new();
        for part in &self.parts {
            if !out.is_empty() {
                out.push('\n');
            }
            match part {
                ContentPart::Text { text } => out.push_str(text),
                ContentPart::Image {
                    media_type,
                    data,
                    alt,
                    ..
                } => {
                    out.push_str(&format!(
                        "[image {media_type}: {}]",
                        alt.as_deref().unwrap_or(&describe_data(data))
                    ));
                }
                ContentPart::File {
                    media_type,
                    name,
                    data,
                } => out.push_str(&format!(
                    "[file {name} ({media_type}): {}]",
                    describe_data(data)
                )),
                ContentPart::Json { value } => out.push_str(&value.to_string()),
            }
        }
        out
    }

    #[must_use]
    pub fn attachments(&self) -> Vec<AttachmentRef> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::Image {
                    media_type, data, ..
                } => attachment_from_part("image", media_type, None, data),
                ContentPart::File {
                    media_type,
                    name,
                    data,
                } => attachment_from_part("file", media_type, Some(name.clone()), data),
                ContentPart::Text { .. } | ContentPart::Json { .. } => None,
            })
            .collect()
    }
}

fn describe_data(data: &crate::parts::ImageData) -> String {
    match data {
        crate::parts::ImageData::Base64 { data } => format!("{} base64 bytes", data.len()),
        crate::parts::ImageData::Path { path, .. } => path.clone(),
        crate::parts::ImageData::Url { url } => url.clone(),
    }
}

fn attachment_from_part(
    kind: &str,
    media_type: &str,
    name: Option<String>,
    data: &crate::parts::ImageData,
) -> Option<AttachmentRef> {
    let crate::parts::ImageData::Path {
        path,
        sha256,
        bytes,
    } = data
    else {
        return None;
    };
    Some(AttachmentRef {
        id: sha256.clone().unwrap_or_else(|| path.clone()),
        kind: kind.to_owned(),
        mime: media_type.to_owned(),
        name,
        path: path.clone(),
        sha256: sha256.clone().unwrap_or_default(),
        bytes: bytes.unwrap_or(0),
        width: None,
        height: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parts::ImageData;

    #[test]
    fn text_outcome_roundtrips() {
        let outcome = ToolOutcome::text("hello");
        assert_eq!(outcome.as_text(), "hello");
        assert!(outcome.metadata.is_empty());
        let json = serde_json::to_string(&outcome).unwrap();
        let back: ToolOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(back, outcome);
    }

    #[test]
    fn path_attachments_are_extracted() {
        let outcome = ToolOutcome::from_parts(vec![ContentPart::Image {
            media_type: "image/png".to_owned(),
            data: ImageData::Path {
                path: ".blora/attachments/s/a.png".to_owned(),
                sha256: Some("abc".to_owned()),
                bytes: Some(12),
            },
            detail: None,
            alt: None,
        }]);
        let refs = outcome.attachments();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].kind, "image");
        assert_eq!(refs[0].sha256, "abc");
    }
}
