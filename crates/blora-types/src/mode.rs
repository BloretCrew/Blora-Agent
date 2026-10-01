// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use serde::{Deserialize, Serialize};

/// Shared runtime modes. These are strategies, not separate kernels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Code,
    Work,
    Agent,
    /// Generate images from a description and save them in the workspace.
    Imagine,
}

impl Mode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Work => "work",
            Self::Agent => "agent",
            Self::Imagine => "imagine",
        }
    }

    pub fn parse(value: &str) -> crate::Result<Self> {
        match value {
            "code" => Ok(Self::Code),
            "work" => Ok(Self::Work),
            "agent" => Ok(Self::Agent),
            "imagine" => Ok(Self::Imagine),
            other => Err(crate::BloraError::Other(format!("unknown mode: {other}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_imagine_and_rejects_unknown() {
        assert_eq!(Mode::parse("imagine").unwrap(), Mode::Imagine);
        assert_eq!(Mode::Imagine.as_str(), "imagine");
        assert!(Mode::parse("picture").is_err());
    }
}
