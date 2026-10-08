// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! `ask_user`: the agent asks a question and the run waits for the answer.
//!
//! The question and its answer are canonical events (`input.requested`,
//! `input.provided`). The wait itself is in-process: interfaces answer through
//! [`Runtime::answer_input`] on the same runtime that is running the turn.

use std::thread;
use std::time::Duration;

use blora_events::{InputProvided, InputRequested, KnownPayload};
use blora_types::{BloraError, CancelToken, Result, RunId, SessionId, TurnId};

use crate::Runtime;

/// Answer returned to the model when nobody can be asked.
pub const NO_USER: &str = "The user is not available to answer in this run (non-interactive). Make a reasonable, reversible choice, state the assumption in your final answer, and continue.";

impl Runtime {
    /// Record an answer (or a dismissal when `answer` is `None`) for the open
    /// question of `session_id`. Fails when the question is not open.
    pub fn answer_input(
        &self,
        session_id: &SessionId,
        request_id: &str,
        answer: Option<String>,
    ) -> Result<()> {
        let pending = self.show_session(session_id)?.pending_input;
        if pending
            .as_ref()
            .is_none_or(|pending| pending.request_id != request_id)
        {
            return Err(BloraError::Other(format!(
                "question {request_id} is not waiting for an answer"
            )));
        }
        let answer = answer
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty());
        let run_id = self
            .show_session(session_id)?
            .runs
            .iter()
            .rev()
            .find(|run| run.status == blora_types::RunStatus::WaitingInput)
            .map(|run| run.id.clone());
        self.emit(
            session_id,
            run_id.as_ref(),
            None,
            KnownPayload::InputProvided(InputProvided {
                request_id: request_id.to_owned(),
                answer: answer.clone(),
            }),
        )?;
        self.input_answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(request_id.to_owned(), answer);
        Ok(())
    }

    /// Ask the user and block until they answer, dismiss, or the run is cancelled.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ask_user(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        call_id: &str,
        question: &str,
        options: Vec<String>,
        interactive: bool,
        cancel: &CancelToken,
    ) -> Result<String> {
        let question = question.trim();
        if question.is_empty() {
            return Err(BloraError::Other("ask_user needs a question".to_owned()));
        }
        if !interactive {
            return Ok(NO_USER.to_owned());
        }
        let options: Vec<String> = options
            .into_iter()
            .map(|option| option.trim().chars().take(120).collect::<String>())
            .filter(|option| !option.is_empty())
            .take(6)
            .collect();
        let request_id = format!("ask_{}", uuid_like(call_id));
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::InputRequested(InputRequested {
                request_id: request_id.clone(),
                question: question.chars().take(2_000).collect(),
                options,
                call_id: Some(call_id.to_owned()),
            }),
        )?;
        loop {
            if cancel.is_cancelled() {
                return Err(BloraError::Cancelled);
            }
            let answered = self
                .input_answers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&request_id);
            if let Some(answer) = answered {
                return Ok(match answer {
                    Some(text) => format!("User answered: {text}"),
                    None => "The user dismissed the question without answering. Proceed with your best judgement and say which assumption you made.".to_owned(),
                });
            }
            thread::sleep(Duration::from_millis(80));
        }
    }
}

/// Stable, collision-resistant request id derived from the call id and time.
fn uuid_like(call_id: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let safe: String = call_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(24)
        .collect();
    format!("{safe}_{nanos:x}")
}
