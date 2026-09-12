// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_types::{
    Actor, AgentId, ApprovalId, ArtifactId, BloraError, Mode, Result, SessionId, TaskId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionCreated {
    pub title: Option<String>,
    pub workspace_path: String,
    pub mode: Mode,
    #[serde(default)]
    pub parent_session_id: Option<SessionId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionResumed {
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionForked {
    pub source_session_id: SessionId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionArchived {
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunCreated {
    pub mode: Mode,
    pub model: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunStarted {
    pub model: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RunPaused {
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RunCancelRequested {
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RunCompleted {
    pub summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunFailed {
    pub error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RunCancelled {
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserInput {
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssistantDelta {
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessageCompleted {
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssistantReasoning {
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelRequested {
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelResponseCompleted {
    pub finish_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolRequested {
    pub tool: String,
    pub arguments: Value,
    #[serde(default)]
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolStarted {
    pub tool: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolOutput {
    pub text: String,
    #[serde(default)]
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCompleted {
    pub tool: String,
    pub ok: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolFailed {
    pub tool: String,
    pub error: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequested {
    pub approval_id: ApprovalId,
    pub capability: String,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApprovalResolved {
    pub approval_id: ApprovalId,
    pub decision: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolicyDenied {
    pub capability: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskCreated {
    pub task_id: TaskId,
    pub title: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub delay_until: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskStarted {
    pub task_id: TaskId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskProgress {
    pub task_id: TaskId,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskCompleted {
    pub task_id: TaskId,
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskFailed {
    pub task_id: TaskId,
    pub error: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskWakeup {
    pub task_id: TaskId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentSpawned {
    pub agent_id: AgentId,
    pub role: String,
    #[serde(default)]
    pub child_session_id: Option<SessionId>,
    #[serde(default)]
    pub prompt: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentMessage {
    pub agent_id: AgentId,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentCompleted {
    pub agent_id: AgentId,
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentFailed {
    pub agent_id: AgentId,
    pub error: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextSnapshotCreated {
    pub snapshot_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextDeltaCreated {
    pub snapshot_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextCompactionStarted {
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextCompactionCompleted {
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetryStarted {
    pub attempt: u32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderChanged {
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageRecorded {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtifactCreated {
    pub artifact_id: ArtifactId,
    pub kind: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckpointCreated {
    pub sequence: u64,
    pub note: Option<String>,
}

/// Strongly-typed payloads. Unknown wire types stay as raw JSON on the envelope.
#[derive(Clone, Debug, PartialEq)]
pub enum KnownPayload {
    SessionCreated(SessionCreated),
    SessionResumed(SessionResumed),
    SessionForked(SessionForked),
    SessionArchived(SessionArchived),
    RunCreated(RunCreated),
    RunStarted(RunStarted),
    RunPaused(RunPaused),
    RunCancelRequested(RunCancelRequested),
    RunCompleted(RunCompleted),
    RunFailed(RunFailed),
    RunCancelled(RunCancelled),
    UserInput(UserInput),
    AssistantDelta(AssistantDelta),
    AssistantMessageCompleted(AssistantMessageCompleted),
    AssistantReasoning(AssistantReasoning),
    ModelRequested(ModelRequested),
    ModelResponseCompleted(ModelResponseCompleted),
    ToolRequested(ToolRequested),
    ToolStarted(ToolStarted),
    ToolOutput(ToolOutput),
    ToolCompleted(ToolCompleted),
    ToolFailed(ToolFailed),
    ApprovalRequested(ApprovalRequested),
    ApprovalResolved(ApprovalResolved),
    PolicyDenied(PolicyDenied),
    TaskCreated(TaskCreated),
    TaskStarted(TaskStarted),
    TaskProgress(TaskProgress),
    TaskCompleted(TaskCompleted),
    TaskFailed(TaskFailed),
    TaskWakeup(TaskWakeup),
    SubagentSpawned(SubagentSpawned),
    SubagentMessage(SubagentMessage),
    SubagentCompleted(SubagentCompleted),
    SubagentFailed(SubagentFailed),
    ContextSnapshotCreated(ContextSnapshotCreated),
    ContextDeltaCreated(ContextDeltaCreated),
    ContextCompactionStarted(ContextCompactionStarted),
    ContextCompactionCompleted(ContextCompactionCompleted),
    RetryStarted(RetryStarted),
    ProviderChanged(ProviderChanged),
    UsageRecorded(UsageRecorded),
    ArtifactCreated(ArtifactCreated),
    CheckpointCreated(CheckpointCreated),
}

impl KnownPayload {
    #[must_use]
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::SessionCreated(_) => "session.created",
            Self::SessionResumed(_) => "session.resumed",
            Self::SessionForked(_) => "session.forked",
            Self::SessionArchived(_) => "session.archived",
            Self::RunCreated(_) => "run.created",
            Self::RunStarted(_) => "run.started",
            Self::RunPaused(_) => "run.paused",
            Self::RunCancelRequested(_) => "run.cancel_requested",
            Self::RunCompleted(_) => "run.completed",
            Self::RunFailed(_) => "run.failed",
            Self::RunCancelled(_) => "run.cancelled",
            Self::UserInput(_) => "user.input",
            Self::AssistantDelta(_) => "assistant.delta",
            Self::AssistantMessageCompleted(_) => "assistant.message.completed",
            Self::AssistantReasoning(_) => "assistant.reasoning",
            Self::ModelRequested(_) => "model.requested",
            Self::ModelResponseCompleted(_) => "model.response.completed",
            Self::ToolRequested(_) => "tool.requested",
            Self::ToolStarted(_) => "tool.started",
            Self::ToolOutput(_) => "tool.output",
            Self::ToolCompleted(_) => "tool.completed",
            Self::ToolFailed(_) => "tool.failed",
            Self::ApprovalRequested(_) => "approval.requested",
            Self::ApprovalResolved(_) => "approval.resolved",
            Self::PolicyDenied(_) => "policy.denied",
            Self::TaskCreated(_) => "task.created",
            Self::TaskStarted(_) => "task.started",
            Self::TaskProgress(_) => "task.progress",
            Self::TaskCompleted(_) => "task.completed",
            Self::TaskFailed(_) => "task.failed",
            Self::TaskWakeup(_) => "task.wakeup",
            Self::SubagentSpawned(_) => "subagent.spawned",
            Self::SubagentMessage(_) => "subagent.message",
            Self::SubagentCompleted(_) => "subagent.completed",
            Self::SubagentFailed(_) => "subagent.failed",
            Self::ContextSnapshotCreated(_) => "context.snapshot.created",
            Self::ContextDeltaCreated(_) => "context.delta.created",
            Self::ContextCompactionStarted(_) => "context.compaction.started",
            Self::ContextCompactionCompleted(_) => "context.compaction.completed",
            Self::RetryStarted(_) => "retry.started",
            Self::ProviderChanged(_) => "provider.changed",
            Self::UsageRecorded(_) => "usage.recorded",
            Self::ArtifactCreated(_) => "artifact.created",
            Self::CheckpointCreated(_) => "checkpoint.created",
        }
    }

    #[must_use]
    pub fn default_actor(&self) -> Actor {
        match self {
            Self::UserInput(_) | Self::ApprovalResolved(_) => Actor::User,
            Self::AssistantDelta(_)
            | Self::AssistantMessageCompleted(_)
            | Self::AssistantReasoning(_) => Actor::Assistant,
            Self::ToolRequested(_)
            | Self::ToolStarted(_)
            | Self::ToolOutput(_)
            | Self::ToolCompleted(_)
            | Self::ToolFailed(_) => Actor::Tool,
            Self::TaskCreated(_)
            | Self::TaskStarted(_)
            | Self::TaskProgress(_)
            | Self::TaskCompleted(_)
            | Self::TaskFailed(_)
            | Self::TaskWakeup(_) => Actor::Scheduler,
            Self::SubagentSpawned(_)
            | Self::SubagentMessage(_)
            | Self::SubagentCompleted(_)
            | Self::SubagentFailed(_) => Actor::Subagent,
            _ => Actor::System,
        }
    }

    pub fn to_value(&self) -> Result<Value> {
        let value = match self {
            Self::SessionCreated(v) => serde_json::to_value(v),
            Self::SessionResumed(v) => serde_json::to_value(v),
            Self::SessionForked(v) => serde_json::to_value(v),
            Self::SessionArchived(v) => serde_json::to_value(v),
            Self::RunCreated(v) => serde_json::to_value(v),
            Self::RunStarted(v) => serde_json::to_value(v),
            Self::RunPaused(v) => serde_json::to_value(v),
            Self::RunCancelRequested(v) => serde_json::to_value(v),
            Self::RunCompleted(v) => serde_json::to_value(v),
            Self::RunFailed(v) => serde_json::to_value(v),
            Self::RunCancelled(v) => serde_json::to_value(v),
            Self::UserInput(v) => serde_json::to_value(v),
            Self::AssistantDelta(v) => serde_json::to_value(v),
            Self::AssistantMessageCompleted(v) => serde_json::to_value(v),
            Self::AssistantReasoning(v) => serde_json::to_value(v),
            Self::ModelRequested(v) => serde_json::to_value(v),
            Self::ModelResponseCompleted(v) => serde_json::to_value(v),
            Self::ToolRequested(v) => serde_json::to_value(v),
            Self::ToolStarted(v) => serde_json::to_value(v),
            Self::ToolOutput(v) => serde_json::to_value(v),
            Self::ToolCompleted(v) => serde_json::to_value(v),
            Self::ToolFailed(v) => serde_json::to_value(v),
            Self::ApprovalRequested(v) => serde_json::to_value(v),
            Self::ApprovalResolved(v) => serde_json::to_value(v),
            Self::PolicyDenied(v) => serde_json::to_value(v),
            Self::TaskCreated(v) => serde_json::to_value(v),
            Self::TaskStarted(v) => serde_json::to_value(v),
            Self::TaskProgress(v) => serde_json::to_value(v),
            Self::TaskCompleted(v) => serde_json::to_value(v),
            Self::TaskFailed(v) => serde_json::to_value(v),
            Self::TaskWakeup(v) => serde_json::to_value(v),
            Self::SubagentSpawned(v) => serde_json::to_value(v),
            Self::SubagentMessage(v) => serde_json::to_value(v),
            Self::SubagentCompleted(v) => serde_json::to_value(v),
            Self::SubagentFailed(v) => serde_json::to_value(v),
            Self::ContextSnapshotCreated(v) => serde_json::to_value(v),
            Self::ContextDeltaCreated(v) => serde_json::to_value(v),
            Self::ContextCompactionStarted(v) => serde_json::to_value(v),
            Self::ContextCompactionCompleted(v) => serde_json::to_value(v),
            Self::RetryStarted(v) => serde_json::to_value(v),
            Self::ProviderChanged(v) => serde_json::to_value(v),
            Self::UsageRecorded(v) => serde_json::to_value(v),
            Self::ArtifactCreated(v) => serde_json::to_value(v),
            Self::CheckpointCreated(v) => serde_json::to_value(v),
        };
        value.map_err(|err| BloraError::event(err.to_string()))
    }

    pub fn from_type(event_type: &str, payload: &Value) -> Result<Option<Self>> {
        let value = payload.clone();
        let known = match event_type {
            "session.created" => Self::SessionCreated(from_value(value)?),
            "session.resumed" => Self::SessionResumed(from_value(value)?),
            "session.forked" => Self::SessionForked(from_value(value)?),
            "session.archived" => Self::SessionArchived(from_value(value)?),
            "run.created" => Self::RunCreated(from_value(value)?),
            "run.started" => Self::RunStarted(from_value(value)?),
            "run.paused" => Self::RunPaused(from_value(value)?),
            "run.cancel_requested" => Self::RunCancelRequested(from_value(value)?),
            "run.completed" => Self::RunCompleted(from_value(value)?),
            "run.failed" => Self::RunFailed(from_value(value)?),
            "run.cancelled" => Self::RunCancelled(from_value(value)?),
            "user.input" => Self::UserInput(from_value(value)?),
            "assistant.delta" => Self::AssistantDelta(from_value(value)?),
            "assistant.message.completed" => Self::AssistantMessageCompleted(from_value(value)?),
            "assistant.reasoning" => Self::AssistantReasoning(from_value(value)?),
            "model.requested" => Self::ModelRequested(from_value(value)?),
            "model.response.completed" => Self::ModelResponseCompleted(from_value(value)?),
            "tool.requested" => Self::ToolRequested(from_value(value)?),
            "tool.started" => Self::ToolStarted(from_value(value)?),
            "tool.output" => Self::ToolOutput(from_value(value)?),
            "tool.completed" => Self::ToolCompleted(from_value(value)?),
            "tool.failed" => Self::ToolFailed(from_value(value)?),
            "approval.requested" => Self::ApprovalRequested(from_value(value)?),
            "approval.resolved" => Self::ApprovalResolved(from_value(value)?),
            "policy.denied" => Self::PolicyDenied(from_value(value)?),
            "task.created" => Self::TaskCreated(from_value(value)?),
            "task.started" => Self::TaskStarted(from_value(value)?),
            "task.progress" => Self::TaskProgress(from_value(value)?),
            "task.completed" => Self::TaskCompleted(from_value(value)?),
            "task.failed" => Self::TaskFailed(from_value(value)?),
            "task.wakeup" => Self::TaskWakeup(from_value(value)?),
            "subagent.spawned" => Self::SubagentSpawned(from_value(value)?),
            "subagent.message" => Self::SubagentMessage(from_value(value)?),
            "subagent.completed" => Self::SubagentCompleted(from_value(value)?),
            "subagent.failed" => Self::SubagentFailed(from_value(value)?),
            "context.snapshot.created" => Self::ContextSnapshotCreated(from_value(value)?),
            "context.delta.created" => Self::ContextDeltaCreated(from_value(value)?),
            "context.compaction.started" => Self::ContextCompactionStarted(from_value(value)?),
            "context.compaction.completed" => Self::ContextCompactionCompleted(from_value(value)?),
            "retry.started" => Self::RetryStarted(from_value(value)?),
            "provider.changed" => Self::ProviderChanged(from_value(value)?),
            "usage.recorded" => Self::UsageRecorded(from_value(value)?),
            "artifact.created" => Self::ArtifactCreated(from_value(value)?),
            "checkpoint.created" => Self::CheckpointCreated(from_value(value)?),
            _ => return Ok(None),
        };
        Ok(Some(known))
    }
}

fn from_value<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|err| BloraError::event(err.to_string()))
}
