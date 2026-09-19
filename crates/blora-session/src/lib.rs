// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Session and run projections rebuilt from the canonical event log.

mod machine;
mod projection;
mod tool_summary;

pub use machine::transition_run;
pub use projection::{
    ApprovalView, RunRecord, SessionProjection, SessionRecord, SubagentView, TaskView,
    TranscriptItem, apply_event, format_routing_switch, rebuild, relative_zh,
};
pub use tool_summary::summarize_tool_run;
