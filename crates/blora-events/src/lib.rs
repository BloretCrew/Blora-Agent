// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Canonical event log types. Unknown event types are preserved and skipped
//! by projections instead of failing session replay.

mod envelope;
mod payload;

pub use envelope::{EventEnvelope, NewEvent};
pub use payload::{
    ApprovalRequested, ApprovalResolved, ArtifactCreated, ArtifactDeleted, ArtifactPresented,
    ArtifactUpdated, AskedQuestion, AssistantDelta, AssistantMessageCompleted, AssistantReasoning,
    CheckpointCreated, ContextCompactionCompleted, ContextCompactionFailed,
    ContextCompactionStarted, ContextDeltaCreated, ContextSnapshotCreated, ExternalSessionBound,
    HarnessChanged, HookCompleted, InputProvided, InputRequested, KnownPayload, ModeChanged,
    ModelRequested, ModelResponseCompleted, PermissionChanged, PlanModeChanged, PlanStep,
    PlanUpdated, PolicyDenied, ProviderChanged, QuestionChoice, RetryStarted, RoutingChanged,
    RunCancelRequested, RunCancelled, RunCompleted, RunCreated, RunFailed, RunPaused, RunStarted,
    SessionArchived, SessionCreated, SessionCwdChanged, SessionForked, SessionResumed,
    SessionTitleChanged, SkillChanged, StructuredOutput, SubagentCompleted, SubagentFailed,
    SubagentMessage, SubagentSpawned, TaskCompleted, TaskCreated, TaskDeleted, TaskFailed,
    TaskProgress, TaskStarted, TaskUpdated, TaskWakeup, ToolCompleted, ToolDiscovered, ToolFailed,
    ToolOutput, ToolRequested, ToolStarted, UsageRecorded, UserInput,
};
