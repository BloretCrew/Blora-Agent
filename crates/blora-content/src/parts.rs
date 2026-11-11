// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use serde::{Deserialize, Serialize};

/// One piece of content sent to a model or returned by a tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
    },
    Image {
        media_type: String,
        data: ImageData,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<ImageDetail>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alt: Option<String>,
    },
    File {
        media_type: String,
        name: String,
        data: ImageData,
    },
    Json {
        value: serde_json::Value,
    },
}

impl ContentPart {
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    #[must_use]
    pub fn json(value: serde_json::Value) -> Self {
        Self::Json { value }
    }

    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_image(&self) -> bool {
        matches!(self, Self::Image { .. })
    }

    #[must_use]
    pub fn is_attachment(&self) -> bool {
        matches!(self, Self::Image { .. } | Self::File { .. })
    }
}

/// Image payload location. Bytes are not serialized directly into events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImageData {
    Base64 {
        data: String,
    },
    Path {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bytes: Option<u64>,
    },
    Url {
        url: String,
    },
}

impl ImageData {
    #[must_use]
    pub fn path(path: impl Into<String>) -> Self {
        Self::Path {
            path: path.into(),
            sha256: None,
            bytes: None,
        }
    }
}

/// Provider hint for image fidelity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageDetail {
    #[default]
    Auto,
    Low,
    High,
}

impl ImageDetail {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Low => "low",
            Self::High => "high",
        }
    }
}

/// A durable reference to a file stored outside the event log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentRef {
    pub id: String,
    /// `image`, `file`, `audio`, or `video`.
    pub kind: String,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Path relative to the workspace.
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

impl AttachmentRef {
    #[must_use]
    pub fn is_image(&self) -> bool {
        self.kind == "image"
    }
}

/// Loads attachment bytes when the caller compiles a transcript.
///
/// Implementations live outside this crate and enforce workspace policy.
pub trait AttachmentResolver {
    /// Read the bytes for `reference`.
    ///
    /// # Errors
    ///
    /// Returns a user-facing message when the attachment is missing or escapes
    /// the workspace.
    fn read(&self, reference: &AttachmentRef) -> Result<Vec<u8>, String>;
}
