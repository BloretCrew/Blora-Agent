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
        // A subagent's approvals belong to the root conversation: that is the
        // session an interactive client is watching. Depth-2 children resolve to
        // the root, not their intermediate parent. The summary names each role
        // on the path so the user knows who is asking.
        let (owner_session, summary) = self.approval_owner(session_id, summary)?;
        let approval_id = ApprovalId::generate();
        let now = Utc::now();
        let owned_by_parent = owner_session != *session_id;
        self.store.insert_approval(&ApprovalRecord {
            id: approval_id.clone(),
            session_id: owner_session.clone(),
            // A child run id would be foreign to the parent session's log.
            run_id: (!owned_by_parent).then(|| run_id.clone()),
            capability: capability.to_owned(),
            summary: summary.clone(),
            status: "pending".to_owned(),
            created_at: now,
            updated_at: now,
        })?;
        self.emit(
            &owner_session,
            (!owned_by_parent).then_some(run_id),
            (!owned_by_parent).then_some(turn_id),
            KnownPayload::ApprovalRequested(ApprovalRequested {
                approval_id: approval_id.clone(),
                capability: capability.to_owned(),
                summary: summary.clone(),
                policy_version: blora_policy::POLICY_VERSION,
            }),
        )?;
        loop {
            if cancel.is_cancelled() {
                self.settle_cancelled_approval(&owner_session, &approval_id)?;
                return Err(BloraError::Cancelled);
            }
            let record = self.store.get_approval(&approval_id)?;
            match record.status.as_str() {
                "allow" => {
                    self.emit(
                        &owner_session,
                        (!owned_by_parent).then_some(run_id),
                        (!owned_by_parent).then_some(turn_id),
                        KnownPayload::ApprovalResolved(ApprovalResolved {
                            approval_id,
                            decision: "allow".to_owned(),
                        }),
                    )?;
                    return Ok(true);
                }
                "deny" => {
                    self.emit(
                        &owner_session,
                        (!owned_by_parent).then_some(run_id),
                        (!owned_by_parent).then_some(turn_id),
                        KnownPayload::ApprovalResolved(ApprovalResolved {
                            approval_id,
                            decision: "deny".to_owned(),
                        }),
                    )?;
                    self.emit(
                        &owner_session,
                        (!owned_by_parent).then_some(run_id),
                        (!owned_by_parent).then_some(turn_id),
                        KnownPayload::PolicyDenied(PolicyDenied {
                            capability: capability.to_owned(),
                            reason: summary.clone(),
                        }),
                    )?;
                    return Ok(false);
                }
                _ => thread::sleep(Duration::from_millis(80)),
            }
        }
    }

    /// Resolve which session should display an approval raised by `session_id`,
    /// following the live subagent chain up to the first session that is not a
    /// child. Also prefixes every role on the chain.
    pub(crate) fn approval_owner(
        &self,
        session_id: &SessionId,
        summary: &str,
    ) -> Result<(SessionId, String)> {
        let owners = self
            .subagent_owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut current = session_id.clone();
        let mut roles = Vec::new();
        let mut hops = 0;
        while let Some((parent, role)) = owners.get(&current).cloned() {
            hops += 1;
            if hops > 8 {
                break;
            }
            roles.push(role);
            current = parent;
        }
        drop(owners);
        let summary = if roles.is_empty() {
            summary.to_owned()
        } else {
            roles.reverse();
            let tag = roles
                .iter()
                .map(|role| format!("subagent:{role}"))
                .collect::<Vec<_>>()
                .join(" > ");
            format!("[{tag}] {summary}")
        };
        Ok((current, summary))
    }

    /// Mark a pending approval denied and record the decision on the event log,
    /// so projections do not keep showing a pending approval forever.
    fn settle_cancelled_approval(
        &self,
        owner_session: &SessionId,
        approval_id: &ApprovalId,
    ) -> Result<()> {
        if self
            .store
            .resolve_approval_row(approval_id, "deny", Utc::now())?
        {
            self.emit(
                owner_session,
                None,
                None,
                KnownPayload::ApprovalResolved(ApprovalResolved {
                    approval_id: approval_id.clone(),
                    decision: "deny".to_owned(),
                }),
            )?;
        }
        Ok(())
    }
}
