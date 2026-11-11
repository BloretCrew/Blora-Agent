// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use serde::{Deserialize, Serialize};

/// How a provider can return a caller-supplied JSON schema.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutputMode {
    /// Native `response_format`/`responseSchema` support.
    Native,
    /// No native support; force a tool call instead.
    #[default]
    ToolOnly,
    /// Not supported.
    None,
}

/// Model features that change how messages and tools are serialized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub tool_result_images: bool,
    #[serde(default)]
    pub structured_output: StructuredOutputMode,
    #[serde(default)]
    pub audio_output: bool,
    #[serde(default)]
    pub video_input: bool,
    #[serde(default)]
    pub reasoning: bool,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            vision: false,
            tool_result_images: false,
            structured_output: StructuredOutputMode::ToolOnly,
            audio_output: false,
            video_input: false,
            reasoning: false,
        }
    }
}

impl ModelCapabilities {
    #[must_use]
    pub fn text_only() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn vision() -> Self {
        Self {
            vision: true,
            tool_result_images: true,
            structured_output: StructuredOutputMode::ToolOnly,
            ..Self::default()
        }
    }

    /// Best-effort defaults for common model families.
    #[must_use]
    pub fn heuristic(model: &str) -> Self {
        let name = model.to_ascii_lowercase();
        let claude_vision = name.contains("claude")
            && !name.contains("claude-2")
            && !name.contains("claude-instant");
        let vision = claude_vision
            || name.contains("gpt-4o")
            || name.contains("gpt-4.1")
            || name.contains("gpt-5")
            || name.contains("o3")
            || name.contains("o4")
            || name.contains("gemini")
            || name.contains("qwen-vl")
            || name.contains("vision");
        let structured = if name.contains("gpt-4o")
            || name.contains("gpt-4.1")
            || name.contains("gpt-5")
            || name.contains("gemini")
        {
            StructuredOutputMode::Native
        } else {
            StructuredOutputMode::ToolOnly
        };
        let reasoning = name.contains("o1")
            || name.contains("o3")
            || name.contains("o4")
            || name.contains("reason")
            || name.contains("thinking");
        Self {
            vision,
            tool_result_images: vision,
            structured_output: structured,
            audio_output: false,
            video_input: name.contains("gemini"),
            reasoning,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristics_cover_common_families() {
        assert!(ModelCapabilities::heuristic("claude-sonnet-5-5").vision);
        assert!(ModelCapabilities::heuristic("gpt-4o-mini").vision);
        assert_eq!(
            ModelCapabilities::heuristic("gpt-4o").structured_output,
            StructuredOutputMode::Native
        );
        assert!(!ModelCapabilities::heuristic("deepseek-chat").vision);
    }
}
