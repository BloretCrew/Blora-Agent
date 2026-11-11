// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Provider-neutral content parts, attachment references, tool outcomes, and
//! model capability flags.
//!
//! This crate is deliberately free of IO and provider types. Binary payloads
//! travel as base64, a workspace path plus hash, or a URL; resolving a path is
//! the caller's job through [`AttachmentResolver`].

mod capability;
mod outcome;
mod parts;

pub use capability::{ModelCapabilities, StructuredOutputMode};
pub use outcome::{ToolMetadata, ToolOutcome};
pub use parts::{AttachmentRef, AttachmentResolver, ContentPart, ImageData, ImageDetail};
