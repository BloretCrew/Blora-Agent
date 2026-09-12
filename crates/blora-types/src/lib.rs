// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Shared identifiers, enumerations, and errors for Blora Agent.

mod actor;
mod cancel;
mod error;
mod id;
mod mode;
mod status;
mod time;
mod visibility;

pub use actor::Actor;
pub use cancel::CancelToken;
pub use error::{BloraError, Result};
pub use id::{
    AgentId, ApprovalId, ArtifactId, CheckpointId, EventId, RunId, SessionId, TaskId, TurnId,
    WorkspaceId,
};
pub use mode::Mode;
pub use status::{RunStatus, SessionStatus};
pub use time::{Clock, FrozenClock, SystemClock};
pub use visibility::Visibility;

/// Canonical event schema version written by this release.
pub const SCHEMA_VERSION: u32 = 1;
