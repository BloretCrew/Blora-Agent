// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Object-safe persistence interface consumed by the Blora runtime.

use crate::*;
use blora_events::*;
use blora_session::*;
use blora_types::*;
use chrono::{DateTime, Utc};

/// Runtime persistence contract. Implementations must preserve append-only
/// event history, monotonically increasing per-session sequence numbers,
/// atomic task claims, conditional approval updates, and FIFO steer consumption.
pub trait RuntimeStore: Send + Sync {
    fn add_team_attachment(
        &self,
        task_id: &blora_types::TeamTaskId,
        name: &str,
        mime: &str,
        size: i64,
        sha256: &str,
        path: &str,
    ) -> Result<()>;
    fn add_team_comment(
        &self,
        task_id: &blora_types::TeamTaskId,
        author: &str,
        body: &str,
    ) -> Result<()>;
    fn add_team_link(
        &self,
        task_id: &blora_types::TeamTaskId,
        kind: &str,
        target: &str,
    ) -> Result<()>;
    fn append(&self, new: NewEvent) -> Result<EventEnvelope>;
    fn cas_task_status(
        &self,
        task_id: &TaskId,
        from: &[TaskStatus],
        to: TaskStatus,
        now: DateTime<Utc>,
        error: Option<&str>,
        run_id: Option<&RunId>,
    ) -> Result<bool>;
    fn claim_due_tasks(&self, now: DateTime<Utc>) -> Result<Vec<TaskRecord>>;
    fn clear_passport_users(&self) -> Result<usize>;
    fn complete_onboarding(&self, surface: &str, version: u32) -> Result<()>;
    fn create_session(&self, spec: CreateSession) -> Result<SessionId>;
    fn create_user(&self, name: &str) -> Result<(UserRecord, String)>;
    fn delete_artifact(&self, id: &ArtifactId) -> Result<bool>;
    fn delete_memory(&self, workspace_path: &str, key: &str) -> Result<bool>;
    fn delete_task(&self, task_id: &TaskId) -> Result<bool>;
    fn delete_team(&self, id: &blora_types::TeamId) -> Result<bool>;
    fn delete_team_task(&self, id: &blora_types::TeamTaskId) -> Result<bool>;
    fn delete_workflow(&self, id: &blora_types::WorkflowId) -> Result<bool>;
    fn enable_session_tool(
        &self,
        session_id: &SessionId,
        tool_name: &str,
        source: &str,
    ) -> Result<()>;
    fn get_agent(&self, agent_id: &AgentId) -> Result<AgentRecord>;
    fn get_approval(&self, id: &ApprovalId) -> Result<ApprovalRecord>;
    fn get_goal(&self, id: &blora_types::GoalId) -> Result<Option<GoalRecord>>;
    fn get_memory(&self, workspace_path: &str, key: &str) -> Result<Option<MemoryRecord>>;
    fn get_session_setting(&self, session_id: &SessionId, key: &str) -> Result<Option<String>>;
    fn get_task(&self, task_id: &TaskId) -> Result<TaskRecord>;
    fn get_team(&self, id: &blora_types::TeamId) -> Result<Option<TeamRecord>>;
    fn get_team_task(&self, id: &blora_types::TeamTaskId) -> Result<Option<TeamTaskRecord>>;
    fn get_workflow(&self, id: &blora_types::WorkflowId) -> Result<Option<WorkflowRecord>>;
    fn heartbeat_team_task(&self, id: &blora_types::TeamTaskId) -> Result<bool>;
    fn insert_agent(&self, record: &AgentRecord) -> Result<()>;
    fn insert_approval(&self, record: &ApprovalRecord) -> Result<()>;
    fn insert_artifact(
        &self,
        id: &blora_types::ArtifactId,
        session_id: &SessionId,
        kind: &str,
        path: Option<&str>,
        content: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()>;
    fn insert_artifact_record(&self, record: &ArtifactRecord) -> Result<()>;
    fn insert_checkpoint(
        &self,
        session_id: &SessionId,
        run_id: Option<&RunId>,
        sequence: u64,
        note: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()>;
    fn insert_goal(&self, goal: &GoalRecord) -> Result<()>;
    fn insert_task(&self, id: &TaskId, spec: &CreateTask, now: DateTime<Utc>) -> Result<()>;
    fn insert_team(&self, team: &TeamRecord) -> Result<()>;
    fn insert_team_column(&self, column: &TeamColumnRecord) -> Result<()>;
    fn insert_team_task(&self, task: &TeamTaskRecord) -> Result<()>;
    fn insert_workflow(&self, workflow: &WorkflowRecord) -> Result<()>;
    fn insert_workflow_run(&self, run: &WorkflowRunRecord) -> Result<()>;
    fn last_sequence(&self, session_id: &SessionId) -> Result<u64>;
    fn latest_workflow_run(
        &self,
        workflow_id: &blora_types::WorkflowId,
    ) -> Result<Option<WorkflowRunRecord>>;
    fn list_agents(&self, parent: &SessionId) -> Result<Vec<AgentRecord>>;
    fn list_all_pending_approvals(&self) -> Result<Vec<ApprovalRecord>>;
    fn list_artifacts(&self, session_id: Option<&SessionId>) -> Result<Vec<ArtifactRecord>>;
    fn get_artifact(&self, id: &ArtifactId) -> Result<Option<ArtifactRecord>>;
    fn list_goals(&self, session_id: &SessionId) -> Result<Vec<GoalRecord>>;
    fn list_memories(&self, workspace_path: &str) -> Result<Vec<MemoryRecord>>;
    fn list_pending_approvals(&self, session_id: &SessionId) -> Result<Vec<ApprovalRecord>>;
    fn list_sessions(&self) -> Result<Vec<SessionSummary>>;
    fn list_session_tools(&self, session_id: &SessionId) -> Result<Vec<String>>;
    fn list_sessions_for_user(&self, user_id: Option<&str>) -> Result<Vec<crate::SessionSummary>>;
    fn list_tasks(&self, session_id: Option<&SessionId>) -> Result<Vec<TaskRecord>>;
    fn list_team_attachments(
        &self,
        task_id: &blora_types::TeamTaskId,
    ) -> Result<Vec<TeamAttachmentRecord>>;
    fn list_team_columns(&self, team_id: &blora_types::TeamId) -> Result<Vec<TeamColumnRecord>>;
    fn list_team_comments(
        &self,
        task_id: &blora_types::TeamTaskId,
    ) -> Result<Vec<(String, String, DateTime<Utc>)>>;
    fn list_team_links(&self, task_id: &blora_types::TeamTaskId) -> Result<Vec<TeamLinkRecord>>;
    fn list_team_tasks(&self, team_id: &blora_types::TeamId) -> Result<Vec<TeamTaskRecord>>;
    fn list_teams(&self, workspace_path: &str) -> Result<Vec<TeamRecord>>;
    fn list_users(&self) -> Result<Vec<UserRecord>>;
    fn list_workflows(&self, session_id: &SessionId) -> Result<Vec<WorkflowRecord>>;
    fn load_events(&self, session_id: &SessionId) -> Result<Vec<EventEnvelope>>;
    fn load_events_after(&self, session_id: &SessionId, after: u64) -> Result<Vec<EventEnvelope>>;
    fn load_projection(&self, session_id: &SessionId) -> Result<SessionProjection>;
    fn mark_agent_delivered(&self, agent_id: &AgentId) -> Result<()>;
    fn onboarding_version(&self, surface: &str) -> Result<u32>;
    fn queue_steer(&self, session_id: &SessionId, message: &str) -> Result<()>;
    fn reschedule_task(
        &self,
        task_id: &TaskId,
        delay_until: DateTime<Utc>,
        now: DateTime<Utc>,
        error: &str,
    ) -> Result<()>;
    fn resolve_approval_row(
        &self,
        id: &ApprovalId,
        decision: &str,
        now: DateTime<Utc>,
    ) -> Result<bool>;
    fn run_task_now(&self, task_id: &TaskId, now: DateTime<Utc>) -> Result<bool>;
    fn running_child_count(&self, parent: &SessionId) -> Result<u32>;
    fn search_transcripts(
        &self,
        query: &str,
        limit: usize,
        exclude: Option<&SessionId>,
        user_id: Option<&str>,
    ) -> Result<Vec<SearchMatch>>;
    #[allow(clippy::type_complexity)]
    fn search_transcripts_page(
        &self,
        query: &str,
        limit: usize,
        exclude: Option<&SessionId>,
        user_id: Option<&str>,
        cursor: Option<&(String, i64)>,
    ) -> Result<(Vec<SearchMatch>, Option<(String, i64)>)>;
    fn session_message_previews(
        &self,
        session_ids: &[SessionId],
    ) -> Result<Vec<(SessionId, u64, Option<String>)>>;
    fn session_parent(&self, session_id: &SessionId) -> Result<Option<SessionId>>;
    fn session_user_id(&self, session_id: &SessionId) -> Result<Option<String>>;
    fn set_session_setting(&self, session_id: &SessionId, key: &str, value: &str) -> Result<()>;
    fn set_session_user(&self, session_id: &SessionId, user_id: &str) -> Result<()>;
    fn set_task_attempt(&self, task_id: &TaskId, attempt: u32, now: DateTime<Utc>) -> Result<()>;
    fn set_team_task_status(
        &self,
        id: &blora_types::TeamTaskId,
        status: &str,
        column_id: Option<&str>,
        assignee: Option<&str>,
        blocked_reason: Option<&str>,
        review_state: Option<&str>,
    ) -> Result<bool>;
    fn set_tui_message_roles(&self, enabled: bool) -> Result<()>;
    fn set_tui_reduce_motion(&self, enabled: bool) -> Result<()>;
    fn take_steers(&self, session_id: &SessionId) -> Result<Vec<String>>;
    fn tui_message_roles(&self) -> Result<bool>;
    fn tui_reduce_motion(&self) -> Result<bool>;
    fn update_agent(
        &self,
        agent_id: &AgentId,
        status: &str,
        summary: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()>;
    fn update_artifact(
        &self,
        id: &ArtifactId,
        title: Option<&str>,
        content: Option<&str>,
        path: Option<&str>,
        mime: Option<&str>,
        size: Option<u64>,
        sha256: Option<&str>,
        metadata: Option<&str>,
    ) -> Result<bool>;
    fn update_goal(
        &self,
        id: &blora_types::GoalId,
        title: Option<&str>,
        description: Option<&str>,
        status: Option<&str>,
        priority: Option<i64>,
        progress: Option<i64>,
        due_at: Option<DateTime<Utc>>,
    ) -> Result<bool>;
    fn update_passport_tokens(
        &self,
        username: &str,
        app_token: &str,
        refresh_token: Option<&str>,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> Result<Option<UserRecord>>;
    fn update_task_fields(
        &self,
        task_id: &TaskId,
        title: Option<&str>,
        prompt: Option<&str>,
        cron: Option<&str>,
        max_attempts: Option<u32>,
        now: DateTime<Utc>,
    ) -> Result<bool>;
    fn update_workflow_run(
        &self,
        id: &blora_types::WorkflowRunId,
        status: &str,
        current_step: i64,
        state_json: &str,
        finished: bool,
    ) -> Result<bool>;
    fn upsert_memory(
        &self,
        workspace_path: &str,
        key: &str,
        value: &str,
        now: DateTime<Utc>,
    ) -> Result<()>;
    fn upsert_passport_user(
        &self,
        username: &str,
        nickname: Option<&str>,
        avatar: Option<&str>,
        email: Option<&str>,
        app_token: Option<&str>,
        refresh_token: Option<&str>,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> Result<UserRecord>;
    fn usage_activity(&self) -> Result<UsageActivity>;
    fn usage_totals(&self, session_id: Option<&SessionId>) -> Result<UsageTotals>;
    fn user_by_passport_username(&self, username: &str) -> Result<Option<UserRecord>>;
    fn user_by_token(&self, token: &str) -> Result<Option<UserRecord>>;
}

impl RuntimeStore for SqliteStore {
    fn add_team_attachment(
        &self,
        task_id: &blora_types::TeamTaskId,
        name: &str,
        mime: &str,
        size: i64,
        sha256: &str,
        path: &str,
    ) -> Result<()> {
        SqliteStore::add_team_attachment(self, task_id, name, mime, size, sha256, path)
    }
    fn add_team_comment(
        &self,
        task_id: &blora_types::TeamTaskId,
        author: &str,
        body: &str,
    ) -> Result<()> {
        SqliteStore::add_team_comment(self, task_id, author, body)
    }
    fn add_team_link(
        &self,
        task_id: &blora_types::TeamTaskId,
        kind: &str,
        target: &str,
    ) -> Result<()> {
        SqliteStore::add_team_link(self, task_id, kind, target)
    }
    fn append(&self, new: NewEvent) -> Result<EventEnvelope> {
        SqliteStore::append(self, new)
    }
    fn cas_task_status(
        &self,
        task_id: &TaskId,
        from: &[TaskStatus],
        to: TaskStatus,
        now: DateTime<Utc>,
        error: Option<&str>,
        run_id: Option<&RunId>,
    ) -> Result<bool> {
        SqliteStore::cas_task_status(self, task_id, from, to, now, error, run_id)
    }
    fn claim_due_tasks(&self, now: DateTime<Utc>) -> Result<Vec<TaskRecord>> {
        SqliteStore::claim_due_tasks(self, now)
    }
    fn clear_passport_users(&self) -> Result<usize> {
        SqliteStore::clear_passport_users(self)
    }
    fn complete_onboarding(&self, surface: &str, version: u32) -> Result<()> {
        SqliteStore::complete_onboarding(self, surface, version)
    }
    fn create_session(&self, spec: CreateSession) -> Result<SessionId> {
        SqliteStore::create_session(self, spec)
    }
    fn create_user(&self, name: &str) -> Result<(UserRecord, String)> {
        SqliteStore::create_user(self, name)
    }
    fn delete_artifact(&self, id: &ArtifactId) -> Result<bool> {
        SqliteStore::delete_artifact(self, id)
    }
    fn delete_memory(&self, workspace_path: &str, key: &str) -> Result<bool> {
        SqliteStore::delete_memory(self, workspace_path, key)
    }
    fn delete_task(&self, task_id: &TaskId) -> Result<bool> {
        SqliteStore::delete_task(self, task_id)
    }
    fn delete_team(&self, id: &blora_types::TeamId) -> Result<bool> {
        SqliteStore::delete_team(self, id)
    }
    fn delete_team_task(&self, id: &blora_types::TeamTaskId) -> Result<bool> {
        SqliteStore::delete_team_task(self, id)
    }
    fn delete_workflow(&self, id: &blora_types::WorkflowId) -> Result<bool> {
        SqliteStore::delete_workflow(self, id)
    }
    fn enable_session_tool(
        &self,
        session_id: &SessionId,
        tool_name: &str,
        source: &str,
    ) -> Result<()> {
        SqliteStore::enable_session_tool(self, session_id, tool_name, source)
    }
    fn get_agent(&self, agent_id: &AgentId) -> Result<AgentRecord> {
        SqliteStore::get_agent(self, agent_id)
    }
    fn get_approval(&self, id: &ApprovalId) -> Result<ApprovalRecord> {
        SqliteStore::get_approval(self, id)
    }
    fn get_goal(&self, id: &blora_types::GoalId) -> Result<Option<GoalRecord>> {
        SqliteStore::get_goal(self, id)
    }
    fn get_memory(&self, workspace_path: &str, key: &str) -> Result<Option<MemoryRecord>> {
        SqliteStore::get_memory(self, workspace_path, key)
    }
    fn get_session_setting(&self, session_id: &SessionId, key: &str) -> Result<Option<String>> {
        SqliteStore::get_session_setting(self, session_id, key)
    }
    fn get_task(&self, task_id: &TaskId) -> Result<TaskRecord> {
        SqliteStore::get_task(self, task_id)
    }
    fn get_team(&self, id: &blora_types::TeamId) -> Result<Option<TeamRecord>> {
        SqliteStore::get_team(self, id)
    }
    fn get_team_task(&self, id: &blora_types::TeamTaskId) -> Result<Option<TeamTaskRecord>> {
        SqliteStore::get_team_task(self, id)
    }
    fn get_workflow(&self, id: &blora_types::WorkflowId) -> Result<Option<WorkflowRecord>> {
        SqliteStore::get_workflow(self, id)
    }
    fn heartbeat_team_task(&self, id: &blora_types::TeamTaskId) -> Result<bool> {
        SqliteStore::heartbeat_team_task(self, id)
    }
    fn insert_agent(&self, record: &AgentRecord) -> Result<()> {
        SqliteStore::insert_agent(self, record)
    }
    fn insert_approval(&self, record: &ApprovalRecord) -> Result<()> {
        SqliteStore::insert_approval(self, record)
    }
    fn insert_artifact(
        &self,
        id: &blora_types::ArtifactId,
        session_id: &SessionId,
        kind: &str,
        path: Option<&str>,
        content: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        SqliteStore::insert_artifact(self, id, session_id, kind, path, content, now)
    }
    fn insert_artifact_record(&self, record: &ArtifactRecord) -> Result<()> {
        SqliteStore::insert_artifact_record(self, record)
    }
    fn insert_checkpoint(
        &self,
        session_id: &SessionId,
        run_id: Option<&RunId>,
        sequence: u64,
        note: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        SqliteStore::insert_checkpoint(self, session_id, run_id, sequence, note, now)
    }
    fn insert_goal(&self, goal: &GoalRecord) -> Result<()> {
        SqliteStore::insert_goal(self, goal)
    }
    fn insert_task(&self, id: &TaskId, spec: &CreateTask, now: DateTime<Utc>) -> Result<()> {
        SqliteStore::insert_task(self, id, spec, now)
    }
    fn insert_team(&self, team: &TeamRecord) -> Result<()> {
        SqliteStore::insert_team(self, team)
    }
    fn insert_team_column(&self, column: &TeamColumnRecord) -> Result<()> {
        SqliteStore::insert_team_column(self, column)
    }
    fn insert_team_task(&self, task: &TeamTaskRecord) -> Result<()> {
        SqliteStore::insert_team_task(self, task)
    }
    fn insert_workflow(&self, workflow: &WorkflowRecord) -> Result<()> {
        SqliteStore::insert_workflow(self, workflow)
    }
    fn insert_workflow_run(&self, run: &WorkflowRunRecord) -> Result<()> {
        SqliteStore::insert_workflow_run(self, run)
    }
    fn last_sequence(&self, session_id: &SessionId) -> Result<u64> {
        SqliteStore::last_sequence(self, session_id)
    }
    fn latest_workflow_run(
        &self,
        workflow_id: &blora_types::WorkflowId,
    ) -> Result<Option<WorkflowRunRecord>> {
        SqliteStore::latest_workflow_run(self, workflow_id)
    }
    fn list_agents(&self, parent: &SessionId) -> Result<Vec<AgentRecord>> {
        SqliteStore::list_agents(self, parent)
    }
    fn list_all_pending_approvals(&self) -> Result<Vec<ApprovalRecord>> {
        SqliteStore::list_all_pending_approvals(self)
    }
    fn list_artifacts(&self, session_id: Option<&SessionId>) -> Result<Vec<ArtifactRecord>> {
        SqliteStore::list_artifacts(self, session_id)
    }
    fn get_artifact(&self, id: &ArtifactId) -> Result<Option<ArtifactRecord>> {
        SqliteStore::get_artifact(self, id)
    }
    fn list_goals(&self, session_id: &SessionId) -> Result<Vec<GoalRecord>> {
        SqliteStore::list_goals(self, session_id)
    }
    fn list_memories(&self, workspace_path: &str) -> Result<Vec<MemoryRecord>> {
        SqliteStore::list_memories(self, workspace_path)
    }
    fn list_pending_approvals(&self, session_id: &SessionId) -> Result<Vec<ApprovalRecord>> {
        SqliteStore::list_pending_approvals(self, session_id)
    }
    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        SqliteStore::list_sessions(self)
    }
    fn list_session_tools(&self, session_id: &SessionId) -> Result<Vec<String>> {
        SqliteStore::list_session_tools(self, session_id)
    }
    fn list_sessions_for_user(&self, user_id: Option<&str>) -> Result<Vec<crate::SessionSummary>> {
        SqliteStore::list_sessions_for_user(self, user_id)
    }
    fn list_tasks(&self, session_id: Option<&SessionId>) -> Result<Vec<TaskRecord>> {
        SqliteStore::list_tasks(self, session_id)
    }
    fn list_team_attachments(
        &self,
        task_id: &blora_types::TeamTaskId,
    ) -> Result<Vec<TeamAttachmentRecord>> {
        SqliteStore::list_team_attachments(self, task_id)
    }
    fn list_team_columns(&self, team_id: &blora_types::TeamId) -> Result<Vec<TeamColumnRecord>> {
        SqliteStore::list_team_columns(self, team_id)
    }
    fn list_team_comments(
        &self,
        task_id: &blora_types::TeamTaskId,
    ) -> Result<Vec<(String, String, DateTime<Utc>)>> {
        SqliteStore::list_team_comments(self, task_id)
    }
    fn list_team_links(&self, task_id: &blora_types::TeamTaskId) -> Result<Vec<TeamLinkRecord>> {
        SqliteStore::list_team_links(self, task_id)
    }
    fn list_team_tasks(&self, team_id: &blora_types::TeamId) -> Result<Vec<TeamTaskRecord>> {
        SqliteStore::list_team_tasks(self, team_id)
    }
    fn list_teams(&self, workspace_path: &str) -> Result<Vec<TeamRecord>> {
        SqliteStore::list_teams(self, workspace_path)
    }
    fn list_users(&self) -> Result<Vec<UserRecord>> {
        SqliteStore::list_users(self)
    }
    fn list_workflows(&self, session_id: &SessionId) -> Result<Vec<WorkflowRecord>> {
        SqliteStore::list_workflows(self, session_id)
    }
    fn load_events(&self, session_id: &SessionId) -> Result<Vec<EventEnvelope>> {
        SqliteStore::load_events(self, session_id)
    }
    fn load_events_after(&self, session_id: &SessionId, after: u64) -> Result<Vec<EventEnvelope>> {
        SqliteStore::load_events_after(self, session_id, after)
    }
    fn load_projection(&self, session_id: &SessionId) -> Result<SessionProjection> {
        SqliteStore::load_projection(self, session_id)
    }
    fn mark_agent_delivered(&self, agent_id: &AgentId) -> Result<()> {
        SqliteStore::mark_agent_delivered(self, agent_id)
    }
    fn onboarding_version(&self, surface: &str) -> Result<u32> {
        SqliteStore::onboarding_version(self, surface)
    }
    fn queue_steer(&self, session_id: &SessionId, message: &str) -> Result<()> {
        SqliteStore::queue_steer(self, session_id, message)
    }
    fn reschedule_task(
        &self,
        task_id: &TaskId,
        delay_until: DateTime<Utc>,
        now: DateTime<Utc>,
        error: &str,
    ) -> Result<()> {
        SqliteStore::reschedule_task(self, task_id, delay_until, now, error)
    }
    fn resolve_approval_row(
        &self,
        id: &ApprovalId,
        decision: &str,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        SqliteStore::resolve_approval_row(self, id, decision, now)
    }
    fn run_task_now(&self, task_id: &TaskId, now: DateTime<Utc>) -> Result<bool> {
        SqliteStore::run_task_now(self, task_id, now)
    }
    fn running_child_count(&self, parent: &SessionId) -> Result<u32> {
        SqliteStore::running_child_count(self, parent)
    }
    fn search_transcripts(
        &self,
        query: &str,
        limit: usize,
        exclude: Option<&SessionId>,
        user_id: Option<&str>,
    ) -> Result<Vec<SearchMatch>> {
        SqliteStore::search_transcripts(self, query, limit, exclude, user_id)
    }
    fn search_transcripts_page(
        &self,
        query: &str,
        limit: usize,
        exclude: Option<&SessionId>,
        user_id: Option<&str>,
        cursor: Option<&(String, i64)>,
    ) -> Result<(Vec<SearchMatch>, Option<(String, i64)>)> {
        SqliteStore::search_transcripts_page(self, query, limit, exclude, user_id, cursor)
    }
    fn session_message_previews(
        &self,
        session_ids: &[SessionId],
    ) -> Result<Vec<(SessionId, u64, Option<String>)>> {
        SqliteStore::session_message_previews(self, session_ids)
    }
    fn session_parent(&self, session_id: &SessionId) -> Result<Option<SessionId>> {
        SqliteStore::session_parent(self, session_id)
    }
    fn session_user_id(&self, session_id: &SessionId) -> Result<Option<String>> {
        SqliteStore::session_user_id(self, session_id)
    }
    fn set_session_setting(&self, session_id: &SessionId, key: &str, value: &str) -> Result<()> {
        SqliteStore::set_session_setting(self, session_id, key, value)
    }
    fn set_session_user(&self, session_id: &SessionId, user_id: &str) -> Result<()> {
        SqliteStore::set_session_user(self, session_id, user_id)
    }
    fn set_task_attempt(&self, task_id: &TaskId, attempt: u32, now: DateTime<Utc>) -> Result<()> {
        SqliteStore::set_task_attempt(self, task_id, attempt, now)
    }
    fn set_team_task_status(
        &self,
        id: &blora_types::TeamTaskId,
        status: &str,
        column_id: Option<&str>,
        assignee: Option<&str>,
        blocked_reason: Option<&str>,
        review_state: Option<&str>,
    ) -> Result<bool> {
        SqliteStore::set_team_task_status(
            self,
            id,
            status,
            column_id,
            assignee,
            blocked_reason,
            review_state,
        )
    }
    fn set_tui_message_roles(&self, enabled: bool) -> Result<()> {
        SqliteStore::set_tui_message_roles(self, enabled)
    }
    fn set_tui_reduce_motion(&self, enabled: bool) -> Result<()> {
        SqliteStore::set_tui_reduce_motion(self, enabled)
    }
    fn take_steers(&self, session_id: &SessionId) -> Result<Vec<String>> {
        SqliteStore::take_steers(self, session_id)
    }
    fn tui_message_roles(&self) -> Result<bool> {
        SqliteStore::tui_message_roles(self)
    }
    fn tui_reduce_motion(&self) -> Result<bool> {
        SqliteStore::tui_reduce_motion(self)
    }
    fn update_agent(
        &self,
        agent_id: &AgentId,
        status: &str,
        summary: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        SqliteStore::update_agent(self, agent_id, status, summary, now)
    }
    fn update_artifact(
        &self,
        id: &ArtifactId,
        title: Option<&str>,
        content: Option<&str>,
        path: Option<&str>,
        mime: Option<&str>,
        size: Option<u64>,
        sha256: Option<&str>,
        metadata: Option<&str>,
    ) -> Result<bool> {
        SqliteStore::update_artifact(self, id, title, content, path, mime, size, sha256, metadata)
    }
    fn update_goal(
        &self,
        id: &blora_types::GoalId,
        title: Option<&str>,
        description: Option<&str>,
        status: Option<&str>,
        priority: Option<i64>,
        progress: Option<i64>,
        due_at: Option<DateTime<Utc>>,
    ) -> Result<bool> {
        SqliteStore::update_goal(
            self,
            id,
            title,
            description,
            status,
            priority,
            progress,
            due_at,
        )
    }
    fn update_passport_tokens(
        &self,
        username: &str,
        app_token: &str,
        refresh_token: Option<&str>,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> Result<Option<UserRecord>> {
        SqliteStore::update_passport_tokens(
            self,
            username,
            app_token,
            refresh_token,
            token_expires_at,
        )
    }
    fn update_task_fields(
        &self,
        task_id: &TaskId,
        title: Option<&str>,
        prompt: Option<&str>,
        cron: Option<&str>,
        max_attempts: Option<u32>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        SqliteStore::update_task_fields(self, task_id, title, prompt, cron, max_attempts, now)
    }
    fn update_workflow_run(
        &self,
        id: &blora_types::WorkflowRunId,
        status: &str,
        current_step: i64,
        state_json: &str,
        finished: bool,
    ) -> Result<bool> {
        SqliteStore::update_workflow_run(self, id, status, current_step, state_json, finished)
    }
    fn upsert_memory(
        &self,
        workspace_path: &str,
        key: &str,
        value: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        SqliteStore::upsert_memory(self, workspace_path, key, value, now)
    }
    fn upsert_passport_user(
        &self,
        username: &str,
        nickname: Option<&str>,
        avatar: Option<&str>,
        email: Option<&str>,
        app_token: Option<&str>,
        refresh_token: Option<&str>,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> Result<UserRecord> {
        SqliteStore::upsert_passport_user(
            self,
            username,
            nickname,
            avatar,
            email,
            app_token,
            refresh_token,
            token_expires_at,
        )
    }
    fn usage_activity(&self) -> Result<UsageActivity> {
        SqliteStore::usage_activity(self)
    }
    fn usage_totals(&self, session_id: Option<&SessionId>) -> Result<UsageTotals> {
        SqliteStore::usage_totals(self, session_id)
    }
    fn user_by_passport_username(&self, username: &str) -> Result<Option<UserRecord>> {
        SqliteStore::user_by_passport_username(self, username)
    }
    fn user_by_token(&self, token: &str) -> Result<Option<UserRecord>> {
        SqliteStore::user_by_token(self, token)
    }
}

/// An isolated in-memory SQLite runtime store. It shares exactly the same
/// schema, transaction and replay behavior as the file-backed adapter.
pub struct InMemoryStore {
    inner: SqliteStore,
}

impl InMemoryStore {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: SqliteStore::open_in_memory()?,
        })
    }

    #[must_use]
    pub fn inner(&self) -> &SqliteStore {
        &self.inner
    }
}

impl RuntimeStore for InMemoryStore {
    fn add_team_attachment(
        &self,
        task_id: &blora_types::TeamTaskId,
        name: &str,
        mime: &str,
        size: i64,
        sha256: &str,
        path: &str,
    ) -> Result<()> {
        SqliteStore::add_team_attachment(&self.inner, task_id, name, mime, size, sha256, path)
    }
    fn add_team_comment(
        &self,
        task_id: &blora_types::TeamTaskId,
        author: &str,
        body: &str,
    ) -> Result<()> {
        SqliteStore::add_team_comment(&self.inner, task_id, author, body)
    }
    fn add_team_link(
        &self,
        task_id: &blora_types::TeamTaskId,
        kind: &str,
        target: &str,
    ) -> Result<()> {
        SqliteStore::add_team_link(&self.inner, task_id, kind, target)
    }
    fn append(&self, new: NewEvent) -> Result<EventEnvelope> {
        SqliteStore::append(&self.inner, new)
    }
    fn cas_task_status(
        &self,
        task_id: &TaskId,
        from: &[TaskStatus],
        to: TaskStatus,
        now: DateTime<Utc>,
        error: Option<&str>,
        run_id: Option<&RunId>,
    ) -> Result<bool> {
        SqliteStore::cas_task_status(&self.inner, task_id, from, to, now, error, run_id)
    }
    fn claim_due_tasks(&self, now: DateTime<Utc>) -> Result<Vec<TaskRecord>> {
        SqliteStore::claim_due_tasks(&self.inner, now)
    }
    fn clear_passport_users(&self) -> Result<usize> {
        SqliteStore::clear_passport_users(&self.inner)
    }
    fn complete_onboarding(&self, surface: &str, version: u32) -> Result<()> {
        SqliteStore::complete_onboarding(&self.inner, surface, version)
    }
    fn create_session(&self, spec: CreateSession) -> Result<SessionId> {
        SqliteStore::create_session(&self.inner, spec)
    }
    fn create_user(&self, name: &str) -> Result<(UserRecord, String)> {
        SqliteStore::create_user(&self.inner, name)
    }
    fn delete_artifact(&self, id: &ArtifactId) -> Result<bool> {
        SqliteStore::delete_artifact(&self.inner, id)
    }
    fn delete_memory(&self, workspace_path: &str, key: &str) -> Result<bool> {
        SqliteStore::delete_memory(&self.inner, workspace_path, key)
    }
    fn delete_task(&self, task_id: &TaskId) -> Result<bool> {
        SqliteStore::delete_task(&self.inner, task_id)
    }
    fn delete_team(&self, id: &blora_types::TeamId) -> Result<bool> {
        SqliteStore::delete_team(&self.inner, id)
    }
    fn delete_team_task(&self, id: &blora_types::TeamTaskId) -> Result<bool> {
        SqliteStore::delete_team_task(&self.inner, id)
    }
    fn delete_workflow(&self, id: &blora_types::WorkflowId) -> Result<bool> {
        SqliteStore::delete_workflow(&self.inner, id)
    }
    fn enable_session_tool(
        &self,
        session_id: &SessionId,
        tool_name: &str,
        source: &str,
    ) -> Result<()> {
        SqliteStore::enable_session_tool(&self.inner, session_id, tool_name, source)
    }
    fn get_agent(&self, agent_id: &AgentId) -> Result<AgentRecord> {
        SqliteStore::get_agent(&self.inner, agent_id)
    }
    fn get_approval(&self, id: &ApprovalId) -> Result<ApprovalRecord> {
        SqliteStore::get_approval(&self.inner, id)
    }
    fn get_goal(&self, id: &blora_types::GoalId) -> Result<Option<GoalRecord>> {
        SqliteStore::get_goal(&self.inner, id)
    }
    fn get_memory(&self, workspace_path: &str, key: &str) -> Result<Option<MemoryRecord>> {
        SqliteStore::get_memory(&self.inner, workspace_path, key)
    }
    fn get_session_setting(&self, session_id: &SessionId, key: &str) -> Result<Option<String>> {
        SqliteStore::get_session_setting(&self.inner, session_id, key)
    }
    fn get_task(&self, task_id: &TaskId) -> Result<TaskRecord> {
        SqliteStore::get_task(&self.inner, task_id)
    }
    fn get_team(&self, id: &blora_types::TeamId) -> Result<Option<TeamRecord>> {
        SqliteStore::get_team(&self.inner, id)
    }
    fn get_team_task(&self, id: &blora_types::TeamTaskId) -> Result<Option<TeamTaskRecord>> {
        SqliteStore::get_team_task(&self.inner, id)
    }
    fn get_workflow(&self, id: &blora_types::WorkflowId) -> Result<Option<WorkflowRecord>> {
        SqliteStore::get_workflow(&self.inner, id)
    }
    fn heartbeat_team_task(&self, id: &blora_types::TeamTaskId) -> Result<bool> {
        SqliteStore::heartbeat_team_task(&self.inner, id)
    }
    fn insert_agent(&self, record: &AgentRecord) -> Result<()> {
        SqliteStore::insert_agent(&self.inner, record)
    }
    fn insert_approval(&self, record: &ApprovalRecord) -> Result<()> {
        SqliteStore::insert_approval(&self.inner, record)
    }
    fn insert_artifact(
        &self,
        id: &blora_types::ArtifactId,
        session_id: &SessionId,
        kind: &str,
        path: Option<&str>,
        content: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        SqliteStore::insert_artifact(&self.inner, id, session_id, kind, path, content, now)
    }
    fn insert_artifact_record(&self, record: &ArtifactRecord) -> Result<()> {
        SqliteStore::insert_artifact_record(&self.inner, record)
    }
    fn insert_checkpoint(
        &self,
        session_id: &SessionId,
        run_id: Option<&RunId>,
        sequence: u64,
        note: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        SqliteStore::insert_checkpoint(&self.inner, session_id, run_id, sequence, note, now)
    }
    fn insert_goal(&self, goal: &GoalRecord) -> Result<()> {
        SqliteStore::insert_goal(&self.inner, goal)
    }
    fn insert_task(&self, id: &TaskId, spec: &CreateTask, now: DateTime<Utc>) -> Result<()> {
        SqliteStore::insert_task(&self.inner, id, spec, now)
    }
    fn insert_team(&self, team: &TeamRecord) -> Result<()> {
        SqliteStore::insert_team(&self.inner, team)
    }
    fn insert_team_column(&self, column: &TeamColumnRecord) -> Result<()> {
        SqliteStore::insert_team_column(&self.inner, column)
    }
    fn insert_team_task(&self, task: &TeamTaskRecord) -> Result<()> {
        SqliteStore::insert_team_task(&self.inner, task)
    }
    fn insert_workflow(&self, workflow: &WorkflowRecord) -> Result<()> {
        SqliteStore::insert_workflow(&self.inner, workflow)
    }
    fn insert_workflow_run(&self, run: &WorkflowRunRecord) -> Result<()> {
        SqliteStore::insert_workflow_run(&self.inner, run)
    }
    fn last_sequence(&self, session_id: &SessionId) -> Result<u64> {
        SqliteStore::last_sequence(&self.inner, session_id)
    }
    fn latest_workflow_run(
        &self,
        workflow_id: &blora_types::WorkflowId,
    ) -> Result<Option<WorkflowRunRecord>> {
        SqliteStore::latest_workflow_run(&self.inner, workflow_id)
    }
    fn list_agents(&self, parent: &SessionId) -> Result<Vec<AgentRecord>> {
        SqliteStore::list_agents(&self.inner, parent)
    }
    fn list_all_pending_approvals(&self) -> Result<Vec<ApprovalRecord>> {
        SqliteStore::list_all_pending_approvals(&self.inner)
    }
    fn list_artifacts(&self, session_id: Option<&SessionId>) -> Result<Vec<ArtifactRecord>> {
        SqliteStore::list_artifacts(&self.inner, session_id)
    }
    fn get_artifact(&self, id: &ArtifactId) -> Result<Option<ArtifactRecord>> {
        SqliteStore::get_artifact(&self.inner, id)
    }
    fn list_goals(&self, session_id: &SessionId) -> Result<Vec<GoalRecord>> {
        SqliteStore::list_goals(&self.inner, session_id)
    }
    fn list_memories(&self, workspace_path: &str) -> Result<Vec<MemoryRecord>> {
        SqliteStore::list_memories(&self.inner, workspace_path)
    }
    fn list_pending_approvals(&self, session_id: &SessionId) -> Result<Vec<ApprovalRecord>> {
        SqliteStore::list_pending_approvals(&self.inner, session_id)
    }
    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        SqliteStore::list_sessions(&self.inner)
    }
    fn list_session_tools(&self, session_id: &SessionId) -> Result<Vec<String>> {
        SqliteStore::list_session_tools(&self.inner, session_id)
    }
    fn list_sessions_for_user(&self, user_id: Option<&str>) -> Result<Vec<crate::SessionSummary>> {
        SqliteStore::list_sessions_for_user(&self.inner, user_id)
    }
    fn list_tasks(&self, session_id: Option<&SessionId>) -> Result<Vec<TaskRecord>> {
        SqliteStore::list_tasks(&self.inner, session_id)
    }
    fn list_team_attachments(
        &self,
        task_id: &blora_types::TeamTaskId,
    ) -> Result<Vec<TeamAttachmentRecord>> {
        SqliteStore::list_team_attachments(&self.inner, task_id)
    }
    fn list_team_columns(&self, team_id: &blora_types::TeamId) -> Result<Vec<TeamColumnRecord>> {
        SqliteStore::list_team_columns(&self.inner, team_id)
    }
    fn list_team_comments(
        &self,
        task_id: &blora_types::TeamTaskId,
    ) -> Result<Vec<(String, String, DateTime<Utc>)>> {
        SqliteStore::list_team_comments(&self.inner, task_id)
    }
    fn list_team_links(&self, task_id: &blora_types::TeamTaskId) -> Result<Vec<TeamLinkRecord>> {
        SqliteStore::list_team_links(&self.inner, task_id)
    }
    fn list_team_tasks(&self, team_id: &blora_types::TeamId) -> Result<Vec<TeamTaskRecord>> {
        SqliteStore::list_team_tasks(&self.inner, team_id)
    }
    fn list_teams(&self, workspace_path: &str) -> Result<Vec<TeamRecord>> {
        SqliteStore::list_teams(&self.inner, workspace_path)
    }
    fn list_users(&self) -> Result<Vec<UserRecord>> {
        SqliteStore::list_users(&self.inner)
    }
    fn list_workflows(&self, session_id: &SessionId) -> Result<Vec<WorkflowRecord>> {
        SqliteStore::list_workflows(&self.inner, session_id)
    }
    fn load_events(&self, session_id: &SessionId) -> Result<Vec<EventEnvelope>> {
        SqliteStore::load_events(&self.inner, session_id)
    }
    fn load_events_after(&self, session_id: &SessionId, after: u64) -> Result<Vec<EventEnvelope>> {
        SqliteStore::load_events_after(&self.inner, session_id, after)
    }
    fn load_projection(&self, session_id: &SessionId) -> Result<SessionProjection> {
        SqliteStore::load_projection(&self.inner, session_id)
    }
    fn mark_agent_delivered(&self, agent_id: &AgentId) -> Result<()> {
        SqliteStore::mark_agent_delivered(&self.inner, agent_id)
    }
    fn onboarding_version(&self, surface: &str) -> Result<u32> {
        SqliteStore::onboarding_version(&self.inner, surface)
    }
    fn queue_steer(&self, session_id: &SessionId, message: &str) -> Result<()> {
        SqliteStore::queue_steer(&self.inner, session_id, message)
    }
    fn reschedule_task(
        &self,
        task_id: &TaskId,
        delay_until: DateTime<Utc>,
        now: DateTime<Utc>,
        error: &str,
    ) -> Result<()> {
        SqliteStore::reschedule_task(&self.inner, task_id, delay_until, now, error)
    }
    fn resolve_approval_row(
        &self,
        id: &ApprovalId,
        decision: &str,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        SqliteStore::resolve_approval_row(&self.inner, id, decision, now)
    }
    fn run_task_now(&self, task_id: &TaskId, now: DateTime<Utc>) -> Result<bool> {
        SqliteStore::run_task_now(&self.inner, task_id, now)
    }
    fn running_child_count(&self, parent: &SessionId) -> Result<u32> {
        SqliteStore::running_child_count(&self.inner, parent)
    }
    fn search_transcripts(
        &self,
        query: &str,
        limit: usize,
        exclude: Option<&SessionId>,
        user_id: Option<&str>,
    ) -> Result<Vec<SearchMatch>> {
        SqliteStore::search_transcripts(&self.inner, query, limit, exclude, user_id)
    }
    fn search_transcripts_page(
        &self,
        query: &str,
        limit: usize,
        exclude: Option<&SessionId>,
        user_id: Option<&str>,
        cursor: Option<&(String, i64)>,
    ) -> Result<(Vec<SearchMatch>, Option<(String, i64)>)> {
        SqliteStore::search_transcripts_page(&self.inner, query, limit, exclude, user_id, cursor)
    }
    fn session_message_previews(
        &self,
        session_ids: &[SessionId],
    ) -> Result<Vec<(SessionId, u64, Option<String>)>> {
        SqliteStore::session_message_previews(&self.inner, session_ids)
    }
    fn session_parent(&self, session_id: &SessionId) -> Result<Option<SessionId>> {
        SqliteStore::session_parent(&self.inner, session_id)
    }
    fn session_user_id(&self, session_id: &SessionId) -> Result<Option<String>> {
        SqliteStore::session_user_id(&self.inner, session_id)
    }
    fn set_session_setting(&self, session_id: &SessionId, key: &str, value: &str) -> Result<()> {
        SqliteStore::set_session_setting(&self.inner, session_id, key, value)
    }
    fn set_session_user(&self, session_id: &SessionId, user_id: &str) -> Result<()> {
        SqliteStore::set_session_user(&self.inner, session_id, user_id)
    }
    fn set_task_attempt(&self, task_id: &TaskId, attempt: u32, now: DateTime<Utc>) -> Result<()> {
        SqliteStore::set_task_attempt(&self.inner, task_id, attempt, now)
    }
    fn set_team_task_status(
        &self,
        id: &blora_types::TeamTaskId,
        status: &str,
        column_id: Option<&str>,
        assignee: Option<&str>,
        blocked_reason: Option<&str>,
        review_state: Option<&str>,
    ) -> Result<bool> {
        SqliteStore::set_team_task_status(
            &self.inner,
            id,
            status,
            column_id,
            assignee,
            blocked_reason,
            review_state,
        )
    }
    fn set_tui_message_roles(&self, enabled: bool) -> Result<()> {
        SqliteStore::set_tui_message_roles(&self.inner, enabled)
    }
    fn set_tui_reduce_motion(&self, enabled: bool) -> Result<()> {
        SqliteStore::set_tui_reduce_motion(&self.inner, enabled)
    }
    fn take_steers(&self, session_id: &SessionId) -> Result<Vec<String>> {
        SqliteStore::take_steers(&self.inner, session_id)
    }
    fn tui_message_roles(&self) -> Result<bool> {
        SqliteStore::tui_message_roles(&self.inner)
    }
    fn tui_reduce_motion(&self) -> Result<bool> {
        SqliteStore::tui_reduce_motion(&self.inner)
    }
    fn update_agent(
        &self,
        agent_id: &AgentId,
        status: &str,
        summary: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        SqliteStore::update_agent(&self.inner, agent_id, status, summary, now)
    }
    fn update_artifact(
        &self,
        id: &ArtifactId,
        title: Option<&str>,
        content: Option<&str>,
        path: Option<&str>,
        mime: Option<&str>,
        size: Option<u64>,
        sha256: Option<&str>,
        metadata: Option<&str>,
    ) -> Result<bool> {
        SqliteStore::update_artifact(
            &self.inner,
            id,
            title,
            content,
            path,
            mime,
            size,
            sha256,
            metadata,
        )
    }
    fn update_goal(
        &self,
        id: &blora_types::GoalId,
        title: Option<&str>,
        description: Option<&str>,
        status: Option<&str>,
        priority: Option<i64>,
        progress: Option<i64>,
        due_at: Option<DateTime<Utc>>,
    ) -> Result<bool> {
        SqliteStore::update_goal(
            &self.inner,
            id,
            title,
            description,
            status,
            priority,
            progress,
            due_at,
        )
    }
    fn update_passport_tokens(
        &self,
        username: &str,
        app_token: &str,
        refresh_token: Option<&str>,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> Result<Option<UserRecord>> {
        SqliteStore::update_passport_tokens(
            &self.inner,
            username,
            app_token,
            refresh_token,
            token_expires_at,
        )
    }
    fn update_task_fields(
        &self,
        task_id: &TaskId,
        title: Option<&str>,
        prompt: Option<&str>,
        cron: Option<&str>,
        max_attempts: Option<u32>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        SqliteStore::update_task_fields(
            &self.inner,
            task_id,
            title,
            prompt,
            cron,
            max_attempts,
            now,
        )
    }
    fn update_workflow_run(
        &self,
        id: &blora_types::WorkflowRunId,
        status: &str,
        current_step: i64,
        state_json: &str,
        finished: bool,
    ) -> Result<bool> {
        SqliteStore::update_workflow_run(
            &self.inner,
            id,
            status,
            current_step,
            state_json,
            finished,
        )
    }
    fn upsert_memory(
        &self,
        workspace_path: &str,
        key: &str,
        value: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        SqliteStore::upsert_memory(&self.inner, workspace_path, key, value, now)
    }
    fn upsert_passport_user(
        &self,
        username: &str,
        nickname: Option<&str>,
        avatar: Option<&str>,
        email: Option<&str>,
        app_token: Option<&str>,
        refresh_token: Option<&str>,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> Result<UserRecord> {
        SqliteStore::upsert_passport_user(
            &self.inner,
            username,
            nickname,
            avatar,
            email,
            app_token,
            refresh_token,
            token_expires_at,
        )
    }
    fn usage_activity(&self) -> Result<UsageActivity> {
        SqliteStore::usage_activity(&self.inner)
    }
    fn usage_totals(&self, session_id: Option<&SessionId>) -> Result<UsageTotals> {
        SqliteStore::usage_totals(&self.inner, session_id)
    }
    fn user_by_passport_username(&self, username: &str) -> Result<Option<UserRecord>> {
        SqliteStore::user_by_passport_username(&self.inner, username)
    }
    fn user_by_token(&self, token: &str) -> Result<Option<UserRecord>> {
        SqliteStore::user_by_token(&self.inner, token)
    }
}
