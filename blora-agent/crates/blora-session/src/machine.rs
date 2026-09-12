// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_events::KnownPayload;
use blora_types::{BloraError, Result, RunStatus};

/// Returns the next run status for a known event, or `None` if the event does
/// not change run status.
pub fn transition_run(current: RunStatus, payload: &KnownPayload) -> Result<Option<RunStatus>> {
    let next = match payload {
        KnownPayload::RunStarted(_) => Some(RunStatus::Running),
        KnownPayload::RunPaused(_) => Some(RunStatus::Paused),
        KnownPayload::ApprovalRequested(_) => Some(RunStatus::WaitingApproval),
        KnownPayload::ApprovalResolved(_) => Some(RunStatus::Running),
        KnownPayload::TaskWakeup(_) => Some(RunStatus::Running),
        KnownPayload::ContextCompactionStarted(_) => Some(RunStatus::Compacting),
        KnownPayload::ContextCompactionCompleted(_) => Some(RunStatus::Running),
        KnownPayload::RunCompleted(_) => Some(RunStatus::Completed),
        KnownPayload::RunFailed(_) => Some(RunStatus::Failed),
        KnownPayload::RunCancelled(_) => Some(RunStatus::Cancelled),
        KnownPayload::RunCancelRequested(_) => None,
        _ => None,
    };

    if let Some(target) = next {
        if current.is_terminal() && target != current {
            return Err(BloraError::IllegalTransition {
                from: current.as_str().to_owned(),
                to: target.as_str().to_owned(),
            });
        }
        if !is_allowed(current, target) {
            return Err(BloraError::IllegalTransition {
                from: current.as_str().to_owned(),
                to: target.as_str().to_owned(),
            });
        }
    }
    Ok(next)
}

fn is_allowed(from: RunStatus, to: RunStatus) -> bool {
    if from == to {
        return true;
    }
    match (from, to) {
        (RunStatus::Queued, RunStatus::Running | RunStatus::Cancelled | RunStatus::Failed) => true,
        (
            RunStatus::Running,
            RunStatus::WaitingApproval
            | RunStatus::WaitingInput
            | RunStatus::WaitingTask
            | RunStatus::Compacting
            | RunStatus::Paused
            | RunStatus::Completed
            | RunStatus::Failed
            | RunStatus::Cancelled,
        ) => true,
        (
            RunStatus::WaitingApproval
            | RunStatus::WaitingInput
            | RunStatus::WaitingTask
            | RunStatus::Compacting
            | RunStatus::Paused,
            RunStatus::Running | RunStatus::Cancelled | RunStatus::Failed,
        ) => true,
        (_, _) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_events::RunStarted;

    #[test]
    fn queued_to_running() {
        let next = transition_run(
            RunStatus::Queued,
            &KnownPayload::RunStarted(RunStarted { model: None }),
        )
        .unwrap();
        assert_eq!(next, Some(RunStatus::Running));
    }

    #[test]
    fn completed_is_terminal() {
        let err = transition_run(
            RunStatus::Completed,
            &KnownPayload::RunStarted(RunStarted { model: None }),
        )
        .unwrap_err();
        assert!(matches!(err, BloraError::IllegalTransition { .. }));
    }
}
