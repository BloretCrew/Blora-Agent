// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    User,
    Assistant,
    Tool,
    System,
    Subagent,
    Scheduler,
}

impl Actor {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::System => "system",
            Self::Subagent => "subagent",
            Self::Scheduler => "scheduler",
        }
    }

    pub fn parse(value: &str) -> crate::Result<Self> {
        match value {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "tool" => Ok(Self::Tool),
            "system" => Ok(Self::System),
            "subagent" => Ok(Self::Subagent),
            "scheduler" => Ok(Self::Scheduler),
            other => Err(crate::BloraError::Other(format!("unknown actor: {other}"))),
        }
    }
}
