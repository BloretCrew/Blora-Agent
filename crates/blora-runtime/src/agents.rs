// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::path::{Path, PathBuf};

use blora_events::{
    KnownPayload, SubagentCompleted, SubagentFailed, SubagentMessage, SubagentSpawned,
};
use blora_storage::{AgentRecord, CreateSession};
use blora_types::{AgentId, BloraError, CancelToken, Mode, Result, SessionId};
use chrono::Utc;
use serde_json::Value;

use crate::coordinator::{RunOptions, Runtime};

pub const MAX_SUBAGENT_DEPTH: u32 = 2;
pub const MAX_RUNNING_CHILDREN: u32 = 3;
pub const SUBAGENT_TURNS: u32 = 6;

struct RoleSpec {
    duty: String,
    budget: u32,
    read_only: bool,
}

impl Runtime {
    pub fn session_depth(&self, session_id: &SessionId) -> Result<u32> {
        let mut depth = 0;
        let mut current = self.store.session_parent(session_id)?;
        while let Some(parent) = current {
            depth += 1;
            if depth > 8 {
                break;
            }
            current = self.store.session_parent(&parent)?;
        }
        Ok(depth)
    }

    pub fn list_subagents(&self, session_id: &SessionId) -> Result<Vec<AgentRecord>> {
        self.store.list_agents(session_id)
    }

    pub fn get_subagent(&self, agent_id: &AgentId) -> Result<AgentRecord> {
        self.store.get_agent(agent_id)
    }

    /// Converge child agents left `running` by a dead process.
    ///
    /// Runs before a new child is admitted so a crash cannot permanently
    /// occupy the per-parent concurrency cap. A child whose last run reached a
    /// terminal state keeps that outcome; anything else is reported as
    /// `interrupted`. Idempotent and safe to call from several threads: the
    /// transition is a conditional update, so only the winner emits an event.
    ///
    /// A row is only reclaimed when its owning process is gone. TUI and the
    /// local Web server share one database, so a row driven by the other
    /// process must be left alone.
    pub fn reconcile_stale_agents(&self) -> Result<usize> {
        let mut converged = 0;
        for agent in self.store.list_running_agents()? {
            let live_here = self
                .subagent_cancels
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains_key(&agent.id);
            if live_here || agent.owner_pid.is_some_and(process_is_alive) {
                continue;
            }
            let projection = self.show_session(&agent.child_session_id).ok();
            let run_status = projection
                .as_ref()
                .and_then(|projection| projection.runs.last())
                .map(|run| run.status);
            let (status, fallback) = match run_status {
                Some(blora_types::RunStatus::Completed) => ("completed", "completed"),
                Some(blora_types::RunStatus::Cancelled) => ("cancelled", "cancelled"),
                Some(blora_types::RunStatus::Failed) => ("failed", "failed"),
                _ => ("failed", "interrupted"),
            };
            // Rebuild the report from the child's own event log instead of a
            // placeholder, so a recovered completion still tells the parent
            // what the child did and what it cost.
            let child_report = projection.as_ref().and_then(|projection| {
                projection.transcript.iter().rev().find_map(|item| {
                    if let blora_session::TranscriptItem::Assistant { text, .. } = item {
                        Some(text.clone())
                    } else {
                        None
                    }
                })
            });
            let summary = agent
                .summary
                .as_deref()
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
                .or(child_report)
                .unwrap_or_else(|| fallback.to_owned());
            let (input_tokens, output_tokens, turns) = projection
                .as_ref()
                .map(|projection| {
                    let turns = projection
                        .transcript
                        .iter()
                        .filter(|item| {
                            matches!(item, blora_session::TranscriptItem::Assistant { .. })
                        })
                        .count() as u32;
                    (projection.input_tokens, projection.output_tokens, turns)
                })
                .unwrap_or_default();
            let payload = if status == "completed" {
                KnownPayload::SubagentCompleted(SubagentCompleted {
                    agent_id: agent.id.clone(),
                    summary: Some(summary.clone()),
                    input_tokens,
                    output_tokens,
                    turns,
                })
            } else {
                KnownPayload::SubagentFailed(SubagentFailed {
                    agent_id: agent.id.clone(),
                    error: summary.clone(),
                })
            };
            // The conditional update and the parent event commit together; a
            // crash between them cannot leave a terminal row with no report.
            if !self.store.finish_agent_with_event(
                &agent.id,
                status,
                Some(&summary),
                payload,
                Utc::now(),
            )? {
                continue;
            }
            converged += 1;
        }
        Ok(converged)
    }

    pub fn spawn_subagent(
        &self,
        parent_session: &SessionId,
        role: &str,
        prompt: &str,
        options: &RunOptions,
        cancel: &CancelToken,
    ) -> Result<String> {
        if prompt.trim().is_empty() {
            return Err(BloraError::Other(
                "subagent prompt must not be empty".to_owned(),
            ));
        }
        let role = normalize_role(role);
        let depth = self.session_depth(parent_session)? + 1;
        if depth > MAX_SUBAGENT_DEPTH {
            return Err(BloraError::Policy(format!(
                "subagent depth {depth} exceeds {MAX_SUBAGENT_DEPTH}"
            )));
        }
        // A previous process may have died with children still marked running.
        // Reclaim that concurrency budget before admitting a new child.
        let _ = self.reconcile_stale_agents();
        let parent = self
            .show_session(parent_session)?
            .session
            .ok_or_else(|| BloraError::SessionNotFound(parent_session.to_string()))?;
        let spec = resolve_role(&parent.workspace_path, &role);
        let child_workspace = parent.workspace_path.clone();
        let child_mode = match parent.mode {
            Mode::Imagine => Mode::Code,
            mode => mode,
        };
        let child_session = self.create_session(CreateSession {
            title: Some(format!("subagent:{role}")),
            workspace_path: child_workspace,
            // Children keep the parent's strategy (Work children can research
            // the web); Imagine has no tool loop to delegate into.
            mode: child_mode,
            parent_session_id: Some(parent_session.clone()),
            // Inherited from the parent inside the insert transaction, so a
            // delegated child is never visible without its owner.
            user_id: None,
        })?;
        let agent_id = AgentId::generate();
        let now = Utc::now();
        // Register the child token before the row becomes visible: reconcile
        // treats a running row without a live token as abandoned, so the two
        // steps must not interleave.
        let child_cancel = cancel.child();
        self.subagent_cancels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(agent_id.clone(), child_cancel.clone());
        if let Err(err) = self.store.try_insert_running_agent(
            &AgentRecord {
                id: agent_id.clone(),
                parent_session_id: parent_session.clone(),
                child_session_id: child_session.clone(),
                role: role.clone(),
                depth,
                status: "running".to_owned(),
                budget_turns: spec.budget,
                summary: None,
                owner_pid: Some(std::process::id()),
                cancel_requested: false,
                created_at: now,
                updated_at: now,
            },
            MAX_RUNNING_CHILDREN,
        ) {
            self.subagent_cancels
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&agent_id);
            // Archive through the event log so projections and the sessions
            // table agree that this child never ran.
            let _ = self.archive_session(&child_session);
            return Err(err);
        }
        let _guard = SubagentCancelGuard {
            runtime: self,
            agent_id: agent_id.clone(),
            child_session: child_session.clone(),
        };
        // Interactive approvals raised by this child surface on the parent.
        self.subagent_owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                child_session.clone(),
                (parent_session.clone(), role.clone()),
            );
        self.emit(
            parent_session,
            None,
            None,
            KnownPayload::SubagentSpawned(SubagentSpawned {
                agent_id: agent_id.clone(),
                role: role.clone(),
                child_session_id: Some(child_session.clone()),
                prompt: Some(prompt.to_owned()),
                depth,
            }),
        )?;
        let child_options = RunOptions {
            model: options.model.clone(),
            mock: options.mock,
            auto_approve: options.auto_approve && !spec.read_only,
            interactive: options.interactive,
            // Children report to the parent agent; only the parent talks to the user.
            can_ask: false,
            max_turns: spec.budget,
            provider: options.provider.clone(),
            read_only: spec.read_only,
            worktree: false,
            passport_user_token: options.passport_user_token.clone(),
            permission: if spec.read_only {
                Some(blora_policy::PermissionMode::Plan)
            } else {
                options.permission
            },
        };
        let child_prompt = subagent_brief(&role, &spec.duty, prompt, depth < MAX_SUBAGENT_DEPTH);
        let result = {
            // Another process can only ask us to stop through the database.
            // Watch that flag for the lifetime of the child run.
            let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
            let watcher = {
                let store = self.store.clone();
                let agent_id = agent_id.clone();
                let token = child_cancel.clone();
                std::thread::spawn(move || {
                    loop {
                        match stop_rx.recv_timeout(std::time::Duration::from_millis(250)) {
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                                // A busy write can briefly fail this read; the
                                // next tick retries, and a real cancel request
                                // stays visible until the child finishes.
                                if store.agent_cancel_requested(&agent_id).unwrap_or(false) {
                                    token.cancel();
                                    return;
                                }
                            }
                            _ => return,
                        }
                    }
                })
            };
            let result = self.run(&child_session, &child_prompt, &child_cancel, &child_options);
            drop(stop_tx);
            let _ = watcher.join();
            result
        };
        let control = self.store.get_agent(&agent_id).ok();
        let projection = self.show_session(&child_session).ok();
        // A run that reached a terminal state is authoritative: a parent cancel
        // racing with the last child turn must not relabel a completed child.
        let run_status = projection
            .as_ref()
            .and_then(|projection| projection.runs.last())
            .map(|run| run.status);
        let cancelled = child_was_cancelled(
            run_status,
            child_cancel.is_cancelled(),
            control.as_ref().map(|agent| agent.status.as_str()),
        );
        let last_assistant = projection.as_ref().and_then(|projection| {
            projection.transcript.iter().rev().find_map(|item| {
                if let blora_session::TranscriptItem::Assistant { text, .. } = item {
                    Some(text.clone())
                } else {
                    None
                }
            })
        });
        let handoff = self
            .subagent_handoffs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&child_session);
        let summary = handoff
            .or(last_assistant)
            .unwrap_or_else(|| "subagent finished without text".to_owned());
        let (input_tokens, output_tokens, turns) = projection
            .as_ref()
            .map(|projection| {
                let turns = projection
                    .transcript
                    .iter()
                    .filter(|item| matches!(item, blora_session::TranscriptItem::Assistant { .. }))
                    .count() as u32;
                (projection.input_tokens, projection.output_tokens, turns)
            })
            .unwrap_or_default();

        if cancelled {
            if control
                .as_ref()
                .is_none_or(|agent| agent.status == "running")
            {
                self.store.finish_agent_with_event(
                    &agent_id,
                    "cancelled",
                    Some("cancelled"),
                    KnownPayload::SubagentFailed(SubagentFailed {
                        agent_id: agent_id.clone(),
                        error: "cancelled".to_owned(),
                    }),
                    Utc::now(),
                )?;
            }
            return Ok(format!(
                "<subagent role=\"{role}\" status=\"cancelled\" turns=\"{turns}\" tokens=\"{}\">\ncancelled\n</subagent>",
                input_tokens + output_tokens
            ));
        }

        match result {
            Ok(_) => {
                self.store.finish_agent_with_event(
                    &agent_id,
                    "completed",
                    Some(&summary),
                    KnownPayload::SubagentCompleted(SubagentCompleted {
                        agent_id: agent_id.clone(),
                        summary: Some(summary.clone()),
                        input_tokens,
                        output_tokens,
                        turns,
                    }),
                    Utc::now(),
                )?;
                Ok(format!(
                    "<subagent role=\"{role}\" status=\"completed\" turns=\"{turns}\" tokens=\"{}\">\n{summary}\n</subagent>",
                    input_tokens + output_tokens
                ))
            }
            Err(err) => {
                let error = err.to_string();
                self.store.finish_agent_with_event(
                    &agent_id,
                    "failed",
                    Some(&error),
                    KnownPayload::SubagentFailed(SubagentFailed {
                        agent_id: agent_id.clone(),
                        error: error.clone(),
                    }),
                    Utc::now(),
                )?;
                Ok(format!(
                    "<subagent role=\"{role}\" status=\"failed\" turns=\"{turns}\" tokens=\"{}\">\n{error}\n</subagent>",
                    input_tokens + output_tokens
                ))
            }
        }
    }

    /// Mark a running subagent as cancelled and interrupt its child run.
    ///
    /// When this process owns the child, the in-memory token stops it. When
    /// another process owns it, the request is persisted and the owner polls
    /// it; we never claim the child finished unless the row is ours.
    pub fn cancel_subagent(&self, agent_id: &AgentId) -> Result<()> {
        let agent = self.store.get_agent(agent_id)?;
        if agent.status != "running" {
            return Ok(());
        }
        let local_token = self
            .subagent_cancels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(agent_id)
            .cloned();
        if let Some(token) = local_token {
            token.cancel();
            // The run loop owns the terminal transition; only claim it here if
            // the child is still running, so a completed result is not lost.
            self.store.finish_agent_with_event(
                agent_id,
                "cancelled",
                Some("cancelled"),
                KnownPayload::SubagentFailed(SubagentFailed {
                    agent_id: agent_id.clone(),
                    error: "cancelled".to_owned(),
                }),
                Utc::now(),
            )?;
            return Ok(());
        }
        // Owned by another live process: hand the request over and let the
        // owner report the terminal state once its run actually stops.
        if self.store.request_agent_cancel(agent_id, Utc::now())? {
            return Ok(());
        }
        // The owner is gone and the request could not be recorded; nothing is
        // running any more, so converge the row instead of leaving it stuck.
        self.reconcile_stale_agents()?;
        Ok(())
    }

    /// Forward cross-process cancel requests to this process's own tokens.
    /// Called from `pump` and the TUI/Web run loops.
    pub fn drain_cancel_requests(&self) -> Result<usize> {
        let mut delivered = 0;
        for agent in self.store.list_cancel_requested_agents()? {
            let token = self
                .subagent_cancels
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&agent.id)
                .cloned();
            if let Some(token) = token {
                token.cancel();
                delivered += 1;
            }
        }
        Ok(delivered)
    }

    /// Queue a steering message for a running subagent. It is delivered as the
    /// next user turn in the child session.
    pub fn steer_subagent(&self, agent_id: &AgentId, message: &str) -> Result<()> {
        let agent = self.store.get_agent(agent_id)?;
        if agent.status != "running" {
            return Err(BloraError::Other(format!(
                "subagent {agent_id} is not running"
            )));
        }
        self.queue_steer(&agent.child_session_id, message)
    }

    /// Capture a child `handoff` so the parent report prefers it over the last
    /// assistant message.
    pub(crate) fn record_subagent_handoff(
        &self,
        session_id: &SessionId,
        arguments: &Value,
    ) -> Result<String> {
        let summary = arguments
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if summary.is_empty() {
            return Err(BloraError::Other("handoff needs summary".to_owned()));
        }
        let files = arguments
            .get("files")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let mut text = summary.to_owned();
        if !files.is_empty() {
            text.push_str("\n\nfiles:\n");
            text.push_str(files);
        }
        if self.store.session_parent(session_id)?.is_some() {
            self.subagent_handoffs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(session_id.clone(), text.clone());
        }
        Ok(format!(
            "HANDOFF recorded. Stop calling tools and confirm you are done.\n{text}"
        ))
    }

    /// Mirror a child assistant message onto the parent session as live progress.
    pub(crate) fn relay_subagent_progress(
        &self,
        child_session: &SessionId,
        text: &str,
    ) -> Result<()> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let Some(agent) = self.store.agent_for_child_session(child_session)? else {
            return Ok(());
        };
        if agent.status != "running" {
            return Ok(());
        }
        let snippet: String = trimmed.chars().take(400).collect();
        self.emit(
            &agent.parent_session_id,
            None,
            None,
            KnownPayload::SubagentMessage(SubagentMessage {
                agent_id: agent.id,
                text: snippet,
            }),
        )
    }
}

struct SubagentCancelGuard<'a> {
    runtime: &'a Runtime,
    agent_id: AgentId,
    child_session: SessionId,
}

impl Drop for SubagentCancelGuard<'_> {
    fn drop(&mut self) {
        self.runtime
            .subagent_cancels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.agent_id);
        self.runtime
            .subagent_handoffs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.child_session);
        self.runtime
            .subagent_owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.child_session);
    }
}

/// Whether a finished child should be reported as cancelled.
///
/// The child run's terminal status wins over the cancel token: cancelling the
/// parent while the last child turn is completing must not discard its result.
fn child_was_cancelled(
    run_status: Option<blora_types::RunStatus>,
    token_cancelled: bool,
    agent_status: Option<&str>,
) -> bool {
    match run_status {
        Some(blora_types::RunStatus::Cancelled) => true,
        Some(blora_types::RunStatus::Completed | blora_types::RunStatus::Failed) => false,
        // No run was recorded (cancel before the first turn) or the run is
        // still open (the provider ignored cancellation).
        _ => token_cancelled || agent_status == Some("cancelled"),
    }
}

/// Whether a process id still names a running process.
///
/// Rows written by older builds have no owner pid and are treated as orphaned.
fn process_is_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        // `tasklist /FI` is local and fast enough for the rare reconcile path.
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output();
        output
            .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
    }
    #[cfg(not(windows))]
    {
        // Signal 0 probes existence without touching the process.
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

fn normalize_role(role: &str) -> String {
    let trimmed = role.trim();
    if valid_role_name(trimmed) {
        trimmed.to_owned()
    } else {
        "worker".to_owned()
    }
}

fn valid_role_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        && name.len() <= 64
}

fn resolve_role(workspace: &str, role: &str) -> RoleSpec {
    if let Some(spec) = builtin_role(role) {
        return spec;
    }
    if let Some(spec) = load_role_file(workspace, role) {
        return spec;
    }
    RoleSpec {
        duty: "You are a subagent working on one self-contained task for a parent agent."
            .to_owned(),
        budget: SUBAGENT_TURNS,
        read_only: false,
    }
}

fn builtin_role(role: &str) -> Option<RoleSpec> {
    let (duty, budget, read_only) = match role {
        "research" => (
            "You are a read-only research subagent. Investigate with read_file, list_dir, glob, search, and git tools. Do not edit files. Back every finding with a path:line reference or the command output it came from.",
            6,
            true,
        ),
        "review" => (
            "You are a read-only review subagent. Inspect the code or change described below. Report concrete problems ordered by severity (bug, risk, style), each with a path:line reference and a suggested fix. Say so if you find nothing significant.",
            6,
            true,
        ),
        "plan" => (
            "You are a read-only planning subagent. Study the relevant code, then produce an ordered implementation plan: steps, files to touch, risks, and how to verify each step. Do not edit files.",
            4,
            true,
        ),
        "code" => (
            "You are an implementation subagent. Make the change described below, following the existing conventions. Run the most relevant build or test command to verify it.",
            8,
            false,
        ),
        _ => return None,
    };
    Some(RoleSpec {
        duty: duty.to_owned(),
        budget,
        read_only,
    })
}

fn load_role_file(workspace: &str, role: &str) -> Option<RoleSpec> {
    let candidates = role_file_candidates(workspace, role);
    let path = candidates.into_iter().find(|path| path.is_file())?;
    let raw = std::fs::read_to_string(path).ok()?;
    Some(parse_role_markdown(&raw))
}

fn role_file_candidates(workspace: &str, role: &str) -> Vec<PathBuf> {
    let mut out = vec![
        Path::new(workspace)
            .join(".blora")
            .join("agents")
            .join(format!("{role}.md")),
        Path::new(workspace)
            .join(".blora")
            .join("agents")
            .join(role)
            .join("AGENT.md"),
    ];
    if let Some(home) = std::env::var_os("BLORA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".blora")))
    {
        out.push(home.join("agents").join(format!("{role}.md")));
        out.push(home.join("agents").join(role).join("AGENT.md"));
    }
    out
}

fn parse_role_markdown(raw: &str) -> RoleSpec {
    let mut read_only = false;
    let mut budget = SUBAGENT_TURNS;
    let body = if let Some(rest) = raw.trim_start().strip_prefix("---") {
        if let Some((frontmatter, rest)) = rest.split_once("\n---") {
            for line in frontmatter.lines() {
                let line = line.trim();
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                let key = key.trim().to_ascii_lowercase();
                let value = value.trim().trim_matches(|ch| ch == '"' || ch == '\'');
                match key.as_str() {
                    "readonly" | "read_only" | "read-only" => {
                        read_only =
                            matches!(value.to_ascii_lowercase().as_str(), "true" | "yes" | "1");
                    }
                    "budget" | "budget_turns" | "turns" => {
                        if let Ok(turns) = value.parse::<u32>() {
                            budget = turns.clamp(1, 40);
                        }
                    }
                    _ => {}
                }
            }
            rest.trim().to_owned()
        } else {
            raw.trim().to_owned()
        }
    } else {
        raw.trim().to_owned()
    };
    let duty = if body.is_empty() {
        "You are a subagent working on one self-contained task for a parent agent.".to_owned()
    } else {
        body
    };
    RoleSpec {
        duty,
        budget,
        read_only,
    }
}

/// Task text for a child agent. The child cannot see the parent conversation,
/// so the brief states its role, limits, and the expected final report.
fn subagent_brief(role: &str, duty: &str, prompt: &str, can_spawn: bool) -> String {
    let mut out = format!("[subagent:{role}] {duty}\n");
    out.push_str(
        "Your turn budget is small: work efficiently, use parallel read-only calls, and stop when the task is done.\n",
    );
    if !can_spawn {
        out.push_str("Do not spawn another subagent.\n");
    }
    out.push_str(
        "Finish with a final message for the parent agent: the result first, then key evidence (paths, commands, outputs), files changed if any, and open questions. Prefer the handoff tool when you have a structured report. The parent only sees that final message.\n\n<task>\n",
    );
    out.push_str(prompt.trim());
    out.push_str("\n</task>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::Runtime;
    use blora_model::{Completion, Provider, StreamEvent};
    use blora_storage::{CreateSession, SqliteStore};
    use blora_types::Mode;
    use std::sync::Arc;
    use std::time::Duration;

    fn duty(role: &str) -> String {
        resolve_role(".", role).duty
    }

    fn session(runtime: &Runtime, mode: Mode, workspace: &str) -> SessionId {
        runtime
            .create_session(CreateSession {
                title: Some("t".to_owned()),
                workspace_path: workspace.to_owned(),
                mode,
                parent_session_id: None,
                user_id: None,
            })
            .unwrap()
    }

    fn yolo() -> RunOptions {
        RunOptions {
            mock: true,
            auto_approve: true,
            ..RunOptions::default()
        }
    }

    struct HangUntilCancel;

    impl Provider for HangUntilCancel {
        fn name(&self) -> &str {
            "mock"
        }

        fn complete(
            &self,
            _request: &blora_model::CompletionRequest,
            cancel: &CancelToken,
            _on_event: &mut dyn FnMut(StreamEvent) -> Result<()>,
        ) -> Result<Completion> {
            for _ in 0..400 {
                if cancel.is_cancelled() {
                    return Err(BloraError::Cancelled);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(Completion {
                text: "hung".to_owned(),
                finish_reason: "stop".to_owned(),
                ..Completion::default()
            })
        }
    }

    #[test]
    fn brief_states_role_task_and_report_contract() {
        let brief = subagent_brief("review", &duty("review"), "  check the parser  ", true);
        assert!(brief.starts_with("[subagent:review]"));
        assert!(brief.contains("ordered by severity"));
        assert!(brief.contains("<task>\ncheck the parser\n</task>"));
        assert!(brief.contains("The parent only sees that final message"));
        assert!(!brief.contains("Do not spawn"));
        assert!(
            subagent_brief("code", &duty("code"), "x", false)
                .contains("Do not spawn another subagent.")
        );
        assert!(subagent_brief("other", &duty("other"), "x", true).contains("self-contained task"));
    }

    #[test]
    fn custom_role_file_sets_duty_budget_and_readonly() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".blora/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("audit.md"),
            "---\nreadonly: true\nbudget: 5\n---\nYou audit secrets and never edit files.\n",
        )
        .unwrap();
        let spec = resolve_role(dir.path().to_str().unwrap(), "audit");
        assert!(spec.read_only);
        assert_eq!(spec.budget, 5);
        assert!(spec.duty.contains("audit secrets"));
        let brief = subagent_brief("audit", &spec.duty, "scan env", true);
        assert!(brief.contains("audit secrets"));
    }

    #[test]
    fn rejects_depth_beyond_limit() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let root = session(&runtime, Mode::Code, ".");
        let child = runtime
            .create_session(CreateSession {
                title: Some("c".to_owned()),
                workspace_path: ".".to_owned(),
                mode: Mode::Code,
                parent_session_id: Some(root),
                user_id: None,
            })
            .unwrap();
        let grandchild = runtime
            .create_session(CreateSession {
                title: Some("g".to_owned()),
                workspace_path: ".".to_owned(),
                mode: Mode::Code,
                parent_session_id: Some(child),
                user_id: None,
            })
            .unwrap();
        assert_eq!(runtime.session_depth(&grandchild).unwrap(), 2);
        let err = runtime
            .spawn_subagent(
                &grandchild,
                "research",
                "too deep",
                &yolo(),
                &CancelToken::new(),
            )
            .unwrap_err();
        assert!(err.to_string().contains("depth"));
    }

    #[test]
    fn cancel_subagent_interrupts_child_run() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Code, dir.path().to_str().unwrap());
        *runtime.test_provider.lock().unwrap() = Some(Arc::new(HangUntilCancel));
        std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                runtime.spawn_subagent(
                    &session,
                    "research",
                    "hang until cancelled",
                    &yolo(),
                    &CancelToken::new(),
                )
            });
            let agent_id = loop {
                if let Ok(rows) = runtime.list_subagents(&session) {
                    if let Some(row) = rows.into_iter().find(|row| row.status == "running") {
                        break row.id;
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            runtime.cancel_subagent(&agent_id).unwrap();
            let report = handle.join().unwrap().unwrap();
            assert!(report.contains("status=\"cancelled\""), "{report}");
            let projection = runtime.show_session(&session).unwrap();
            assert_eq!(projection.subagents[0].status, "cancelled");
        });
    }

    #[test]
    fn invalid_role_falls_back_to_worker() {
        assert_eq!(normalize_role("../x"), "worker");
        assert_eq!(normalize_role("review"), "review");
        assert_eq!(normalize_role(" custom_role "), "custom_role");
    }

    #[test]
    fn finished_child_is_not_relabelled_by_a_racing_parent_cancel() {
        use blora_types::RunStatus;
        assert!(!child_was_cancelled(
            Some(RunStatus::Completed),
            true,
            Some("running")
        ));
        assert!(!child_was_cancelled(
            Some(RunStatus::Failed),
            true,
            Some("running")
        ));
        assert!(child_was_cancelled(Some(RunStatus::Cancelled), false, None));
        assert!(child_was_cancelled(None, true, Some("running")));
        assert!(child_was_cancelled(None, false, Some("cancelled")));
        assert!(!child_was_cancelled(None, false, Some("running")));
    }

    #[test]
    fn reconcile_marks_orphaned_running_agents_interrupted() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let parent = session(&runtime, Mode::Code, ".");
        let child = runtime
            .create_session(CreateSession {
                title: Some("subagent:worker".to_owned()),
                workspace_path: ".".to_owned(),
                mode: Mode::Code,
                parent_session_id: Some(parent.clone()),
                user_id: None,
            })
            .unwrap();
        let agent_id = AgentId::generate();
        let now = Utc::now();
        runtime
            .store
            .try_insert_running_agent(
                &AgentRecord {
                    id: agent_id.clone(),
                    parent_session_id: parent.clone(),
                    child_session_id: child.clone(),
                    role: "worker".to_owned(),
                    depth: 1,
                    status: "running".to_owned(),
                    budget_turns: SUBAGENT_TURNS,
                    summary: None,
                    owner_pid: None,
                    cancel_requested: false,
                    created_at: now,
                    updated_at: now,
                },
                u32::MAX,
            )
            .unwrap();
        // The crashed process had already announced the spawn.
        runtime
            .append_event(
                &parent,
                None,
                KnownPayload::SubagentSpawned(SubagentSpawned {
                    agent_id: agent_id.clone(),
                    role: "worker".to_owned(),
                    child_session_id: Some(child),
                    prompt: Some("do work".to_owned()),
                    depth: 1,
                }),
            )
            .unwrap();

        assert_eq!(runtime.reconcile_stale_agents().unwrap(), 1);
        let agent = runtime.get_subagent(&agent_id).unwrap();
        assert_eq!(agent.status, "failed");
        assert_eq!(agent.summary.as_deref(), Some("interrupted"));
        let projection = runtime.show_session(&parent).unwrap();
        assert!(
            projection
                .subagents
                .iter()
                .any(|agent| agent.id == agent_id && agent.status == "failed")
        );
        // Idempotent: a converged row is not reported twice.
        assert_eq!(runtime.reconcile_stale_agents().unwrap(), 0);
    }

    #[test]
    fn reconcile_rebuilds_the_child_report_and_usage() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let parent = session(&runtime, Mode::Code, dir.path().to_str().unwrap());
        let child = runtime
            .create_session(CreateSession {
                title: Some("subagent:worker".to_owned()),
                workspace_path: dir.path().to_str().unwrap().to_owned(),
                mode: Mode::Code,
                parent_session_id: Some(parent.clone()),
                user_id: None,
            })
            .unwrap();
        // A crashed child that had already produced a final answer and usage.
        runtime
            .run_mock(
                &child,
                "measure",
                &CancelToken::new(),
                &crate::MockRunOptions {
                    chunks: vec!["measured 42 widgets".to_owned()],
                    ..Default::default()
                },
            )
            .unwrap();
        runtime
            .append_event(
                &child,
                None,
                KnownPayload::UsageRecorded(blora_events::UsageRecorded {
                    input_tokens: 11,
                    output_tokens: 7,
                    cached_tokens: 0,
                }),
            )
            .unwrap();
        let agent_id = AgentId::generate();
        let now = Utc::now();
        runtime
            .store
            .try_insert_running_agent(
                &AgentRecord {
                    id: agent_id.clone(),
                    parent_session_id: parent.clone(),
                    child_session_id: child.clone(),
                    role: "worker".to_owned(),
                    depth: 1,
                    status: "running".to_owned(),
                    budget_turns: SUBAGENT_TURNS,
                    summary: None,
                    owner_pid: None,
                    cancel_requested: false,
                    created_at: now,
                    updated_at: now,
                },
                u32::MAX,
            )
            .unwrap();
        runtime
            .append_event(
                &parent,
                None,
                KnownPayload::SubagentSpawned(SubagentSpawned {
                    agent_id: agent_id.clone(),
                    role: "worker".to_owned(),
                    child_session_id: Some(child),
                    prompt: Some("measure".to_owned()),
                    depth: 1,
                }),
            )
            .unwrap();

        assert_eq!(runtime.reconcile_stale_agents().unwrap(), 1);
        let agent = runtime.get_subagent(&agent_id).unwrap();
        assert_eq!(agent.status, "completed");
        assert_eq!(agent.summary.as_deref(), Some("measured 42 widgets"));
        let events = runtime.events(&parent).unwrap();
        let done = events
            .iter()
            .find(|event| event.event_type == "subagent.completed")
            .expect("completed event");
        assert_eq!(done.payload["input_tokens"], 11);
        assert_eq!(done.payload["output_tokens"], 7);
        assert_eq!(done.payload["turns"], 1);
    }

    #[test]
    fn reconcile_skips_children_this_process_is_running() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let parent = session(&runtime, Mode::Code, ".");
        let child = runtime
            .create_session(CreateSession {
                title: Some("subagent:worker".to_owned()),
                workspace_path: ".".to_owned(),
                mode: Mode::Code,
                parent_session_id: Some(parent),
                user_id: None,
            })
            .unwrap();
        let agent_id = AgentId::generate();
        let now = Utc::now();
        runtime
            .store
            .try_insert_running_agent(
                &AgentRecord {
                    id: agent_id.clone(),
                    parent_session_id: runtime.store.session_parent(&child).unwrap().unwrap(),
                    child_session_id: child,
                    role: "worker".to_owned(),
                    depth: 1,
                    status: "running".to_owned(),
                    budget_turns: SUBAGENT_TURNS,
                    summary: None,
                    owner_pid: None,
                    cancel_requested: false,
                    created_at: now,
                    updated_at: now,
                },
                u32::MAX,
            )
            .unwrap();
        runtime
            .subagent_cancels
            .lock()
            .unwrap()
            .insert(agent_id.clone(), CancelToken::new());
        assert_eq!(runtime.reconcile_stale_agents().unwrap(), 0);
        assert_eq!(runtime.get_subagent(&agent_id).unwrap().status, "running");
    }

    #[test]
    fn reconcile_leaves_rows_owned_by_a_live_process_alone() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let parent = session(&runtime, Mode::Code, ".");
        let child = runtime
            .create_session(CreateSession {
                title: Some("subagent:worker".to_owned()),
                workspace_path: ".".to_owned(),
                mode: Mode::Code,
                parent_session_id: Some(parent),
                user_id: None,
            })
            .unwrap();
        let agent_id = AgentId::generate();
        let now = Utc::now();
        runtime
            .store
            .try_insert_running_agent(
                &AgentRecord {
                    id: agent_id.clone(),
                    parent_session_id: runtime.store.session_parent(&child).unwrap().unwrap(),
                    child_session_id: child,
                    role: "worker".to_owned(),
                    depth: 1,
                    status: "running".to_owned(),
                    budget_turns: SUBAGENT_TURNS,
                    summary: None,
                    // The current test process is still alive, so this is a
                    // child another process (TUI vs Web) is driving.
                    owner_pid: Some(std::process::id()),
                    cancel_requested: false,
                    created_at: now,
                    updated_at: now,
                },
                u32::MAX,
            )
            .unwrap();
        assert_eq!(runtime.reconcile_stale_agents().unwrap(), 0);
        assert_eq!(runtime.get_subagent(&agent_id).unwrap().status, "running");
    }

    #[test]
    fn spawn_over_the_running_cap_archives_the_child_by_event() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let parent = session(&runtime, Mode::Code, ".");
        let now = Utc::now();
        for _ in 0..MAX_RUNNING_CHILDREN {
            let child = runtime
                .create_session(CreateSession {
                    title: Some("subagent:worker".to_owned()),
                    workspace_path: ".".to_owned(),
                    mode: Mode::Code,
                    parent_session_id: Some(parent.clone()),
                    user_id: None,
                })
                .unwrap();
            let agent_id = AgentId::generate();
            runtime
                .store
                .try_insert_running_agent(
                    &AgentRecord {
                        id: agent_id.clone(),
                        parent_session_id: parent.clone(),
                        child_session_id: child,
                        role: "worker".to_owned(),
                        depth: 1,
                        status: "running".to_owned(),
                        budget_turns: SUBAGENT_TURNS,
                        summary: None,
                        owner_pid: None,
                        cancel_requested: false,
                        created_at: now,
                        updated_at: now,
                    },
                    u32::MAX,
                )
                .unwrap();
            // These children are "live" in this process, so stale reconcile
            // leaves them alone and the cap actually applies.
            runtime
                .subagent_cancels
                .lock()
                .unwrap()
                .insert(agent_id, CancelToken::new());
        }
        let err = runtime
            .spawn_subagent(
                &parent,
                "research",
                "one too many",
                &yolo(),
                &CancelToken::new(),
            )
            .unwrap_err();
        assert!(
            err.to_string().contains("too many running subagents"),
            "{err}"
        );
        // The rejected child was created and archived through the event log.
        let child = runtime
            .list_sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.title.as_deref() == Some("subagent:research"))
            .expect("child session");
        assert_eq!(child.status, blora_types::SessionStatus::Archived);
        let projection = runtime.show_session(&child.id).unwrap();
        assert_eq!(
            projection.session.unwrap().status,
            blora_types::SessionStatus::Archived
        );
    }

    #[test]
    fn delegated_child_inherits_the_parent_user() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let (alice, _) = runtime.create_user("alice").unwrap();
        let parent = session(&runtime, Mode::Code, ".");
        runtime.set_session_user(&parent, &alice.id).unwrap();
        let child = runtime
            .create_session(CreateSession {
                title: Some("subagent:worker".to_owned()),
                workspace_path: ".".to_owned(),
                mode: Mode::Code,
                parent_session_id: Some(parent.clone()),
                user_id: None,
            })
            .unwrap();
        assert_eq!(
            runtime.session_user_id(&child).unwrap().as_deref(),
            Some(alice.id.as_str())
        );
        // A root session created without a parent is still unowned.
        let other = session(&runtime, Mode::Code, ".");
        assert!(runtime.session_user_id(&other).unwrap().is_none());
    }

    #[test]
    fn search_fails_closed_for_unowned_sessions_in_a_multi_user_store() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let (alice, _) = runtime.create_user("alice").unwrap();
        let owned = session(&runtime, Mode::Code, ".");
        runtime.set_session_user(&owned, &alice.id).unwrap();
        runtime
            .append_event(
                &owned,
                None,
                KnownPayload::UserInput(blora_events::UserInput {
                    text: "alice secret plan".to_owned(),
                }),
            )
            .unwrap();
        let unowned = session(&runtime, Mode::Code, ".");
        let hits = runtime
            .session_search(Some(&unowned), "alice secret plan", 10)
            .unwrap();
        assert!(hits.is_empty(), "{hits:?}");
        // Another session of the same owner still sees that history.
        let sibling = session(&runtime, Mode::Code, ".");
        runtime.set_session_user(&sibling, &alice.id).unwrap();
        let hits = runtime
            .session_search(Some(&sibling), "alice secret plan", 10)
            .unwrap();
        assert!(!hits.is_empty());
    }

    #[test]
    fn subagents_never_block_on_ask_user() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let parent = session(&runtime, Mode::Code, ".");
        // Interactive for approvals, but a child must not wait for a person.
        let report = runtime
            .spawn_subagent(
                &parent,
                "research",
                "ask the user something",
                &RunOptions {
                    mock: true,
                    interactive: true,
                    ..RunOptions::default()
                },
                &CancelToken::new(),
            )
            .unwrap();
        assert!(report.contains("status=\"completed\""), "{report}");
        // Nothing is left waiting for input anywhere.
        assert!(
            runtime
                .show_session(&parent)
                .unwrap()
                .pending_input
                .is_none()
        );
    }

    #[test]
    fn cross_process_cancel_request_stops_the_owner() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.sqlite");
        let owner = Runtime::new(SqliteStore::open(&db).unwrap());
        let outsider = Runtime::new(SqliteStore::open(&db).unwrap());
        let parent = {
            let id = owner
                .create_session(CreateSession {
                    title: Some("parent".to_owned()),
                    workspace_path: dir.path().to_str().unwrap().to_owned(),
                    mode: Mode::Code,
                    parent_session_id: None,
                    user_id: None,
                })
                .unwrap();
            id
        };
        *owner.test_provider.lock().unwrap() = Some(Arc::new(HangUntilCancel));
        std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                owner.spawn_subagent(
                    &parent,
                    "research",
                    "hang until cancelled from elsewhere",
                    &yolo(),
                    &CancelToken::new(),
                )
            });
            let agent_id = loop {
                if let Ok(rows) = outsider.list_subagents(&parent) {
                    if let Some(row) = rows.into_iter().find(|row| row.status == "running") {
                        break row.id;
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            // The outsider has no in-memory token; it must persist the request.
            outsider.cancel_subagent(&agent_id).unwrap();
            let report = handle.join().unwrap().unwrap();
            assert!(report.contains("status=\"cancelled\""), "{report}");
            let agent = owner.get_subagent(&agent_id).unwrap();
            assert_eq!(agent.status, "cancelled");
        });
    }

    #[test]
    fn subagent_approvals_surface_on_the_parent_session() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let parent = session(&runtime, Mode::Code, dir.path().to_str().unwrap());
        std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                runtime.spawn_subagent(
                    &parent,
                    "code",
                    "WRITE_FILE please",
                    &RunOptions {
                        mock: true,
                        interactive: true,
                        ..RunOptions::default()
                    },
                    &CancelToken::new(),
                )
            });
            let pending = loop {
                if let Ok(rows) = runtime.pending_approvals(&parent) {
                    if let Some(row) = rows.into_iter().next() {
                        break row;
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            assert!(
                pending.summary.contains("[subagent:code]"),
                "{}",
                pending.summary
            );
            runtime.resolve_approval(&pending.id, true).unwrap();
            let report = handle.join().unwrap().unwrap();
            assert!(report.contains("status=\"completed\""), "{report}");
        });
        assert_eq!(
            std::fs::read_to_string(dir.path().join("ok.txt")).unwrap(),
            "ok"
        );
    }

    #[test]
    fn nested_subagent_approvals_reach_the_root_session() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let root = session(&runtime, Mode::Code, ".");
        let middle = runtime
            .create_session(CreateSession {
                title: Some("subagent:worker".to_owned()),
                workspace_path: ".".to_owned(),
                mode: Mode::Code,
                parent_session_id: Some(root.clone()),
                user_id: None,
            })
            .unwrap();
        let leaf = runtime
            .create_session(CreateSession {
                title: Some("subagent:code".to_owned()),
                workspace_path: ".".to_owned(),
                mode: Mode::Code,
                parent_session_id: Some(middle.clone()),
                user_id: None,
            })
            .unwrap();
        runtime
            .subagent_owners
            .lock()
            .unwrap()
            .insert(middle.clone(), (root.clone(), "worker".to_owned()));
        runtime
            .subagent_owners
            .lock()
            .unwrap()
            .insert(leaf.clone(), (middle.clone(), "code".to_owned()));
        let (owner, summary) = runtime.approval_owner(&leaf, "write ok.txt").unwrap();
        assert_eq!(owner, root);
        assert!(summary.contains("subagent:worker"), "{summary}");
        assert!(summary.contains("subagent:code"), "{summary}");
    }
}
