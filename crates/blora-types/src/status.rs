// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Active,
    Archived,
}

impl SessionStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }

    pub fn parse(value: &str) -> crate::Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            other => Err(crate::BloraError::Other(format!(
                "unknown session status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    #[default]
    Queued,
    Running,
    WaitingApproval,
    WaitingInput,
    WaitingTask,
    Compacting,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl RunStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::WaitingInput => "waiting_input",
            Self::WaitingTask => "waiting_task",
            Self::Compacting => "compacting",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn parse(value: &str) -> crate::Result<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "waiting_approval" => Ok(Self::WaitingApproval),
            "waiting_input" => Ok(Self::WaitingInput),
            "waiting_task" => Ok(Self::WaitingTask),
            "compacting" => Ok(Self::Compacting),
            "paused" => Ok(Self::Paused),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(crate::BloraError::Other(format!(
                "unknown run status: {other}"
            ))),
        }
    }
}
