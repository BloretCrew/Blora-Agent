// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::thread;
use std::time::Duration;

use blora_events::{ApprovalRequested, ApprovalResolved, KnownPayload, PolicyDenied};
use blora_storage::ApprovalRecord;
use blora_types::{ApprovalId, BloraError, CancelToken, Result, RunId, SessionId, TurnId};
use chrono::Utc;

use crate::Runtime;

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
            }),
        )?;
        loop {
            if cancel.is_cancelled() {
                let _ = self
                    .store
                    .resolve_approval_row(&approval_id, "deny", Utc::now());
                return Err(BloraError::Cancelled);
            }
            let record = self.store.get_approval(&approval_id)?;
            match record.status.as_str() {
                "allow" => {
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
}
