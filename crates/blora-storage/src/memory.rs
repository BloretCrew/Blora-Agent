// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Isolated in-memory runtime store.

use crate::{RuntimeStore, SqliteStore};
use blora_types::Result;

/// A private in-memory SQLite database implementing the complete runtime store contract.
/// Each instance owns an independent connection and disappears when dropped.
pub struct InMemoryStore {
    inner: SqliteStore,
}

impl InMemoryStore {
    pub fn new() -> Result<Self> {
        Ok(Self { inner: SqliteStore::open_in_memory()? })
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
        self.inner.add_team_attachment(task_id, name, mime, size, sha256, path)
    }
    fn add_team_comment(
        &self,
        task_id: &blora_types::TeamTaskId,
        author: &str,
        body: &str,
    ) -> Result<()> {
        self.inner.add_team_comment(task_id, author, body)
    }
    fn add_team_link(
        &self,
        task_id: &blora_types::TeamTaskId,
        kind: &str,
        target: &str,
    ) -> Result<()> {
        self.inner.add_team_link(task_id, kind, target)
    }
    fn append(&self, new: NewEvent) -> Result<EventEnvelope> {
        self.inner.append(new)
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
        self.inner.cas_task_status(task_id, from, to, now, error, run_id)
    }
    fn claim_due_tasks(&self, now: DateTime<Utc>) -> Result<Vec<TaskRecord>> {
        self.inner.claim_due_tasks(now)
    }
    fn clear_passport_users(&self) -> Result<usize> {
        self.inner.clear_passport_users()
    }
    fn complete_onboarding(&self, surface: &str, version: u32) -> Result<()> {
        self.inner.complete_onboarding(surface, version)
    }
    fn create_session(&self, spec: CreateSession) -> Result<SessionId> {
        self.inner.create_session(spec)
    }
    fn create_user(&self, name: &str) -> Result<(UserRecord, String)> {
        self.inner.create_user(name)
    }
    fn delete_artifact(&self, id: &ArtifactId) -> Result<bool> {
        self.inner.delete_artifact(id)
    }
    fn delete_memory(&self, workspace_path: &str, key: &str) -> Result<bool> {
        self.inner.delete_memory(workspace_path, key)
    }
    fn delete_task(&self, task_id: &TaskId) -> Result<bool> {
        self.inner.delete_task(task_id)
    }
    fn delete_team(&self, id: &blora_types::TeamId) -> Result<bool> {
        self.inner.delete_team(id)
    }
    fn delete_team_task(&self, id: &blora_types::TeamTaskId) -> Result<bool> {
        self.inner.delete_team_task(id)
    }
    fn delete_workflow(&self, id: &blora_types::WorkflowId) -> Result<bool> {
        self.inner.delete_workflow(id)
    }
    fn enable_session_tool(
        &self,
        session_id: &SessionId,
        tool_name: &str,
        source: &str,
    ) -> Result<()> {
        self.inner.enable_session_tool(session_id, tool_name, source)
    }
    fn get_agent(&self, agent_id: &AgentId) -> Result<AgentRecord> {
        self.inner.get_agent(agent_id)
    }
    fn get_approval(&self, id: &ApprovalId) -> Result<ApprovalRecord> {
        self.inner.get_approval(id)
    }
    fn get_goal(&self, id: &blora_types::GoalId) -> Result<Option<GoalRecord>> {
        self.inner.get_goal(id)
    }
    fn get_memory(&self, workspace_path: &str, key: &str) -> Result<Option<MemoryRecord>> {
        self.inner.get_memory(workspace_path, key)
    }
    fn get_session_setting(&self, session_id: &SessionId, key: &str) -> Result<Option<String>> {
        self.inner.get_session_setting(session_id, key)
    }
    fn get_task(&self, task_id: &TaskId) -> Result<TaskRecord> {
        self.inner.get_task(task_id)
    }
    fn get_team(&self, id: &blora_types::TeamId) -> Result<Option<TeamRecord>> {
        self.inner.get_team(id)
    }
    fn get_team_task(&self, id: &blora_types::TeamTaskId) -> Result<Option<TeamTaskRecord>> {
        self.inner.get_team_task(id)
    }
    fn get_workflow(&self, id: &blora_types::WorkflowId) -> Result<Option<WorkflowRecord>> {
        self.inner.get_workflow(id)
    }
    fn heartbeat_team_task(&self, id: &blora_types::TeamTaskId) -> Result<bool> {
        self.inner.heartbeat_team_task(id)
    }
    fn insert_agent(&self, record: &AgentRecord) -> Result<()> {
        self.inner.insert_agent(record)
    }
    fn insert_approval(&self, record: &ApprovalRecord) -> Result<()> {
        self.inner.insert_approval(record)
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
        self.inner.insert_artifact(id, session_id, kind, path, content, now)
    }
    fn insert_artifact_record(&self, record: &ArtifactRecord) -> Result<()> {
        self.inner.insert_artifact_record(record)
    }
    fn insert_checkpoint(
        &self,
        session_id: &SessionId,
        run_id: Option<&RunId>,
        sequence: u64,
        note: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.inner.insert_checkpoint(session_id, run_id, sequence, note, now)
    }
    fn insert_goal(&self, goal: &GoalRecord) -> Result<()> {
        self.inner.insert_goal(goal)
    }
    fn insert_task(&self, id: &TaskId, spec: &CreateTask, now: DateTime<Utc>) -> Result<()> {
        self.inner.insert_task(id, spec, now)
    }
    fn insert_team(&self, team: &TeamRecord) -> Result<()> {
        self.inner.insert_team(team)
    }
    fn insert_team_column(&self, column: &TeamColumnRecord) -> Result<()> {
        self.inner.insert_team_column(column)
    }
    fn insert_team_task(&self, task: &TeamTaskRecord) -> Result<()> {
        self.inner.insert_team_task(task)
    }
    fn insert_workflow(&self, workflow: &WorkflowRecord) -> Result<()> {
        self.inner.insert_workflow(workflow)
    }
    fn insert_workflow_run(&self, run: &WorkflowRunRecord) -> Result<()> {
        self.inner.insert_workflow_run(run)
    }
    fn last_sequence(&self, session_id: &SessionId) -> Result<u64> {
        self.inner.last_sequence(session_id)
    }
    fn latest_workflow_run(
        &self,
        workflow_id: &blora_types::WorkflowId,
    ) -> Result<Option<WorkflowRunRecord>> {
        self.inner.latest_workflow_run(workflow_id)
    }
    fn list_agents(&self, parent: &SessionId) -> Result<Vec<AgentRecord>> {
        self.inner.list_agents(parent)
    }
    fn list_all_pending_approvals(&self) -> Result<Vec<ApprovalRecord>> {
        self.inner.list_all_pending_approvals()
    }
    fn list_artifacts(&self, session_id: Option<&SessionId>) -> Result<Vec<ArtifactRecord>> {
        self.inner.list_artifacts(session_id)
    }
    fn get_artifact(&self, id: &ArtifactId) -> Result<Option<ArtifactRecord>> {
        self.inner.get_artifact(id)
    }
    fn list_goals(&self, session_id: &SessionId) -> Result<Vec<GoalRecord>> {
        self.inner.list_goals(session_id)
    }
    fn list_memories(&self, workspace_path: &str) -> Result<Vec<MemoryRecord>> {
        self.inner.list_memories(workspace_path)
    }
    fn list_pending_approvals(&self, session_id: &SessionId) -> Result<Vec<ApprovalRecord>> {
        self.inner.list_pending_approvals(session_id)
    }
    fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        self.inner.list_sessions()
    }
    fn list_session_tools(&self, session_id: &SessionId) -> Result<Vec<String>> {
        self.inner.list_session_tools(session_id)
    }
    fn list_sessions_for_user(&self, user_id: Option<&str>) -> Result<Vec<crate::SessionSummary>> {
        self.inner.list_sessions_for_user(user_id)
    }
    fn list_tasks(&self, session_id: Option<&SessionId>) -> Result<Vec<TaskRecord>> {
        self.inner.list_tasks(session_id)
    }
    fn list_team_attachments(
        &self,
        task_id: &blora_types::TeamTaskId,
    ) -> Result<Vec<TeamAttachmentRecord>> {
        self.inner.list_team_attachments(task_id)
    }
    fn list_team_columns(&self, team_id: &blora_types::TeamId) -> Result<Vec<TeamColumnRecord>> {
        self.inner.list_team_columns(team_id)
    }
    fn list_team_comments(
        &self,
        task_id: &blora_types::TeamTaskId,
    ) -> Result<Vec<(String, String, DateTime<Utc>)>> {
        self.inner.list_team_comments(task_id)
    }
    fn list_team_links(&self, task_id: &blora_types::TeamTaskId) -> Result<Vec<TeamLinkRecord>> {
        self.inner.list_team_links(task_id)
    }
    fn list_team_tasks(&self, team_id: &blora_types::TeamId) -> Result<Vec<TeamTaskRecord>> {
        self.inner.list_team_tasks(team_id)
    }
    fn list_teams(&self, workspace_path: &str) -> Result<Vec<TeamRecord>> {
        self.inner.list_teams(workspace_path)
    }
    fn list_users(&self) -> Result<Vec<UserRecord>> {
        self.inner.list_users()
    }
    fn list_workflows(&self, session_id: &SessionId) -> Result<Vec<WorkflowRecord>> {
        self.inner.list_workflows(session_id)
    }
    fn load_events(&self, session_id: &SessionId) -> Result<Vec<EventEnvelope>> {
        self.inner.load_events(session_id)
    }
    fn load_events_after(&self, session_id: &SessionId, after: u64) -> Result<Vec<EventEnvelope>> {
        self.inner.load_events_after(session_id, after)
    }
    fn load_projection(&self, session_id: &SessionId) -> Result<SessionProjection> {
        self.inner.load_projection(session_id)
    }
    fn mark_agent_delivered(&self, agent_id: &AgentId) -> Result<()> {
        self.inner.mark_agent_delivered(agent_id)
    }
    fn onboarding_version(&self, surface: &str) -> Result<u32> {
        self.inner.onboarding_version(surface)
    }
    fn queue_steer(&self, session_id: &SessionId, message: &str) -> Result<()> {
        self.inner.queue_steer(session_id, message)
    }
    fn reschedule_task(
        &self,
        task_id: &TaskId,
        delay_until: DateTime<Utc>,
        now: DateTime<Utc>,
        error: &str,
    ) -> Result<()> {
        self.inner.reschedule_task(task_id, delay_until, now, error)
    }
    fn resolve_approval_row(
        &self,
        id: &ApprovalId,
        decision: &str,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        self.inner.resolve_approval_row(id, decision, now)
    }
    fn run_task_now(&self, task_id: &TaskId, now: DateTime<Utc>) -> Result<bool> {
        self.inner.run_task_now(task_id, now)
    }
    fn running_child_count(&self, parent: &SessionId) -> Result<u32> {
        self.inner.running_child_count(parent)
    }
    fn search_transcripts(
        &self,
        query: &str,
        limit: usize,
        exclude: Option<&SessionId>,
        user_id: Option<&str>,
    ) -> Result<Vec<SearchMatch>> {
        self.inner.search_transcripts(query, limit, exclude, user_id)
    }
    fn search_transcripts_page(
        &self,
        query: &str,
        limit: usize,
        exclude: Option<&SessionId>,
        user_id: Option<&str>,
        cursor: Option<&(String, i64)>,
    ) -> Result<(Vec<SearchMatch>, Option<(String, i64)>)> {
        self.inner.search_transcripts_page(query, limit, exclude, user_id, cursor)
    }
    fn session_message_previews(
        &self,
        session_ids: &[SessionId],
    ) -> Result<Vec<(SessionId, u64, Option<String>)>> {
        self.inner.session_message_previews(session_ids)
    }
    fn session_parent(&self, session_id: &SessionId) -> Result<Option<SessionId>> {
        self.inner.session_parent(session_id)
    }
    fn session_user_id(&self, session_id: &SessionId) -> Result<Option<String>> {
        self.inner.session_user_id(session_id)
    }
    fn set_session_setting(&self, session_id: &SessionId, key: &str, value: &str) -> Result<()> {
        self.inner.set_session_setting(session_id, key, value)
    }
    fn set_session_user(&self, session_id: &SessionId, user_id: &str) -> Result<()> {
        self.inner.set_session_user(session_id, user_id)
    }
    fn set_task_attempt(&self, task_id: &TaskId, attempt: u32, now: DateTime<Utc>) -> Result<()> {
        self.inner.set_task_attempt(task_id, attempt, now)
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
        self.inner.set_team_task_status(id, status, column_id, assignee, blocked_reason, review_state)
    }
    fn set_tui_message_roles(&self, enabled: bool) -> Result<()> {
        self.inner.set_tui_message_roles(enabled)
    }
    fn take_steers(&self, session_id: &SessionId) -> Result<Vec<String>> {
        self.inner.take_steers(session_id)
    }
    fn tui_message_roles(&self) -> Result<bool> {
        self.inner.tui_message_roles()
    }
    fn update_agent(
        &self,
        agent_id: &AgentId,
        status: &str,
        summary: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.inner.update_agent(agent_id, status, summary, now)
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
        self.inner.update_artifact(id, title, content, path, mime, size, sha256, metadata)
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
        self.inner.update_goal(id, title, description, status, priority, progress, due_at)
    }
    fn update_passport_tokens(
        &self,
        username: &str,
        app_token: &str,
        refresh_token: Option<&str>,
        token_expires_at: Option<DateTime<Utc>>,
    ) -> Result<Option<UserRecord>> {
        self.inner.update_passport_tokens(username, app_token, refresh_token, token_expires_at)
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
        self.inner.update_task_fields(task_id, title, prompt, cron, max_attempts, now)
    }
    fn update_workflow_run(
        &self,
        id: &blora_types::WorkflowRunId,
        status: &str,
        current_step: i64,
        state_json: &str,
        finished: bool,
    ) -> Result<bool> {
        self.inner.update_workflow_run(id, status, current_step, state_json, finished)
    }
    fn upsert_memory(
        &self,
        workspace_path: &str,
        key: &str,
        value: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.inner.upsert_memory(workspace_path, key, value, now)
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
        self.inner.upsert_passport_user(username, nickname, avatar, email, app_token, refresh_token, token_expires_at)
    }
    fn usage_activity(&self) -> Result<UsageActivity> {
        self.inner.usage_activity()
    }
    fn usage_totals(&self, session_id: Option<&SessionId>) -> Result<UsageTotals> {
        self.inner.usage_totals(session_id)
    }
    fn user_by_passport_username(&self, username: &str) -> Result<Option<UserRecord>> {
        self.inner.user_by_passport_username(username)
    }
    fn user_by_token(&self, token: &str) -> Result<Option<UserRecord>> {
        self.inner.user_by_token(token)
    }
}
