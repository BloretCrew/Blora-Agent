// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::thread;
use std::time::{Duration, Instant};

use blora_events::{ApprovalRequested, ApprovalResolved, KnownPayload, PolicyDenied};
use blora_storage::ApprovalRecord;
use blora_tools::ToolRegistry;
use blora_types::{ApprovalId, BloraError, CancelToken, Result, RunId, SessionId, TurnId};
use chrono::Utc;

use crate::Runtime;
use crate::config::{ApprovalDecision, ApprovalPolicy, ApprovalRequest};

impl Runtime {
    pub fn pending_approvals(&self, session_id: &SessionId) -> Result<Vec<ApprovalRecord>> {
        self.store.list_pending_approvals(session_id)
    }

    pub fn pending_approvals_all(&self) -> Result<Vec<ApprovalRecord>> {
        self.store.list_all_pending_approvals()
    }

    pub fn resolve_approval(&self, approval_id: &ApprovalId, allow: bool) -> Result<()> {
        let decision = if allow { "allow" } else { "deny" };
        if !self
            .store
            .resolve_approval_row(approval_id, decision, Utc::now())?
        {
            return Err(BloraError::Other(format!(
                "approval {approval_id} is not pending"
            )));
        }
        Ok(())
    }

    pub(crate) fn await_approval(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        capability: &str,
        summary: &str,
        cancel: &CancelToken,
    ) -> Result<bool> {
        let approval_id = ApprovalId::generate();
        let now = Utc::now();
        self.store.insert_approval(&ApprovalRecord {
            id: approval_id.clone(),
            session_id: session_id.clone(),
            run_id: Some(run_id.clone()),
            capability: capability.to_owned(),
            summary: summary.to_owned(),
            status: "pending".to_owned(),
            created_at: now,
            updated_at: now,
        })?;
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::ApprovalRequested(ApprovalRequested {
                approval_id: approval_id.clone(),
                capability: capability.to_owned(),
                summary: summary.to_owned(),
                policy_version: blora_policy::POLICY_VERSION,
            }),
        )?;
        let policy = self.config.callbacks.approval.clone();
        let immediate = match &policy {
            ApprovalPolicy::DenyAll => Some(false),
            ApprovalPolicy::AllowAll => Some(true),
            ApprovalPolicy::AllowReadOnly => Some(ToolRegistry::is_read_only(capability)),
            ApprovalPolicy::Auto(handler) => {
                let request = ApprovalRequest {
                    approval_id: approval_id.clone(),
                    session_id: session_id.clone(),
                    run_id: run_id.clone(),
                    capability: capability.to_owned(),
                    summary: summary.to_owned(),
                };
                match handler.decide(&request) {
                    ApprovalDecision::Allow => Some(true),
                    ApprovalDecision::Deny => Some(false),
                    ApprovalDecision::AskUser => None,
                }
            }
            ApprovalPolicy::Manual { .. } => None,
        };
        if let Some(allow) = immediate {
            self.store.resolve_approval_row(
                &approval_id,
                if allow { "allow" } else { "deny" },
                Utc::now(),
            )?;
            return self.apply_approval_decision(
                session_id,
                run_id,
                turn_id,
                &approval_id,
                capability,
                summary,
                allow,
            );
        }

        let deadline = match &policy {
            ApprovalPolicy::Manual { timeout } => *timeout,
            ApprovalPolicy::Auto(_) => Some(Duration::from_secs(300)),
            _ => None,
        };
        if matches!(policy, ApprovalPolicy::Manual { .. }) {
            if let Some(0) = deadline.map(|value| value.as_secs()) {
                self.store
                    .resolve_approval_row(&approval_id, "deny", Utc::now())?;
                return self.apply_approval_decision(
                    session_id,
                    run_id,
                    turn_id,
                    &approval_id,
                    capability,
                    &format!("{summary} (approval timed out)"),
                    false,
                );
            }
        }
        let started = Instant::now();
        loop {
            if cancel.is_cancelled() {
                let _ = self
                    .store
                    .resolve_approval_row(&approval_id, "deny", Utc::now());
                return Err(BloraError::Cancelled);
            }
            if deadline.is_some_and(|limit| started.elapsed() >= limit) {
                self.store
                    .resolve_approval_row(&approval_id, "deny", Utc::now())?;
                return self.apply_approval_decision(
                    session_id,
                    run_id,
                    turn_id,
                    &approval_id,
                    capability,
                    &format!("{summary} (approval timed out)"),
                    false,
                );
            }
            let record = self.store.get_approval(&approval_id)?;
            match record.status.as_str() {
                "allow" => {
                    self.store
                        .resolve_approval_row(&approval_id, "allow", Utc::now())?;
                    self.emit(
                        session_id,
                        Some(run_id),
                        Some(turn_id),
                        KnownPayload::ApprovalResolved(ApprovalResolved {
                            approval_id,
                            decision: "allow".to_owned(),
                        }),
                    )?;
                    return Ok(true);
                }
                "deny" => {
                    self.store
                        .resolve_approval_row(&approval_id, "deny", Utc::now())?;
                    self.emit(
                        session_id,
                        Some(run_id),
                        Some(turn_id),
                        KnownPayload::ApprovalResolved(ApprovalResolved {
                            approval_id,
                            decision: "deny".to_owned(),
                        }),
                    )?;
                    self.emit(
                        session_id,
                        Some(run_id),
                        Some(turn_id),
                        KnownPayload::PolicyDenied(PolicyDenied {
                            capability: capability.to_owned(),
                            reason: summary.to_owned(),
                        }),
                    )?;
                    return Ok(false);
                }
                _ => thread::sleep(Duration::from_millis(80)),
            }
        }
    }

    fn apply_approval_decision(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        approval_id: &ApprovalId,
        capability: &str,
        summary: &str,
        allow: bool,
    ) -> Result<bool> {
        let decision = if allow { "allow" } else { "deny" };
        self.store
            .resolve_approval_row(approval_id, decision, Utc::now())?;
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::ApprovalResolved(ApprovalResolved {
                approval_id: approval_id.clone(),
                decision: decision.to_owned(),
            }),
        )?;
        if !allow {
            self.emit(
                session_id,
                Some(run_id),
                Some(turn_id),
                KnownPayload::PolicyDenied(PolicyDenied {
                    capability: capability.to_owned(),
                    reason: summary.to_owned(),
                }),
            )?;
        }
        Ok(allow)
    }
}
