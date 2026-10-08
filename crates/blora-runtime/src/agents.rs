// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_events::{KnownPayload, SubagentCompleted, SubagentFailed, SubagentSpawned};
use blora_storage::{AgentRecord, CreateSession};
use blora_types::{AgentId, BloraError, CancelToken, Mode, Result, SessionId};
use chrono::Utc;

use crate::coordinator::{RunOptions, Runtime};

pub const MAX_SUBAGENT_DEPTH: u32 = 2;
pub const MAX_RUNNING_CHILDREN: u32 = 3;
pub const SUBAGENT_TURNS: u32 = 6;

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
        let depth = self.session_depth(parent_session)? + 1;
        if depth > MAX_SUBAGENT_DEPTH {
            return Err(BloraError::Policy(format!(
                "subagent depth {depth} exceeds {MAX_SUBAGENT_DEPTH}"
            )));
        }
        let running = self.store.running_child_count(parent_session)?;
        if running >= MAX_RUNNING_CHILDREN {
            return Err(BloraError::Policy(format!(
                "too many running subagents ({running}/{MAX_RUNNING_CHILDREN})"
            )));
        }
        let parent = self
            .show_session(parent_session)?
            .session
            .ok_or_else(|| BloraError::SessionNotFound(parent_session.to_string()))?;
        let (budget, read_only) = role_limits(role);
        let child_session = self.create_session(CreateSession {
            title: Some(format!("subagent:{role}")),
            workspace_path: parent.workspace_path,
            // Children keep the parent's strategy (Work children can research
            // the web); Imagine has no tool loop to delegate into.
            mode: match parent.mode {
                Mode::Imagine => Mode::Code,
                mode => mode,
            },
            parent_session_id: Some(parent_session.clone()),
        })?;
        let agent_id = AgentId::generate();
        let now = Utc::now();
        self.store.insert_agent(&AgentRecord {
            id: agent_id.clone(),
            parent_session_id: parent_session.clone(),
            child_session_id: child_session.clone(),
            role: role.to_owned(),
            depth,
            status: "running".to_owned(),
            budget_turns: budget,
            summary: None,
            created_at: now,
            updated_at: now,
        })?;
        self.emit(
            parent_session,
            None,
            None,
            KnownPayload::SubagentSpawned(SubagentSpawned {
                agent_id: agent_id.clone(),
                role: role.to_owned(),
                child_session_id: Some(child_session.clone()),
                prompt: Some(prompt.to_owned()),
                depth,
            }),
        )?;
        let child_options = RunOptions {
            model: options.model.clone(),
            mock: options.mock,
            auto_approve: options.auto_approve && !read_only,
            interactive: options.interactive,
            // Children report to the parent agent; only the parent talks to the user.
            can_ask: false,
            max_turns: budget,
            provider: options.provider.clone(),
            read_only,
            worktree: false,
            passport_user_token: options.passport_user_token.clone(),
            permission: if read_only {
                Some(blora_policy::PermissionMode::Plan)
            } else {
                options.permission
            },
        };
        let child_prompt = subagent_brief(role, prompt, depth < MAX_SUBAGENT_DEPTH);
        let result = self.run(&child_session, &child_prompt, cancel, &child_options);
        // Steer/stop: a control row may have been written while the child ran.
        let control = self.store.get_agent(&agent_id).ok();
        if control.as_ref().is_some_and(|a| a.status == "cancelled") {
            return Err(BloraError::Cancelled);
        }
        let projection = self.show_session(&child_session).ok();
        let summary = projection
            .as_ref()
            .and_then(|projection| {
                projection.transcript.iter().rev().find_map(|item| {
                    if let blora_session::TranscriptItem::Assistant { text, .. } = item {
                        Some(text.clone())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| "subagent finished without text".to_owned());
        let (input_tokens, output_tokens, turns) = projection
            .as_ref()
            .map(|p| {
                let turns = p
                    .transcript
                    .iter()
                    .filter(|item| matches!(item, blora_session::TranscriptItem::Assistant { .. }))
                    .count() as u32;
                (p.input_tokens, p.output_tokens, turns)
            })
            .unwrap_or_default();
        match result {
            Ok(_) => {
                self.store
                    .update_agent(&agent_id, "completed", Some(&summary), Utc::now())?;
                self.emit(
                    parent_session,
                    None,
                    None,
                    KnownPayload::SubagentCompleted(SubagentCompleted {
                        agent_id,
                        summary: Some(summary.clone()),
                        input_tokens,
                        output_tokens,
                        turns,
                    }),
                )?;
                Ok(format!(
                    "<subagent role=\"{role}\" turns=\"{turns}\" tokens=\"{}\">\n{summary}\n</subagent>",
                    input_tokens + output_tokens
                ))
            }
            Err(err) => {
                let error = err.to_string();
                self.store
                    .update_agent(&agent_id, "failed", Some(&error), Utc::now())?;
                self.emit(
                    parent_session,
                    None,
                    None,
                    KnownPayload::SubagentFailed(SubagentFailed {
                        agent_id,
                        error: error.clone(),
                    }),
                )?;
                Err(err)
            }
        }
    }

    /// Mark a running subagent as cancelled. The child run observes this at the
    /// next safe point via its parent's cancel token or when it returns.
    pub fn cancel_subagent(&self, agent_id: &AgentId) -> Result<()> {
        let agent = self.store.get_agent(agent_id)?;
        if agent.status != "running" {
            return Ok(());
        }
        self.store
            .update_agent(agent_id, "cancelled", Some("cancelled"), Utc::now())?;
        self.emit(
            &agent.parent_session_id,
            None,
            None,
            KnownPayload::SubagentFailed(SubagentFailed {
                agent_id: agent_id.clone(),
                error: "cancelled".to_owned(),
            }),
        )
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
}

/// Task text for a child agent. The child cannot see the parent conversation,
/// so the brief states its role, limits, and the expected final report.
fn subagent_brief(role: &str, prompt: &str, can_spawn: bool) -> String {
    let duty = match role {
        "research" => {
            "You are a read-only research subagent. Investigate with read_file, list_dir, glob, search, and git tools. Do not edit files. Back every finding with a path:line reference or the command output it came from."
        }
        "review" => {
            "You are a read-only review subagent. Inspect the code or change described below. Report concrete problems ordered by severity (bug, risk, style), each with a path:line reference and a suggested fix. Say so if you find nothing significant."
        }
        "plan" => {
            "You are a read-only planning subagent. Study the relevant code, then produce an ordered implementation plan: steps, files to touch, risks, and how to verify each step. Do not edit files."
        }
        "code" => {
            "You are an implementation subagent. Make the change described below, following the existing conventions. Run the most relevant build or test command to verify it."
        }
        _ => "You are a subagent working on one self-contained task for a parent agent.",
    };
    let mut out = format!("[subagent:{role}] {duty}\n");
    out.push_str(
        "Your turn budget is small: work efficiently, use parallel read-only calls, and stop when the task is done.\n",
    );
    if !can_spawn {
        out.push_str("Do not spawn another subagent.\n");
    }
    out.push_str(
        "Finish with a final message for the parent agent: the result first, then key evidence (paths, commands, outputs), files changed if any, and open questions. The parent only sees that final message.\n\n<task>\n",
    );
    out.push_str(prompt.trim());
    out.push_str("\n</task>");
    out
}

fn role_limits(role: &str) -> (u32, bool) {
    match role {
        "research" | "review" => (6, true),
        "plan" => (4, true),
        "code" => (8, false),
        _ => (SUBAGENT_TURNS, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_states_role_task_and_report_contract() {
        let brief = subagent_brief("review", "  check the parser  ", true);
        assert!(brief.starts_with("[subagent:review]"));
        assert!(brief.contains("ordered by severity"));
        assert!(brief.contains("<task>\ncheck the parser\n</task>"));
        assert!(brief.contains("The parent only sees that final message"));
        assert!(!brief.contains("Do not spawn"));
        assert!(subagent_brief("code", "x", false).contains("Do not spawn another subagent."));
        assert!(subagent_brief("other", "x", true).contains("self-contained task"));
    }
}
