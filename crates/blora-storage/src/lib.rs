// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Local SQLite persistence. The event table is the source of truth.

mod approvals;
mod sqlite;
mod tasks;

pub use approvals::ApprovalRecord;
pub use sqlite::{CreateSession, MemoryRecord, SessionSummary, SqliteStore, UsageTotals};
pub use tasks::{AgentRecord, CreateTask, TaskRecord};
