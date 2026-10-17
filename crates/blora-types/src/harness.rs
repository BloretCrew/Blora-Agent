// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use serde::{Deserialize, Serialize};

/// Stable execution backend identifier, independent of provider configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HarnessKind {
    #[default]
    #[serde(rename = "blora")]
    Blora,
    #[serde(rename = "claude-code")]
    ClaudeCode,
    #[serde(rename = "codex")]
    Codex,
    #[serde(rename = "grok-build")]
    GrokBuild,
    #[serde(rename = "hermes-agent")]
    HermesAgent,
    #[serde(rename = "openclaw")]
    OpenClaw,
    #[serde(rename = "opencode")]
    OpenCode,
    #[serde(rename = "qwen-code")]
    QwenCode,
    #[serde(rename = "deepseek-harness")]
    DeepSeekHarness,
}

impl HarnessKind {
    pub const fn all() -> [Self; 9] {
        [
            Self::Blora,
            Self::ClaudeCode,
            Self::Codex,
            Self::GrokBuild,
            Self::HermesAgent,
            Self::OpenClaw,
            Self::OpenCode,
            Self::QwenCode,
            Self::DeepSeekHarness,
        ]
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Blora => "blora",
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::GrokBuild => "grok-build",
            Self::HermesAgent => "hermes-agent",
            Self::OpenClaw => "openclaw",
            Self::OpenCode => "opencode",
            Self::QwenCode => "qwen-code",
            Self::DeepSeekHarness => "deepseek-harness",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Blora => "Blora",
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::GrokBuild => "Grok Build",
            Self::HermesAgent => "Hermes Agent",
            Self::OpenClaw => "OpenClaw",
            Self::OpenCode => "OpenCode",
            Self::QwenCode => "Qwen Code",
            Self::DeepSeekHarness => "DeepSeek Harness",
        }
    }
}

impl std::fmt::Display for HarnessKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_ids_and_default() {
        assert_eq!(HarnessKind::default(), HarnessKind::Blora);
        for kind in HarnessKind::all() {
            assert!(!kind.display_name().is_empty());
            let value =
                serde::de::value::StrDeserializer::<serde::de::value::Error>::new(kind.as_str());
            assert_eq!(HarnessKind::deserialize(value).unwrap(), kind);
        }
    }
}
