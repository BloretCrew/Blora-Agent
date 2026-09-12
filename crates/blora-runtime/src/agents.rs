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
        let child_session = self.create_session(CreateSession {
            title: Some(format!("subagent:{role}")),
            workspace_path: parent.workspace_path,
            mode: Mode::Code,
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
            budget_turns: SUBAGENT_TURNS,
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
            }),
        )?;
        let child_options = RunOptions {
            model: options.model.clone(),
            mock: options.mock,
            auto_approve: options.auto_approve,
            max_turns: SUBAGENT_TURNS,
        };
        let child_prompt = format!("[subagent:{role}] {prompt}\nDo not spawn another subagent.");
        let result = self.run(&child_session, &child_prompt, cancel, &child_options);
        let summary = self
            .show_session(&child_session)
            .ok()
            .and_then(|projection| {
                projection.transcript.into_iter().rev().find_map(|item| {
                    if let blora_session::TranscriptItem::Assistant { text, .. } = item {
                        Some(text)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| "subagent finished without text".to_owned());
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
                    }),
                )?;
                Ok(summary)
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
}
