// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::path::{Path, PathBuf};

use crate::error::{GithubError, Result};
use crate::mention::{self, DEFAULT_MENTIONS};

#[derive(Clone, Debug)]
pub struct WorkflowOptions {
    pub provider: String,
    pub model: String,
    pub permission: String,
    pub mentions: Vec<String>,
    pub action_ref: String,
}

impl Default for WorkflowOptions {
    fn default() -> Self {
        Self {
            provider: String::new(),
            model: String::new(),
            permission: "yolo".into(),
            mentions: mention::parse_mention_list(DEFAULT_MENTIONS),
            action_ref: "main".into(),
        }
    }
}

#[must_use]
pub fn render_workflow(opts: &WorkflowOptions) -> String {
    let mentions = if opts.mentions.is_empty() {
        mention::parse_mention_list(DEFAULT_MENTIONS)
    } else {
        opts.mentions.clone()
    };
    let mut needles = Vec::new();
    for mention in &mentions {
        for variant in variants(mention) {
            if variant.contains('\'') || variant.contains('\n') {
                continue;
            }
            if !needles.iter().any(|existing: &String| existing == &variant) {
                needles.push(variant);
            }
        }
    }
    let condition = needles
        .iter()
        .map(|needle| format!("contains(github.event.comment.body, '{needle}')"))
        .collect::<Vec<_>>()
        .join(" ||\n      ");
    let mention_value = mentions.join(",");
    let permission = if opts.permission.trim().is_empty() {
        "yolo"
    } else {
        opts.permission.trim()
    };
    let action_ref = if opts.action_ref.trim().is_empty() {
        "main"
    } else {
        opts.action_ref.trim()
    };
    let mut extra_env = String::new();
    if !opts.provider.trim().is_empty() {
        extra_env.push_str(&format!(
            "\n          BLORA_PROVIDER: \"{}\"",
            yaml_quote(opts.provider.trim())
        ));
    }
    if !opts.model.trim().is_empty() {
        extra_env.push_str(&format!(
            "\n          BLORA_MODEL: \"{}\"",
            yaml_quote(opts.model.trim())
        ));
    }
    format!(
        r#"name: blora

on:
  issue_comment:
    types: [created]
  pull_request_review_comment:
    types: [created]

concurrency:
  group: blora-${{{{ github.event.issue.number || github.event.pull_request.number || github.run_id }}}}
  cancel-in-progress: false

jobs:
  blora:
    if: |
      {condition}
    runs-on: ubuntu-latest
    timeout-minutes: 30
    permissions:
      contents: write
      issues: write
      pull-requests: write
    steps:
      - name: Checkout repository
        uses: actions/checkout@v4
        with:
          fetch-depth: 1
          persist-credentials: false

      - name: Run Blora Agent
        uses: bloret-crew/blora-agent/github@{action_ref}
        env:
          GITHUB_TOKEN: ${{{{ secrets.GITHUB_TOKEN }}}}
          BLORA_API_KEY: ${{{{ secrets.BLORA_API_KEY }}}}
          BLORA_NETWORK: "1"{extra_env}
        with:
          mentions: "{mention_quoted}"
          permission: {permission}
"#,
        mention_quoted = yaml_quote(&mention_value),
    )
}

pub fn install_workflow(workspace: &Path, opts: &WorkflowOptions, force: bool) -> Result<PathBuf> {
    let path = workspace.join(".github/workflows/blora.yml");
    if path.exists() && !force {
        return Err(GithubError::new(format!(
            "工作流已存在：{}。加上 --force 才会覆盖。",
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| GithubError::new(format!("无法创建目录：{err}")))?;
    }
    std::fs::write(&path, render_workflow(opts))
        .map_err(|err| GithubError::new(format!("无法写入工作流：{err}")))?;
    Ok(path)
}

#[must_use]
pub fn install_instructions(path: &Path) -> String {
    format!(
        "已写入 {}\n\n下一步：\n1. 提交并推送 .github/workflows/blora.yml\n2. 在仓库的 Actions secrets 里添加 BLORA_API_KEY。需要指定供应商时，重新运行 install 并加上 --provider 与 --model，或在仓库变量里设置 BLORA_PROVIDER 和 BLORA_MODEL。\n3. 用有写权限的账号在 Issue 或 Pull Request 里评论。例如：/blora 总结这个问题，或 @blora 修好登录失败。\n\n触发词默认是 /blora、/ba 和 @blora。评论里的要求决定 Blora 只回复，还是改代码、推送、打开 Pull Request。工作流不会自动开 PR。",
        path.display()
    )
}

fn variants(mention: &str) -> Vec<String> {
    let mut out = vec![
        mention.to_string(),
        mention.to_lowercase(),
        mention.to_uppercase(),
    ];
    if let Some(rest) = mention
        .strip_prefix('@')
        .or_else(|| mention.strip_prefix('/'))
    {
        let sigil = &mention[..mention.len() - rest.len()];
        let mut chars = rest.chars();
        if let Some(first) = chars.next() {
            out.push(format!(
                "{sigil}{}{}",
                first.to_uppercase(),
                chars.as_str().to_lowercase()
            ));
        }
    }
    out
}

fn yaml_quote(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_mentions_at_blora_and_does_not_schedule_itself() {
        let yaml = render_workflow(&WorkflowOptions::default());
        assert!(yaml.contains("@blora"));
        assert!(yaml.contains("/blora"));
        assert!(yaml.contains("/ba"));
        assert!(yaml.contains("contents: write"));
        assert!(yaml.contains("persist-credentials: false"));
        assert!(yaml.contains("BLORA_NETWORK"));
        assert!(!yaml.contains("\n  schedule:"));
        assert!(!yaml.contains("BLORA_PROVIDER"));
    }

    #[test]
    fn install_refuses_to_overwrite_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let opts = WorkflowOptions {
            provider: "openai".into(),
            model: "gpt-4o-mini".into(),
            ..WorkflowOptions::default()
        };
        let path = install_workflow(dir.path(), &opts, false).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("BLORA_PROVIDER"));
        assert!(text.contains("gpt-4o-mini"));
        assert!(install_workflow(dir.path(), &opts, false).is_err());
        assert!(install_workflow(dir.path(), &opts, true).is_ok());
        let message = install_instructions(&path);
        assert!(message.contains("BLORA_API_KEY"));
        assert!(message.contains("@blora"));
    }
}
