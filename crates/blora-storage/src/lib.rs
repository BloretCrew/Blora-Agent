// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Local SQLite persistence. The event table is the source of truth.

mod sqlite;
mod tasks;

pub use sqlite::{CreateSession, SessionSummary, SqliteStore};
pub use tasks::{AgentRecord, CreateTask, TaskRecord};
