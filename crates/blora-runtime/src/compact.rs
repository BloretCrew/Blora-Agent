// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Context compaction: model-written structured summary, verbatim tail, and a
//! three-strike circuit breaker.
//!
//! System blocks never enter the compaction input, so project rules survive by
//! construction. User messages are listed verbatim in the summary prompt so the
//! model cannot lose the operator's own words.

use blora_context::{compile_transcript, estimate_messages, split_at_compaction};
use blora_events::{
    ArtifactCreated, CheckpointCreated, ContextCompactionCompleted, ContextCompactionFailed,
    ContextCompactionStarted, KnownPayload,
};
use blora_model::{ChatMessage, CompletionRequest, Provider};
use blora_types::{ArtifactId, BloraError, CancelToken, Result, SessionId};
use chrono::Utc;

use crate::Runtime;

/// Consecutive failures after which auto-compaction stops trying.
pub const COMPACTION_BREAKER: u32 = 3;
/// Tokens of transcript tail kept verbatim after the summary.
const TAIL_TOKENS: u64 = 6_000;
/// A summary must shrink the history by at least this much to be accepted.
const MIN_REDUCTION_PCT: u64 = 20;

#[derive(Clone, Debug)]
pub struct CompactionReport {
    pub summary: String,
    pub tokens_before: u64,
    pub tokens_after: u64,
    pub preserve_from_sequence: Option<u64>,
}

impl Runtime {
    /// Manual compaction without a model (deterministic fallback summary).
    pub fn compact(&self, session_id: &SessionId) -> Result<String> {
        self.compact_with(session_id, None, &CancelToken::new(), "manual")
            .map(|report| report.summary)
    }

    /// Compact using `provider` when given, falling back to a deterministic
    /// summary when the model call fails.
    pub fn compact_with(
        &self,
        session_id: &SessionId,
        provider: Option<(&dyn Provider, &str)>,
        cancel: &CancelToken,
        reason: &str,
    ) -> Result<CompactionReport> {
        let events = self.store.load_events(session_id)?;
        let consecutive = consecutive_failures(&events);
        if reason != "manual" && consecutive >= COMPACTION_BREAKER {
            return Err(BloraError::Other(format!(
                "compaction breaker open after {consecutive} failures"
            )));
        }
        let hook = crate::hooks::run(
            crate::hooks::PRE_COMPACT,
            &serde_json::json!({"session_id": session_id.to_string(), "reason": reason}),
        );
        self.emit(
            session_id,
            None,
            None,
            KnownPayload::ContextCompactionStarted(ContextCompactionStarted {
                reason: Some(reason.to_owned()),
            }),
        )?;

        let split = split_at_compaction(&events);
        let transcript = compile_transcript(split.events)?;
        let tokens_before = estimate_messages(&transcript);
        let (head, tail_start_sequence) = split_tail(split.events, &transcript);
        let instructions = hook.additional_context.clone().unwrap_or_default();

        let attempt = match provider {
            Some((provider, model)) if !head.is_empty() => model_summary(
                provider,
                model,
                &head,
                split.summary.as_deref(),
                &instructions,
                cancel,
            ),
            _ => Err(BloraError::Other("no provider for compaction".to_owned())),
        };
        let (summary, degraded) = match attempt {
            Ok(summary) => (summary, false),
            Err(BloraError::Cancelled) => return Err(BloraError::Cancelled),
            Err(err) => {
                if provider.is_some() {
                    self.emit(
                        session_id,
                        None,
                        None,
                        KnownPayload::ContextCompactionFailed(ContextCompactionFailed {
                            error: err.to_string(),
                            consecutive_failures: consecutive + 1,
                        }),
                    )?;
                    if reason != "manual" {
                        return Err(err);
                    }
                }
                (fallback_summary(&head, split.summary.as_deref()), true)
            }
        };

        let summary_tokens = blora_context::estimate_tokens(&summary);
        let tail_tokens = tokens_before.saturating_sub(estimate_messages(&head));
        let tokens_after = summary_tokens + tail_tokens;
        let insufficient =
            tokens_before > 0 && tokens_after * 100 > tokens_before * (100 - MIN_REDUCTION_PCT);
        if !degraded && insufficient {
            let err = BloraError::Other(format!(
                "compaction did not reduce context enough ({tokens_before} -> {tokens_after})"
            ));
            self.emit(
                session_id,
                None,
                None,
                KnownPayload::ContextCompactionFailed(ContextCompactionFailed {
                    error: err.to_string(),
                    consecutive_failures: consecutive + 1,
                }),
            )?;
            return Err(err);
        }

        self.emit(
            session_id,
            None,
            None,
            KnownPayload::ContextCompactionCompleted(ContextCompactionCompleted {
                summary: summary.clone(),
                preserve_from_sequence: tail_start_sequence,
                tokens_before,
                tokens_after,
            }),
        )?;
        let artifact_id = ArtifactId::generate();
        self.store.insert_artifact(
            &artifact_id,
            session_id,
            "compaction",
            None,
            Some(&summary),
            Utc::now(),
        )?;
        self.emit(
            session_id,
            None,
            None,
            KnownPayload::ArtifactCreated(ArtifactCreated {
                artifact_id,
                kind: "compaction".to_owned(),
                path: "sqlite:artifacts".to_owned(),
            }),
        )?;
        self.checkpoint(session_id, None, Some("compaction"))?;
        let _ = self.distill_memories(session_id);
        Ok(CompactionReport {
            summary,
            tokens_before,
            tokens_after,
            preserve_from_sequence: tail_start_sequence,
        })
    }

    pub fn checkpoint(
        &self,
        session_id: &SessionId,
        run_id: Option<&blora_types::RunId>,
        note: Option<&str>,
    ) -> Result<()> {
        let projection = self.show_session(session_id)?;
        let sequence = projection.last_sequence;
        self.store
            .insert_checkpoint(session_id, run_id, sequence, note, Utc::now())?;
        self.emit(
            session_id,
            run_id,
            None,
            KnownPayload::CheckpointCreated(CheckpointCreated {
                sequence,
                note: note.map(ToOwned::to_owned),
            }),
        )?;
        Ok(())
    }
}

fn consecutive_failures(events: &[blora_events::EventEnvelope]) -> u32 {
    let mut count = 0;
    for event in events.iter().rev() {
        match event.event_type.as_str() {
            "context.compaction.failed" => count += 1,
            "context.compaction.completed" => break,
            _ => {}
        }
    }
    count
}

/// Split the transcript into (head to summarise, sequence of first kept event).
/// The tail is chosen by walking back from the end until `TAIL_TOKENS` is
/// exceeded, then snapping forward to a user-message boundary so no tool call
/// is separated from its result.
fn split_tail(
    events: &[blora_events::EventEnvelope],
    transcript: &[ChatMessage],
) -> (Vec<ChatMessage>, Option<u64>) {
    let mut budget = TAIL_TOKENS;
    let mut cut = transcript.len();
    for (index, message) in transcript.iter().enumerate().rev() {
        let cost = estimate_messages(std::slice::from_ref(message));
        if cost > budget {
            break;
        }
        budget -= cost;
        cut = index;
    }
    while cut < transcript.len() && transcript[cut].role != "user" {
        cut += 1;
    }
    if cut == 0 {
        // Nothing worth summarising separately; keep at most the last user turn.
        cut = transcript
            .iter()
            .rposition(|m| m.role == "user")
            .unwrap_or(transcript.len());
    }
    let head = transcript[..cut].to_vec();
    // Map the kept user message back to its event sequence.
    let kept_user_text = transcript.get(cut).and_then(|m| m.content.clone());
    let sequence = kept_user_text.and_then(|text| {
        events
            .iter()
            .filter(|event| event.event_type == "user.input")
            .filter(|event| {
                event
                    .payload
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|t| t == text)
            })
            .map(|event| event.sequence)
            .next_back()
    });
    (head, sequence)
}

fn model_summary(
    provider: &dyn Provider,
    model: &str,
    head: &[ChatMessage],
    previous: Option<&str>,
    instructions: &str,
    cancel: &CancelToken,
) -> Result<String> {
    let mut serialized = String::new();
    let mut user_messages = Vec::new();
    for message in head {
        let role = message.role.as_str();
        let text = message.content.as_deref().unwrap_or("");
        if role == "user" && !text.starts_with('<') {
            user_messages.push(text.to_owned());
        }
        serialized.push_str(&format!(
            "[{role}]: {}\n",
            blora_context::head_tail(text, 4_000)
        ));
        if let Some(calls) = &message.tool_calls {
            for call in calls {
                serialized.push_str(&format!(
                    "[tool_call]: {}({})\n",
                    call.name,
                    blora_context::head_tail(&call.arguments, 500)
                ));
            }
        }
    }
    let mut prompt = String::from("[compaction request]\n");
    prompt.push_str(
        "Write a handoff summary for another engineer who will continue this session. Use exactly these sections:\n\
         ## Goal\n## Constraints\n## Progress (done / in progress / blocked)\n## Key decisions\n## Files and symbols (exact paths, function names, error strings)\n## All user messages (verbatim, in order)\n## Next steps\n\
         Be terse. Do not mention the compaction process. Preserve identifiers exactly.\n",
    );
    if !instructions.trim().is_empty() {
        prompt.push_str("Extra focus requested by the operator:\n");
        prompt.push_str(instructions.trim());
        prompt.push('\n');
    }
    if let Some(previous) = previous {
        prompt.push_str(
            "\n<prior_summary>\nPreserve everything still true; update or drop what changed.\n",
        );
        prompt.push_str(previous);
        prompt.push_str("\n</prior_summary>\n");
    }
    prompt.push_str("\n<user_messages>\n");
    for text in &user_messages {
        prompt.push_str("- ");
        prompt.push_str(&blora_context::head_tail(text, 1_000));
        prompt.push('\n');
    }
    prompt.push_str("</user_messages>\n\n<conversation>\n");
    prompt.push_str(&serialized);
    prompt.push_str("</conversation>");

    let request = CompletionRequest {
        model: model.to_owned(),
        messages: vec![ChatMessage::text("user", prompt)],
        tools: Vec::new(),
        max_output_tokens: Some(4_000),
        cache_key: None,
    };
    let completion = provider.complete(&request, cancel, &mut |_| Ok(()))?;
    let summary = completion.text.trim().to_owned();
    if summary.chars().count() < 40 {
        return Err(BloraError::provider("degenerate compaction summary"));
    }
    Ok(summary)
}

fn fallback_summary(head: &[ChatMessage], previous: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(previous) = previous {
        out.push_str("## Prior summary\n");
        out.push_str(previous);
        out.push_str("\n\n");
    }
    out.push_str("## All user messages\n");
    for message in head.iter().filter(|m| m.role == "user") {
        if let Some(text) = &message.content {
            if !text.starts_with('<') {
                out.push_str("- ");
                out.push_str(&blora_context::head_tail(text, 400));
                out.push('\n');
            }
        }
    }
    out.push_str("## Tool activity\n");
    for message in head {
        if let Some(calls) = &message.tool_calls {
            for call in calls {
                out.push_str(&format!(
                    "- {}({})\n",
                    call.name,
                    blora_context::head_tail(&call.arguments, 120)
                ));
            }
        }
    }
    out.push_str("## Last assistant message\n");
    if let Some(text) = head
        .iter()
        .rev()
        .find(|m| m.role == "assistant" && m.content.is_some())
        .and_then(|m| m.content.as_deref())
    {
        out.push_str(&blora_context::head_tail(text, 800));
    } else {
        out.push_str("(none)");
    }
    out
}
