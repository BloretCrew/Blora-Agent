// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    User,
    Internal,
    Audit,
}

impl Visibility {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Internal => "internal",
            Self::Audit => "audit",
        }
    }

    pub fn parse(value: &str) -> crate::Result<Self> {
        match value {
            "user" => Ok(Self::User),
            "internal" => Ok(Self::Internal),
            "audit" => Ok(Self::Audit),
            other => Err(crate::BloraError::Other(format!(
                "unknown visibility: {other}"
            ))),
        }
    }
}
