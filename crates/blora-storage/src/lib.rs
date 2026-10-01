// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Local SQLite persistence. The event table is the source of truth.

mod approvals;
mod onboarding;
mod sqlite;
mod tasks;
mod users;

pub use approvals::ApprovalRecord;
pub use sqlite::{
    ArtifactRecord, CreateSession, MemoryRecord, SessionSummary, SqliteStore, UsageTotals,
};
pub use tasks::{AgentRecord, CreateTask, TaskRecord};
pub use users::UserRecord;
