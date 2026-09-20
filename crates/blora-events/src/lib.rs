// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Canonical event log types. Unknown event types are preserved and skipped
//! by projections instead of failing session replay.

mod envelope;
mod payload;

pub use envelope::{EventEnvelope, NewEvent};
pub use payload::{
    ApprovalRequested, ApprovalResolved, ArtifactCreated, AssistantDelta,
    AssistantMessageCompleted, AssistantReasoning, CheckpointCreated, ContextCompactionCompleted,
    ContextCompactionFailed, ContextCompactionStarted, ContextDeltaCreated, ContextSnapshotCreated,
    HookCompleted, KnownPayload, ModelRequested, ModelResponseCompleted, PolicyDenied,
    ModeChanged, ProviderChanged, RetryStarted, RoutingChanged, RunCancelRequested, RunCancelled, RunCompleted, RunCreated,
    RunFailed, RunPaused, RunStarted, SessionArchived, SessionCreated, SessionForked,
    SessionResumed, SessionTitleChanged, SubagentCompleted, SubagentFailed, SubagentMessage, SubagentSpawned,
    TaskCompleted, TaskCreated, TaskFailed, TaskProgress, TaskStarted, TaskWakeup, ToolCompleted,
    ToolFailed, ToolOutput, ToolRequested, ToolStarted, UsageRecorded, UserInput,
};
