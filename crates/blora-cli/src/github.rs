// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::path::{Path, PathBuf};

use blora_github::{
    AgentDriver, AgentRequest, AgentResponse, CommandRepo, Config, DEFAULT_MENTIONS, Status,
    UreqHttp, WorkflowOptions, execute, install_instructions, install_workflow,
};
use blora_runtime::{CancelToken, PermissionMode, RunOptions, Runtime};
use blora_session::TranscriptItem;
use blora_storage::CreateSession;
use blora_types::{Mode, SessionId};

pub fn install(
    workspace: &Path,
    force: bool,
    provider: Option<String>,
    model: Option<String>,
    permission: String,
    mentions: String,
) -> Result<(), Box<dyn std::error::Error>> {
    if PermissionMode::parse(&permission).is_none() {
        return Err(format!("无法识别的权限模式：{permission}").into());
    }
    let opts = WorkflowOptions {
        provider: provider.unwrap_or_default(),
        model: model.unwrap_or_default(),
        permission,
        mentions: blora_github_mentions(&mentions),
        action_ref: "main".into(),
    };
    let path = install_workflow(workspace, &opts, force)?;
    println!("{}", install_instructions(&path));
    Ok(())
}

pub fn run(
    runtime: &Runtime,
    event: Option<PathBuf>,
    token: Option<String>,
    dry_run: bool,
    workspace: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = workspace.unwrap_or(std::env::current_dir()?);
    let event_path = event
        .or_else(|| std::env::var("GITHUB_EVENT_PATH").ok().map(PathBuf::from))
        .ok_or("找不到 GitHub 事件。Actions 会提供 GITHUB_EVENT_PATH，本地试跑请传入 --event。")?;
    let raw = std::fs::read_to_string(&event_path)
        .map_err(|err| format!("无法读取 {}：{err}", event_path.display()))?;
    let event_json: serde_json::Value =
        serde_json::from_str(&raw).map_err(|err| format!("GitHub 事件不是 JSON：{err}"))?;
    let permission = env_nonempty("BLORA_GITHUB_PERMISSION").unwrap_or_else(|| "yolo".into());
    if PermissionMode::parse(&permission).is_none() {
        return Err(format!("无法识别的权限模式：{permission}").into());
    }
    let max_turns = env_nonempty("BLORA_GITHUB_MAX_TURNS")
        .as_deref()
        .map(str::parse::<u32>)
        .transpose()
        .map_err(|_| "BLORA_GITHUB_MAX_TURNS 必须是正整数".to_string())?
        .unwrap_or(24);
    let token = token
        .or_else(|| env_nonempty("BLORA_GITHUB_TOKEN"))
        .or_else(|| env_nonempty("GITHUB_TOKEN"));
    let attachment_dir = std::env::var_os("RUNNER_TEMP")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("blora-attachments");
    let session_export_path =
        std::env::var_os("RUNNER_TEMP").map(|dir| PathBuf::from(dir).join("blora-session.json"));
    let mut config = Config::new(event_json);
    config.event_name = env_nonempty("GITHUB_EVENT_NAME");
    config.repository = env_nonempty("GITHUB_REPOSITORY");
    config.actor = env_nonempty("GITHUB_ACTOR");
    config.run_id = env_nonempty("GITHUB_RUN_ID").unwrap_or_else(|| "local".into());
    config.token = token;
    config.mentions = blora_github_mentions(
        env_nonempty("MENTIONS")
            .as_deref()
            .unwrap_or(DEFAULT_MENTIONS),
    );
    config.prompt_override = env_nonempty("PROMPT");
    config.permission = permission;
    config.provider = env_nonempty("BLORA_PROVIDER").unwrap_or_default();
    config.model = env_nonempty("BLORA_MODEL").unwrap_or_default();
    config.max_turns = max_turns;
    config.dry_run = dry_run;
    config.attachment_dir = attachment_dir;
    config.session_export_path = session_export_path;
    config.workspace = workspace.clone();
    config.now = chrono::Utc::now();

    let report = execute(
        &config,
        &UreqHttp,
        &CommandRepo::new(&workspace),
        &RuntimeAgent { runtime },
    )?;
    match report.status {
        Status::Failed => Err(report.detail.into()),
        Status::DryRun => {
            println!("{}", report.prompt);
            Ok(())
        }
        Status::Skipped | Status::Completed => {
            println!("{}", report.detail);
            Ok(())
        }
    }
}

fn blora_github_mentions(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

struct RuntimeAgent<'a> {
    runtime: &'a Runtime,
}

impl AgentDriver for RuntimeAgent<'_> {
    fn run(&self, request: &AgentRequest<'_>) -> Result<AgentResponse, blora_github::GithubError> {
        let permission = PermissionMode::parse(request.permission).ok_or_else(|| {
            blora_github::GithubError::new(format!("无法识别的权限模式：{}", request.permission))
        })?;
        let session_id = self
            .runtime
            .create_session(CreateSession {
                title: Some(request.title.to_string()),
                workspace_path: request.workspace.display().to_string(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .map_err(|err| blora_github::GithubError::new(err.to_string()))?;
        self.runtime
            .run(
                &session_id,
                request.prompt,
                &CancelToken::new(),
                &RunOptions {
                    model: request.model.to_string(),
                    mock: false,
                    auto_approve: permission.auto_approve(),
                    interactive: false,
                    max_turns: request.max_turns.max(1),
                    provider: request.provider.to_string(),
                    read_only: request.read_only,
                    worktree: false,
                    passport_user_token: None,
                    permission: Some(permission),
                },
            )
            .map_err(|err| blora_github::GithubError::new(err.to_string()))?;
        let projection = self
            .runtime
            .show_session(&session_id)
            .map_err(|err| blora_github::GithubError::new(err.to_string()))?;
        let text = projection
            .transcript
            .iter()
            .rev()
            .find_map(|item| match item {
                TranscriptItem::Assistant { text, .. } if !text.trim().is_empty() => {
                    Some(text.clone())
                }
                _ => None,
            })
            .unwrap_or_default();
        Ok(AgentResponse {
            session_id: session_id.to_string(),
            text,
        })
    }

    fn export(&self, session_id: &str, path: &Path) -> Result<(), blora_github::GithubError> {
        let id = SessionId::parse(session_id)
            .map_err(|err| blora_github::GithubError::new(err.to_string()))?;
        let value = self
            .runtime
            .export_session(&id)
            .map_err(|err| blora_github::GithubError::new(err.to_string()))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| blora_github::GithubError::new(format!("导出会话失败：{err}")))?;
        }
        let body = serde_json::to_string_pretty(&value)
            .map_err(|err| blora_github::GithubError::new(format!("导出会话失败：{err}")))?;
        std::fs::write(path, body)
            .map_err(|err| blora_github::GithubError::new(format!("导出会话失败：{err}")))?;
        Ok(())
    }
}
