// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! `ask_user`: the agent asks one or more questions and the run waits.
//!
//! The questionnaire and its answer are canonical events (`input.requested`,
//! `input.provided`). The wait itself is in-process: interfaces answer through
//! [`Runtime::answer_input`] on the same runtime that is running the turn.

use std::path::Path;
use std::thread;
use std::time::Duration;

use blora_events::{AskedQuestion, InputProvided, InputRequested, KnownPayload, QuestionChoice};
use blora_policy::Policy;
use blora_types::{BloraError, CancelToken, Result, RunId, SessionId, TurnId};
use serde_json::Value;

use crate::Runtime;

/// Answer returned to the model when nobody can be asked.
pub const NO_USER: &str = "The user is not available to answer in this run (non-interactive). Make a reasonable, reversible choice, state the assumption in your final answer, and continue.";

const MAX_QUESTIONS: usize = 6;
const MAX_OPTIONS: usize = 6;

impl Runtime {
    /// Record an answer (or a dismissal when `answer` is `None`) for the open
    /// question of `session_id`. Fails when the question is not open.
    pub fn answer_input(
        &self,
        session_id: &SessionId,
        request_id: &str,
        answer: Option<String>,
    ) -> Result<()> {
        let projection = self.show_session(session_id)?;
        let pending = projection.pending_input.clone();
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
        if let Some(text) = &answer {
            if text.chars().count() > 16_000 {
                return Err(BloraError::Other("answer too long".to_owned()));
            }
            if let (Some(pending), Some(session)) = (&pending, projection.session.as_ref()) {
                validate_question_answer(
                    Path::new(&session.workspace_path),
                    &pending.questions,
                    text,
                )?;
            }
        }
        let run_id = projection
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
    pub(crate) fn ask_user(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        call_id: &str,
        questions: Vec<AskedQuestion>,
        interactive: bool,
        cancel: &CancelToken,
    ) -> Result<String> {
        if questions.is_empty() {
            return Err(BloraError::Other("ask_user needs a question".to_owned()));
        }
        if !interactive {
            return Ok(NO_USER.to_owned());
        }
        let summary = questions
            .iter()
            .map(|question| question.question.clone())
            .collect::<Vec<_>>()
            .join("\n");
        let options = questions
            .first()
            .map(|question| {
                question
                    .options
                    .iter()
                    .map(|choice| choice.label.clone())
                    .collect()
            })
            .unwrap_or_default();
        let request_id = format!("ask_{}", uuid_like(call_id));
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::InputRequested(InputRequested {
                request_id: request_id.clone(),
                question: summary.chars().take(2_000).collect(),
                options,
                call_id: Some(call_id.to_owned()),
                questions,
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
                    Some(text) => format!("User answered:\n{text}"),
                    None => "The user dismissed the question without answering. Proceed with your best judgement and say which assumption you made.".to_owned(),
                });
            }
            thread::sleep(Duration::from_millis(80));
        }
    }
}

/// Turn a tool-call payload into the questionnaire stored on the event.
pub fn normalize_questions(arguments: &Value) -> Result<Vec<AskedQuestion>> {
    if let Some(list) = arguments.get("questions").and_then(Value::as_array) {
        if list.is_empty() {
            return Err(BloraError::Other(
                "ask_user needs at least one question".to_owned(),
            ));
        }
        if list.len() > MAX_QUESTIONS {
            return Err(BloraError::Other(format!(
                "ask_user accepts at most {MAX_QUESTIONS} questions"
            )));
        }
        let mut questions = Vec::with_capacity(list.len());
        for item in list {
            questions.push(normalize_one(item)?);
        }
        assign_ids(&mut questions);
        return Ok(questions);
    }
    let question = arguments
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    if question.is_empty() {
        return Err(BloraError::Other("ask_user needs a question".to_owned()));
    }
    let mut legacy = serde_json::json!({
        "question": question,
        "type": if arguments.get("options").and_then(Value::as_array).is_some_and(|items| !items.is_empty()) {
            "single"
        } else {
            "text"
        }
    });
    if let Some(options) = arguments.get("options") {
        legacy["options"] = options.clone();
    }
    let mut questions = vec![normalize_one(&legacy)?];
    assign_ids(&mut questions);
    Ok(questions)
}

/// One line per question: `1. Title：value`.
pub fn format_question_answers(questions: &[AskedQuestion], values: &[String]) -> Result<String> {
    if questions.len() != values.len() {
        return Err(BloraError::Other(
            "every question needs an answer".to_owned(),
        ));
    }
    Ok(questions
        .iter()
        .zip(values)
        .enumerate()
        .map(|(index, (question, value))| {
            let value = value.replace(['\n', '\r'], " ");
            format!(
                "{}. {}：{}",
                index + 1,
                question.answer_title(),
                value.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

/// Folder and file answers must name a real path inside `workspace`.
pub fn validate_question_answer(
    workspace: &Path,
    questions: &[AskedQuestion],
    answer: &str,
) -> Result<()> {
    if !questions
        .iter()
        .any(|question| question.kind == "folder" || question.kind == "file")
    {
        return Ok(());
    }
    let lines: Vec<&str> = answer.lines().collect();
    if lines.len() != questions.len() {
        return Err(BloraError::Other(
            "answer does not match the open questions".to_owned(),
        ));
    }
    for (index, question) in questions.iter().enumerate() {
        if question.kind != "folder" && question.kind != "file" {
            continue;
        }
        let prefix = format!("{}. {}：", index + 1, question.answer_title());
        let value = lines[index].strip_prefix(&prefix).ok_or_else(|| {
            BloraError::Other("answer does not match the open questions".to_owned())
        })?;
        checked_relative(workspace, value, &question.kind)?;
    }
    Ok(())
}

fn normalize_one(item: &Value) -> Result<AskedQuestion> {
    let question = clean_question(item.get("question").and_then(Value::as_str).unwrap_or(""))?;
    let kind = canonical_kind(
        item.get("type")
            .and_then(Value::as_str)
            .or_else(|| item.get("kind").and_then(Value::as_str))
            .unwrap_or(if item.get("options").and_then(Value::as_array).is_some() {
                "single"
            } else {
                "text"
            }),
    )?;
    let header = item
        .get("header")
        .and_then(Value::as_str)
        .and_then(clean_header);
    let options = match kind.as_str() {
        "single" | "multi" => choice_options(item.get("options"), true)?,
        "boolean" => boolean_options(),
        _ => Vec::new(),
    };
    Ok(AskedQuestion {
        id: item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        kind,
        question,
        header,
        options,
    })
}

fn canonical_kind(raw: &str) -> Result<String> {
    let kind = match raw.trim().to_ascii_lowercase().as_str() {
        "single" | "single_select" | "choice" => "single",
        "multi" | "multiple" | "multi_select" | "multiselect" => "multi",
        "boolean" | "bool" | "yesno" | "judge" => "boolean",
        "text" | "subjective" | "open" | "free" => "text",
        "folder" | "directory" | "dir" => "folder",
        "file" => "file",
        other => {
            return Err(BloraError::Other(format!("unknown question type {other}")));
        }
    };
    Ok(kind.to_owned())
}

fn choice_options(value: Option<&Value>, require: bool) -> Result<Vec<QuestionChoice>> {
    let Some(items) = value.and_then(Value::as_array) else {
        return if require {
            Err(BloraError::Other(
                "single and multi questions need 2 to 6 options".to_owned(),
            ))
        } else {
            Ok(Vec::new())
        };
    };
    let mut options = Vec::new();
    for item in items {
        let (label, description) = match item {
            Value::String(label) => (label.clone(), None),
            Value::Object(_) => (
                item.get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                item.get("description")
                    .and_then(Value::as_str)
                    .map(|text| text.chars().take(160).collect::<String>())
                    .filter(|text| !text.trim().is_empty()),
            ),
            _ => continue,
        };
        let label: String = label.trim().chars().take(80).collect();
        if label.is_empty() || is_other_label(&label) {
            continue;
        }
        if options.len() == MAX_OPTIONS {
            break;
        }
        options.push(QuestionChoice {
            label,
            description,
            other: false,
        });
    }
    if options.len() < 2 {
        return Err(BloraError::Other(
            "single and multi questions need 2 to 6 options".to_owned(),
        ));
    }
    options.push(other_choice());
    Ok(options)
}

fn boolean_options() -> Vec<QuestionChoice> {
    vec![
        QuestionChoice {
            label: "是".to_owned(),
            description: None,
            other: false,
        },
        QuestionChoice {
            label: "否".to_owned(),
            description: None,
            other: false,
        },
        other_choice(),
    ]
}

fn other_choice() -> QuestionChoice {
    QuestionChoice {
        label: "其他".to_owned(),
        description: None,
        other: true,
    }
}

fn is_other_label(label: &str) -> bool {
    matches!(
        label.trim().to_ascii_lowercase().as_str(),
        "其他" | "其它" | "other"
    )
}

fn clean_question(raw: &str) -> Result<String> {
    let text: String = raw.trim().chars().take(2_000).collect();
    if text.is_empty() {
        Err(BloraError::Other("ask_user needs a question".to_owned()))
    } else {
        Ok(text)
    }
}

fn clean_header(raw: &str) -> Option<String> {
    let text: String = raw
        .chars()
        .filter(|ch| *ch != '：' && *ch != ':' && *ch != '\n' && *ch != '\r')
        .take(16)
        .collect();
    let text = text.trim().to_owned();
    if text.is_empty() { None } else { Some(text) }
}

fn assign_ids(questions: &mut [AskedQuestion]) {
    let mut seen = std::collections::HashSet::new();
    for (index, question) in questions.iter_mut().enumerate() {
        let cleaned: String = question
            .id
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
            .take(24)
            .collect();
        let mut id = cleaned;
        if id.is_empty() || seen.contains(&id) {
            let mut number = index + 1;
            id = format!("q{number}");
            while seen.contains(&id) {
                number += 1;
                id = format!("q{number}");
            }
        }
        seen.insert(id.clone());
        question.id = id;
    }
}

fn checked_relative(workspace: &Path, raw: &str, kind: &str) -> Result<String> {
    let raw = raw.trim().trim_end_matches(['/', '\\']);
    if raw.is_empty() || raw == "." {
        return if kind == "folder" {
            Ok(".".to_owned())
        } else {
            Err(BloraError::Other(
                "choose a file inside the workspace".to_owned(),
            ))
        };
    }
    if raw.starts_with('/')
        || raw.starts_with('\\')
        || raw.chars().nth(1) == Some(':')
        || raw
            .split(['/', '\\'])
            .any(|part| part.is_empty() || part == "..")
    {
        return Err(BloraError::Other(
            "path must stay inside the workspace".to_owned(),
        ));
    }
    let policy = Policy::new(workspace, true)?;
    let resolved = policy.resolve(raw)?;
    let ok = if kind == "folder" {
        resolved.is_dir()
    } else {
        resolved.is_file()
    };
    if !ok {
        return Err(BloraError::Other(format!(
            "choose a {kind} inside the workspace"
        )));
    }
    let relative = resolved
        .strip_prefix(policy.workspace())
        .unwrap_or(Path::new(raw));
    let text = relative.to_string_lossy().replace('\\', "/");
    Ok(if text.is_empty() {
        ".".to_owned()
    } else {
        text
    })
}

/// Stable, collision-resistant request id derived from the call id and time.
fn uuid_like(call_id: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let safe: String = call_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(24)
        .collect();
    format!("{safe}_{nanos:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_mixed_questions_and_appends_other() {
        let questions = normalize_questions(&json!({
            "questions": [
                {"type": "single", "header": "数据库", "question": "用哪个？", "options": [{"label": "SQLite", "description": "本地"}, "Other", {"label": "Postgres"}]},
                {"type": "multi", "question": "要哪些功能？", "options": ["日志", "指标"]},
                {"type": "boolean", "question": "现在发布？", "options": ["也许"]},
                {"type": "text", "question": "还有什么？"},
                {"type": "folder", "header": "目录", "question": "放在哪？"},
                {"type": "file", "question": "改哪个文件？"}
            ]
        }))
        .unwrap();
        assert_eq!(questions.len(), 6);
        assert_eq!(questions[0].options.len(), 3);
        assert!(questions[0].options.last().unwrap().other);
        assert!(
            questions[0]
                .options
                .iter()
                .all(|choice| choice.label != "Other")
        );
        assert_eq!(questions[1].kind, "multi");
        assert!(questions[1].options.last().unwrap().other);
        assert_eq!(
            questions[2]
                .options
                .iter()
                .map(|choice| choice.label.as_str())
                .collect::<Vec<_>>(),
            vec!["是", "否", "其他"]
        );
        assert!(questions[3].options.is_empty());
        assert!(questions[4].options.is_empty() && questions[5].options.is_empty());
        let text = format_question_answers(
            &questions,
            &[
                "Postgres".into(),
                "日志, 自建追踪".into(),
                "是".into(),
                "先写测试".into(),
                "crates/blora-tui".into(),
                "Cargo.toml".into(),
            ],
        )
        .unwrap();
        assert_eq!(
            text,
            "1. 数据库：Postgres\n2. 要哪些功能？：日志, 自建追踪\n3. 现在发布？：是\n4. 还有什么？：先写测试\n5. 目录：crates/blora-tui\n6. 改哪个文件？：Cargo.toml"
        );
    }

    #[test]
    fn legacy_shape_becomes_one_question() {
        let choice = normalize_questions(
            &json!({"question": "Which database?", "options": ["SQLite", "Postgres"]}),
        )
        .unwrap();
        assert_eq!(choice[0].kind, "single");
        assert!(choice[0].options.last().unwrap().other);
        let open = normalize_questions(&json!({"question": "Proceed?"})).unwrap();
        assert_eq!(open[0].kind, "text");
        assert!(normalize_questions(&json!({"questions": []})).is_err());
        assert!(
            normalize_questions(
                &json!({"questions": [{"type": "single", "question": "Q?", "options": ["only"]}]} )
            )
            .is_err()
        );
    }

    #[test]
    fn path_answers_must_stay_inside_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("a.txt"), "a").unwrap();
        let questions = normalize_questions(&json!({
            "questions": [
                {"type": "text", "question": "备注？"},
                {"type": "folder", "header": "目录", "question": "放哪？"},
                {"type": "file", "question": "哪个文件？"}
            ]
        }))
        .unwrap();
        let good =
            format_question_answers(&questions, &["ok".into(), "sub".into(), "sub/a.txt".into()])
                .unwrap();
        assert!(validate_question_answer(dir.path(), &questions, &good).is_ok());
        for bad in ["../secret", "/etc/passwd", "sub/missing.txt", "nope"] {
            let answer =
                format_question_answers(&questions, &["ok".into(), "sub".into(), bad.into()])
                    .unwrap();
            assert!(
                validate_question_answer(dir.path(), &questions, &answer).is_err(),
                "{bad}"
            );
        }
        let outside =
            format_question_answers(&questions, &["ok".into(), "..".into(), "sub/a.txt".into()])
                .unwrap();
        assert!(validate_question_answer(dir.path(), &questions, &outside).is_err());
    }
}
