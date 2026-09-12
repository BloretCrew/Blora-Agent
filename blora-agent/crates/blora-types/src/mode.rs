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
}

impl Mode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Work => "work",
            Self::Agent => "agent",
        }
    }

    pub fn parse(value: &str) -> crate::Result<Self> {
        match value {
            "code" => Ok(Self::Code),
            "work" => Ok(Self::Work),
            "agent" => Ok(Self::Agent),
            other => Err(crate::BloraError::Other(format!("unknown mode: {other}"))),
        }
    }
}
