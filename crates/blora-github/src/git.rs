// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::path::PathBuf;
use std::process::Command;

use crate::error::{GithubError, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub branch: String,
    pub head: String,
    pub dirty: bool,
}

/// Git operations the orchestrator uses around the agent. The agent itself
/// also has a shell; these calls only prepare a branch and preserve leftover edits.
pub trait Repo {
    fn checkout_new(&self, branch: &str) -> Result<()>;
    fn checkout_local(&self, branch: &str, depth: u32, token: &str) -> Result<()>;
    fn checkout_fork(
        &self,
        fork_full_name: &str,
        remote_branch: &str,
        local_branch: &str,
        depth: u32,
        token: &str,
    ) -> Result<()>;
    fn snapshot(&self) -> Result<Snapshot>;
    fn commit_changes(&self, message: &str, coauthor: Option<&str>) -> Result<bool>;
    fn push(&self, full_name: &str, branch: &str, token: &str) -> Result<()>;
    fn diff_stat(&self) -> Result<String>;
}

#[derive(Clone, Debug)]
pub struct CommandRepo {
    cwd: PathBuf,
}

impl CommandRepo {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self { cwd: cwd.into() }
    }

    fn exec(&self, args: &[String]) -> Result<String> {
        let output = Command::new("git")
            .arg("-c")
            .arg("safe.directory=*")
            .args(args)
            .current_dir(&self.cwd)
            .output()
            .map_err(|err| GithubError::new(format!("无法执行 git：{err}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(GithubError::new(format!(
                "git {} 失败：{}",
                display_args(args),
                if stderr.is_empty() {
                    format!("退出码 {}", output.status.code().unwrap_or(1))
                } else {
                    stderr
                }
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn exec_authed(&self, token: &str, args: &[String]) -> Result<String> {
        if token.is_empty() {
            return self.exec(args);
        }
        let header = format!(
            "http.extraheader=AUTHORIZATION: basic {}",
            base64(format!("x-access-token:{token}").as_bytes())
        );
        let mut full = vec!["-c".to_string(), header];
        full.extend(args.iter().cloned());
        self.exec(&full)
    }
}

impl Repo for CommandRepo {
    fn checkout_new(&self, branch: &str) -> Result<()> {
        check_ref(branch)?;
        self.exec(&args(["checkout", "-b", branch]))?;
        Ok(())
    }

    fn checkout_local(&self, branch: &str, depth: u32, token: &str) -> Result<()> {
        check_ref(branch)?;
        let depth = depth.max(1);
        self.exec_authed(
            token,
            &args_owned(vec![
                "fetch".into(),
                "origin".into(),
                format!("--depth={depth}"),
                branch.to_string(),
            ]),
        )?;
        if self.exec(&args(["checkout", branch])).is_err() {
            self.exec(&args_owned(vec![
                "checkout".into(),
                "-B".into(),
                branch.to_string(),
                format!("origin/{branch}"),
            ]))?;
        }
        Ok(())
    }

    fn checkout_fork(
        &self,
        fork_full_name: &str,
        remote_branch: &str,
        local_branch: &str,
        depth: u32,
        token: &str,
    ) -> Result<()> {
        check_ref(remote_branch)?;
        check_ref(local_branch)?;
        if !fork_full_name.contains('/') || fork_full_name.contains(' ') {
            return Err(GithubError::new(format!(
                "无效的 fork 仓库：{fork_full_name}"
            )));
        }
        let url = format!("https://github.com/{fork_full_name}.git");
        let _ = self.exec(&args(["remote", "remove", "fork"]));
        self.exec(&args_owned(vec![
            "remote".into(),
            "add".into(),
            "fork".into(),
            url,
        ]))?;
        self.exec_authed(
            token,
            &args_owned(vec![
                "fetch".into(),
                "fork".into(),
                format!("--depth={}", depth.max(1)),
                remote_branch.to_string(),
            ]),
        )?;
        self.exec(&args_owned(vec![
            "checkout".into(),
            "-B".into(),
            local_branch.to_string(),
            format!("fork/{remote_branch}"),
        ]))?;
        Ok(())
    }

    fn snapshot(&self) -> Result<Snapshot> {
        let branch = self
            .exec(&args(["rev-parse", "--abbrev-ref", "HEAD"]))?
            .trim()
            .to_string();
        let head = self.exec(&args(["rev-parse", "HEAD"]))?.trim().to_string();
        let status = self.exec(&args(["status", "--porcelain"]))?;
        let dirty = status.lines().any(|line| {
            let path = porcelain_path(line);
            !path.is_empty() && !excluded_path(&path)
        });
        Ok(Snapshot {
            branch,
            head,
            dirty,
        })
    }

    fn commit_changes(&self, message: &str, coauthor: Option<&str>) -> Result<bool> {
        self.exec(&args(["add", "-A"]))?;
        let cached = self.exec(&args(["diff", "--cached", "--name-only"]))?;
        let excluded: Vec<String> = cached
            .lines()
            .map(str::trim)
            .filter(|path| !path.is_empty() && excluded_path(path))
            .map(ToOwned::to_owned)
            .collect();
        if !excluded.is_empty() {
            let mut reset = vec!["reset".to_string(), "-q".to_string(), "--".to_string()];
            reset.extend(excluded);
            self.exec(&reset)?;
        }
        let still = self
            .exec(&args(["diff", "--cached", "--name-only"]))?
            .trim()
            .to_string();
        if still.is_empty() {
            return Ok(false);
        }
        let mut body = message.trim().to_string();
        body.push('\n');
        if let Some(login) = coauthor.filter(|login| valid_login(login)) {
            body.push('\n');
            body.push_str(&format!(
                "Co-authored-by: {login} <{login}@users.noreply.github.com>\n"
            ));
        }
        let file = tempfile::NamedTempFile::new()
            .map_err(|err| GithubError::new(format!("无法写入提交说明：{err}")))?;
        std::fs::write(file.path(), body)
            .map_err(|err| GithubError::new(format!("无法写入提交说明：{err}")))?;
        self.exec(&args_owned(vec![
            "-c".into(),
            "user.name=Blora Agent".into(),
            "-c".into(),
            "user.email=blora-agent@users.noreply.github.com".into(),
            "commit".into(),
            "-F".into(),
            file.path().display().to_string(),
        ]))?;
        Ok(true)
    }

    fn push(&self, full_name: &str, branch: &str, token: &str) -> Result<()> {
        check_ref(branch)?;
        if !full_name.contains('/') || full_name.contains(' ') {
            return Err(GithubError::new(format!("无效的仓库名：{full_name}")));
        }
        let url = format!("https://github.com/{full_name}.git");
        self.exec_authed(
            token,
            &args_owned(vec!["push".into(), url, format!("HEAD:{branch}")]),
        )?;
        Ok(())
    }

    fn diff_stat(&self) -> Result<String> {
        self.exec(&args(["diff", "--stat"]))
    }
}

#[must_use]
pub fn excluded_path(path: &str) -> bool {
    let path = path.trim().trim_matches('"').replace('\\', "/");
    if path.split('/').any(|part| part == ".blora") {
        return true;
    }
    let name = path.rsplit('/').next().unwrap_or(path.as_str());
    if name == ".env" || name == ".env.local" {
        return true;
    }
    if let Some(rest) = name.strip_prefix(".env.") {
        if rest.ends_with(".local") && rest != "local" {
            return true;
        }
    }
    false
}

pub fn refuse_default_push(
    branch: &str,
    remote: &str,
    base_repo: &str,
    default_branch: &str,
) -> bool {
    !default_branch.is_empty() && branch == default_branch && remote == base_repo
}

fn porcelain_path(line: &str) -> String {
    let rest = line.get(3..).unwrap_or("").trim();
    let path = rest
        .rsplit_once(" -> ")
        .map(|(_, new_path)| new_path)
        .unwrap_or(rest);
    path.trim().trim_matches('"').to_string()
}

fn check_ref(name: &str) -> Result<()> {
    if name.is_empty()
        || name.starts_with('-')
        || name.contains("..")
        || name.contains(' ')
        || name.contains('\\')
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '.' | '-'))
    {
        return Err(GithubError::new(format!("无效的 git 引用：{name}")));
    }
    Ok(())
}

fn valid_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 39
        && login
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

fn args<const N: usize>(parts: [&str; N]) -> Vec<String> {
    parts.into_iter().map(str::to_string).collect()
}

fn args_owned(parts: Vec<String>) -> Vec<String> {
    parts
}

fn display_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.to_ascii_lowercase().contains("extraheader")
                || arg.to_ascii_lowercase().contains("authorization")
            {
                "http.extraheader=***".to_string()
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut index = 0;
    while index + 3 <= data.len() {
        let chunk =
            ((data[index] as u32) << 16) | ((data[index + 1] as u32) << 8) | data[index + 2] as u32;
        out.push(TABLE[((chunk >> 18) & 63) as usize] as char);
        out.push(TABLE[((chunk >> 12) & 63) as usize] as char);
        out.push(TABLE[((chunk >> 6) & 63) as usize] as char);
        out.push(TABLE[(chunk & 63) as usize] as char);
        index += 3;
    }
    let rest = data.len() - index;
    if rest == 1 {
        let chunk = (data[index] as u32) << 16;
        out.push(TABLE[((chunk >> 18) & 63) as usize] as char);
        out.push(TABLE[((chunk >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rest == 2 {
        let chunk = ((data[index] as u32) << 16) | ((data[index + 1] as u32) << 8);
        out.push(TABLE[((chunk >> 18) & 63) as usize] as char);
        out.push(TABLE[((chunk >> 12) & 63) as usize] as char);
        out.push(TABLE[((chunk >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_local_state_and_real_env_files() {
        assert!(excluded_path(".blora/tool-output/x.txt"));
        assert!(excluded_path(".env"));
        assert!(excluded_path("pkg/.env.local"));
        assert!(excluded_path(".env.production.local"));
        assert!(!excluded_path(".env.example"));
        assert!(!excluded_path("src/main.rs"));
    }

    #[test]
    fn hides_credentials_in_git_errors_and_encodes_basic_auth() {
        let shown = display_args(&[
            "-c".into(),
            "http.extraheader=AUTHORIZATION: basic c2VjcmV0".into(),
            "push".into(),
        ]);
        assert!(!shown.contains("c2VjcmV0"));
        assert!(shown.contains("push"));
        assert_eq!(base64(b"hello"), "aGVsbG8=");
        assert!(refuse_default_push("main", "acme/app", "acme/app", "main"));
        assert!(!refuse_default_push(
            "main",
            "alice/app",
            "acme/app",
            "main"
        ));
        assert!(!refuse_default_push(
            "blora/issue1",
            "acme/app",
            "acme/app",
            "main"
        ));
    }

    #[test]
    fn commit_skips_blora_dir_and_env_files() {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        run(&["init", "-b", "main"]);
        std::fs::write(dir.path().join("README.md"), "hi").unwrap();
        run(&["add", "README.md"]);
        run(&["commit", "-m", "init"]);

        let repo = CommandRepo::new(dir.path());
        repo.checkout_new("blora/test").unwrap();
        std::fs::create_dir_all(dir.path().join(".blora")).unwrap();
        std::fs::write(dir.path().join(".blora").join("note.txt"), "local").unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=1").unwrap();
        std::fs::write(dir.path().join(".env.local"), "SECRET=2").unwrap();
        std::fs::write(dir.path().join(".env.example"), "SECRET=\n").unwrap();
        std::fs::write(dir.path().join("src.txt"), "change").unwrap();

        let before = repo.snapshot().unwrap();
        assert!(before.dirty);
        assert!(repo.commit_changes("保存改动", Some("alice")).unwrap());
        let after = repo.snapshot().unwrap();
        assert!(!after.dirty);
        assert_ne!(after.head, before.head);

        let names = Command::new("git")
            .args(["show", "--name-only", "--pretty=format:", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&names.stdout);
        assert!(text.contains("src.txt"));
        assert!(text.contains(".env.example"));
        assert!(!text.contains(".env.local"));
        assert!(!text.lines().any(|line| line.trim() == ".env"));
        assert!(!text.contains(".blora"));
    }
}
