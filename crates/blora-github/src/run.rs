// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use crate::api::GithubApi;
use crate::attach;
use crate::context::{IssueContext, Note, PullContext, ReactionSite};
use crate::error::{GithubError, Result};
use crate::event::{self, ParseHint, ParsedEvent};
use crate::git::{self, Repo, Snapshot};
use crate::http::Http;
use crate::mention::{self, DEFAULT_MENTIONS};
use crate::prompt::{self, PromptParts, Trigger};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Skipped,
    DryRun,
    Completed,
    Failed,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub status: Status,
    pub detail: String,
    pub prompt: String,
    pub session_id: Option<String>,
    pub branch: Option<String>,
    pub pushed: bool,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub event: Value,
    pub event_name: Option<String>,
    pub repository: Option<String>,
    pub actor: Option<String>,
    pub run_id: String,
    pub token: Option<String>,
    pub mentions: Vec<String>,
    pub prompt_override: Option<String>,
    pub permission: String,
    pub provider: String,
    pub model: String,
    pub max_turns: u32,
    pub dry_run: bool,
    pub api_base: String,
    pub attachment_dir: PathBuf,
    pub session_export_path: Option<PathBuf>,
    pub workspace: PathBuf,
    pub now: DateTime<Utc>,
}

impl Config {
    #[must_use]
    pub fn new(event: Value) -> Self {
        Self {
            event,
            event_name: None,
            repository: None,
            actor: None,
            run_id: "100".into(),
            token: Some("token-for-tests".into()),
            mentions: mention::parse_mention_list(DEFAULT_MENTIONS),
            prompt_override: None,
            permission: "yolo".into(),
            provider: String::new(),
            model: String::new(),
            max_turns: 24,
            dry_run: false,
            api_base: "https://api.github.com".into(),
            attachment_dir: std::env::temp_dir().join("blora-attachments"),
            session_export_path: None,
            workspace: PathBuf::from("."),
            now: Utc.with_ymd_and_hms(2026, 9, 26, 3, 4, 5).unwrap(),
        }
    }
}

pub struct AgentRequest<'a> {
    pub workspace: &'a Path,
    pub title: &'a str,
    pub prompt: &'a str,
    pub permission: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub max_turns: u32,
    pub read_only: bool,
}

#[derive(Clone, Debug)]
pub struct AgentResponse {
    pub session_id: String,
    pub text: String,
}

pub trait AgentDriver {
    fn run(&self, request: &AgentRequest<'_>) -> Result<AgentResponse>;

    fn export(&self, _session_id: &str, _path: &Path) -> Result<()> {
        Ok(())
    }
}

struct Loaded {
    issue: Option<IssueContext>,
    pull: Option<PullContext>,
    default_branch: String,
}

struct Prepared {
    branch: String,
    remote: String,
}

struct Eyes<'a, H: Http> {
    api: &'a GithubApi<'a, H>,
    owner: String,
    repo: String,
    site: ReactionSite,
    id: u64,
}

impl<H: Http> Drop for Eyes<'_, H> {
    fn drop(&mut self) {
        let _ = self
            .api
            .delete_reaction(&self.owner, &self.repo, &self.site, self.id);
    }
}

pub fn execute<H: Http, R: Repo, A: AgentDriver>(
    config: &Config,
    http: &H,
    repo: &R,
    agent: &A,
) -> Result<Report> {
    let event = event::parse(
        &config.event,
        &ParseHint {
            event_name: config.event_name.clone(),
            repository: config.repository.clone(),
            actor: config.actor.clone(),
        },
    )?;
    if event.is_comment() && mention::find_mention(&event.comment_body, &config.mentions).is_none()
    {
        return Ok(report(
            Status::Skipped,
            "评论里没有触发词，已跳过。",
            "",
            None,
            None,
            false,
        ));
    }
    let trigger =
        prompt::trigger_text(&event, &config.mentions, config.prompt_override.as_deref())?;
    if config.dry_run {
        return dry_run(config, http, &event, &trigger);
    }
    let token = token_of(config)?;
    let api = GithubApi::new(http, token, &config.api_base);
    if let Some(message) = ensure_permission(&api, &event)? {
        publish(&api, &event, token, &message);
        return Ok(failed(&message));
    }
    let _eyes = watch_reaction(&api, &event);
    match work(config, http, repo, agent, &api, &event, &trigger, token) {
        Ok(report) => Ok(report),
        Err(err) => {
            let message = prompt::redact(&err.message, token);
            publish(&api, &event, token, &message);
            Ok(failed(&message))
        }
    }
}

fn dry_run<H: Http>(
    config: &Config,
    http: &H,
    event: &ParsedEvent,
    trigger: &Trigger,
) -> Result<Report> {
    let loaded = if let Some(token) = config.token.as_deref().filter(|token| !token.is_empty()) {
        let api = GithubApi::new(http, token, &config.api_base);
        load_context(&api, event)?
    } else {
        Loaded {
            issue: payload_issue(event),
            pull: payload_pull(event),
            default_branch: String::new(),
        }
    };
    let prompt = build_prompt(config, http, event, trigger, &loaded)?;
    Ok(report(Status::DryRun, &prompt, &prompt, None, None, false))
}

fn work<H: Http, R: Repo, A: AgentDriver>(
    config: &Config,
    http: &H,
    repo: &R,
    agent: &A,
    api: &GithubApi<'_, H>,
    event: &ParsedEvent,
    trigger: &Trigger,
    token: &str,
) -> Result<Report> {
    let loaded = load_context(api, event)?;
    let prepared = prepare_branch(repo, event, &loaded, token, config)?;
    let baseline = repo.snapshot()?;
    let prompt = build_prompt(config, http, event, trigger, &loaded)?;
    let title = session_title(event);
    let response = agent.run(&AgentRequest {
        workspace: &config.workspace,
        title: &title,
        prompt: &prompt,
        permission: &config.permission,
        provider: &config.provider,
        model: &config.model,
        max_turns: config.max_turns.max(1),
        read_only: false,
    })?;
    if let Some(path) = &config.session_export_path {
        if let Err(err) = agent.export(&response.session_id, path) {
            eprintln!("导出会话失败：{}", prompt::redact(&err.message, token));
        }
    }
    let reply = if response.text.trim().is_empty() {
        "Blora Agent 没有产生文字回复。".to_string()
    } else {
        response.text
    };
    let after = repo.snapshot()?;
    let (note, pushed) = preserve_edits(
        repo, agent, config, &baseline, &after, &prepared, &loaded, event, &reply,
    )?;
    let comment =
        prompt::assemble_comment(&reply, &note, &event.owner, &event.repo, &config.run_id);
    let comment = prompt::redact(&comment, token);
    publish(api, event, token, &comment);
    if event.is_repo_automation() {
        println!("{comment}");
    }
    Ok(report(
        Status::Completed,
        &comment,
        &prompt,
        Some(response.session_id),
        Some(prepared.branch),
        pushed,
    ))
}

fn preserve_edits<R: Repo, A: AgentDriver>(
    repo: &R,
    agent: &A,
    config: &Config,
    baseline: &Snapshot,
    after: &Snapshot,
    prepared: &Prepared,
    loaded: &Loaded,
    event: &ParsedEvent,
    reply: &str,
) -> Result<(String, bool)> {
    if after.branch != prepared.branch || after.branch == "HEAD" {
        eprintln!(
            "当前分支是 {}，与准备好的 {} 不同，不代为推送。",
            after.branch, prepared.branch
        );
        return Ok((String::new(), false));
    }
    let title = loaded
        .issue
        .as_ref()
        .map(|issue| issue.title.clone())
        .or_else(|| loaded.pull.as_ref().map(|pull| pull.title.clone()))
        .unwrap_or_default();
    if after.dirty {
        let subject = summarize(agent, config, reply, event.number, &title);
        let coauthor = event.actor.as_deref().filter(|_| !event.is_schedule());
        if !repo.commit_changes(&subject, coauthor)? {
            return Ok((String::new(), false));
        }
    } else if after.head == baseline.head {
        return Ok((String::new(), false));
    }
    let branch = repo.snapshot()?.branch;
    if git::refuse_default_push(
        &branch,
        &prepared.remote,
        &event.full_name(),
        &loaded.default_branch,
    ) {
        return Ok((format!("当前分支 {branch} 是默认分支，没有推送。"), false));
    }
    if let Err(err) = repo.push(&prepared.remote, &branch, token_of(config)?) {
        let stat = repo.diff_stat().unwrap_or_default();
        return Err(GithubError::new(format!("改动没有推送：{err}\n\n{stat}")));
    }
    Ok((format!("改动已推送到 `{branch}`。"), true))
}

fn summarize<A: AgentDriver>(
    agent: &A,
    config: &Config,
    reply: &str,
    number: Option<u64>,
    title: &str,
) -> String {
    let clipped: String = reply.chars().take(4_000).collect();
    let prompt = format!("用不超过 60 个字符概括下面的工作，只输出这一句：\n\n{clipped}");
    let summary = agent
        .run(&AgentRequest {
            workspace: &config.workspace,
            title: "GitHub 提交说明",
            prompt: &prompt,
            permission: "plan",
            provider: &config.provider,
            model: &config.model,
            max_turns: 1,
            read_only: true,
        })
        .ok()
        .map(|response| prompt::one_line(&response.text, 72))
        .filter(|line| line != "blora");
    summary.unwrap_or_else(|| prompt::fallback_subject(number, title))
}

fn prepare_branch<R: Repo>(
    repo: &R,
    event: &ParsedEvent,
    loaded: &Loaded,
    token: &str,
    config: &Config,
) -> Result<Prepared> {
    if let Some(pull) = &loaded.pull {
        if pull.head_ref.is_empty() {
            return Err(GithubError::new("Pull Request 没有 head 分支。"));
        }
        let base_repo = if pull.base_repo.is_empty() {
            event.full_name()
        } else {
            pull.base_repo.clone()
        };
        let head_repo = if pull.head_repo.is_empty() {
            base_repo.clone()
        } else {
            pull.head_repo.clone()
        };
        let depth = u32::try_from(pull.commits.max(20)).unwrap_or(20);
        if head_repo == base_repo {
            repo.checkout_local(&pull.head_ref, depth, token)?;
            return Ok(Prepared {
                branch: pull.head_ref.clone(),
                remote: base_repo,
            });
        }
        let local = branch_name("pr", event.number, &config.run_id, config.now);
        repo.checkout_fork(&head_repo, &pull.head_ref, &local, depth, token)?;
        return Ok(Prepared {
            branch: local,
            remote: head_repo,
        });
    }
    let kind = if event.name == "schedule" {
        "schedule"
    } else if event.name == "workflow_dispatch" {
        "dispatch"
    } else {
        "issue"
    };
    let branch = branch_name(kind, event.number, &config.run_id, config.now);
    repo.checkout_new(&branch)?;
    Ok(Prepared {
        branch,
        remote: event.full_name(),
    })
}

fn build_prompt<H: Http>(
    config: &Config,
    http: &H,
    event: &ParsedEvent,
    trigger: &Trigger,
    loaded: &Loaded,
) -> Result<String> {
    let rendered = prompt::render(&PromptParts {
        trigger: &trigger.text,
        review: event.review.as_ref(),
        issue: loaded.issue.as_ref(),
        pull: loaded.pull.as_ref(),
        attachments: &[],
        default_branch: &loaded.default_branch,
    });
    let token = config.token.as_deref().unwrap_or("");
    let (rewritten, _) = attach::rewrite(&rendered, http, token, &config.attachment_dir)?;
    Ok(rewritten)
}

fn load_context<H: Http>(api: &GithubApi<'_, H>, event: &ParsedEvent) -> Result<Loaded> {
    let default_branch = api.default_branch(&event.owner, &event.repo)?;
    if event.is_pull_request {
        let number = event
            .number
            .ok_or_else(|| GithubError::new("Pull Request 事件缺少编号。"))?;
        let mut pull = api.load_pull(&event.owner, &event.repo, number)?;
        drop_trigger(&mut pull.comments, event.comment_id);
        pull.review_comments
            .retain(|note| event.comment_id != Some(note.id));
        return Ok(Loaded {
            issue: None,
            pull: Some(pull),
            default_branch,
        });
    }
    if let Some(number) = event.number {
        let mut issue = api.load_issue(&event.owner, &event.repo, number)?;
        drop_trigger(&mut issue.comments, event.comment_id);
        return Ok(Loaded {
            issue: Some(issue),
            pull: None,
            default_branch,
        });
    }
    Ok(Loaded {
        issue: None,
        pull: None,
        default_branch,
    })
}

fn payload_issue(event: &ParsedEvent) -> Option<IssueContext> {
    if event.is_pull_request {
        return None;
    }
    let number = event.number?;
    if event.issue_title.is_empty() && event.issue_body.is_empty() {
        return None;
    }
    Some(IssueContext {
        number,
        title: event.issue_title.clone(),
        body: event.issue_body.clone(),
        author: event.actor.clone().unwrap_or_default(),
        state: String::new(),
        created_at: String::new(),
        comments: Vec::new(),
    })
}

fn payload_pull(event: &ParsedEvent) -> Option<PullContext> {
    if !event.is_pull_request {
        return None;
    }
    Some(PullContext {
        number: event.number.unwrap_or(0),
        title: event.issue_title.clone(),
        body: event.issue_body.clone(),
        author: event.actor.clone().unwrap_or_default(),
        state: String::new(),
        created_at: String::new(),
        base_ref: String::new(),
        head_ref: String::new(),
        base_repo: event.full_name(),
        head_repo: event.full_name(),
        commits: 1,
        files: Vec::new(),
        comments: Vec::new(),
        review_comments: Vec::new(),
        reviews: Vec::new(),
    })
}

fn ensure_permission<H: Http>(
    api: &GithubApi<'_, H>,
    event: &ParsedEvent,
) -> Result<Option<String>> {
    if event.is_schedule() {
        return Ok(None);
    }
    let Some(actor) = event.actor.as_deref() else {
        return Ok(Some("这个事件没有触发者，无法确认写权限。".to_string()));
    };
    let level = api.permission(&event.owner, &event.repo, actor)?;
    if level == "admin" || level == "write" {
        return Ok(None);
    }
    Ok(Some(format!("{actor} 对仓库没有写权限，已拒绝执行。")))
}

fn watch_reaction<'a, H: Http>(
    api: &'a GithubApi<'a, H>,
    event: &ParsedEvent,
) -> Option<Eyes<'a, H>> {
    if event.is_repo_automation() && event.number.is_none() {
        return None;
    }
    let site = event.reaction_site()?;
    let id = api.add_eyes(&event.owner, &event.repo, &site).ok()?;
    Some(Eyes {
        api,
        owner: event.owner.clone(),
        repo: event.repo.clone(),
        site,
        id,
    })
}

fn publish<H: Http>(api: &GithubApi<'_, H>, event: &ParsedEvent, token: &str, body: &str) {
    let Some(number) = event.number else {
        return;
    };
    let body = prompt::redact(body, token);
    if let Err(err) = api.comment(&event.owner, &event.repo, number, &body) {
        eprintln!(
            "评论没有发到 GitHub：{}",
            prompt::redact(&err.message, token)
        );
    }
}

fn drop_trigger(notes: &mut Vec<Note>, id: Option<u64>) {
    if let Some(id) = id {
        notes.retain(|note| note.id != id);
    }
}

fn session_title(event: &ParsedEvent) -> String {
    match event.number {
        Some(number) => format!("GitHub #{number}"),
        None if event.name == "schedule" => "GitHub schedule".into(),
        None if event.name == "workflow_dispatch" => "GitHub dispatch".into(),
        None => "GitHub".into(),
    }
}

fn branch_name(kind: &str, number: Option<u64>, run_id: &str, now: DateTime<Utc>) -> String {
    let stamp = now.format("%Y%m%d%H%M%S");
    let nonce = run_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(6)
        .collect::<String>();
    let nonce = if nonce.is_empty() {
        "run".to_string()
    } else {
        nonce
    };
    match kind {
        "pr" => format!("blora/pr{}-{stamp}", number.unwrap_or(0)),
        "schedule" => format!("blora/schedule-{nonce}-{stamp}"),
        "dispatch" => format!("blora/dispatch-{nonce}-{stamp}"),
        _ => format!("blora/issue{}-{stamp}", number.unwrap_or(0)),
    }
}

fn token_of(config: &Config) -> Result<&str> {
    config
        .token
        .as_deref()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            GithubError::new("缺少 GitHub token。设置 GITHUB_TOKEN 或 BLORA_GITHUB_TOKEN。")
        })
}

fn report(
    status: Status,
    detail: &str,
    prompt: &str,
    session_id: Option<String>,
    branch: Option<String>,
    pushed: bool,
) -> Report {
    Report {
        status,
        detail: detail.to_string(),
        prompt: prompt.to_string(),
        session_id,
        branch,
        pushed,
    }
}

fn failed(detail: &str) -> Report {
    report(Status::Failed, detail, "", None, None, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpRequest, HttpResponse};
    use std::cell::RefCell;
    use std::rc::Rc;

    struct World {
        branch: String,
        head: String,
        dirty: bool,
        checkouts: Vec<String>,
        commits: Vec<String>,
        pushes: Vec<String>,
        fail_push: bool,
    }

    struct FakeRepo {
        world: Rc<RefCell<World>>,
    }

    impl Repo for FakeRepo {
        fn checkout_new(&self, branch: &str) -> Result<()> {
            let mut world = self.world.borrow_mut();
            world.branch = branch.to_string();
            world.checkouts.push(format!("new:{branch}"));
            Ok(())
        }

        fn checkout_local(&self, branch: &str, _depth: u32, _token: &str) -> Result<()> {
            let mut world = self.world.borrow_mut();
            world.branch = branch.to_string();
            world.checkouts.push(format!("local:{branch}"));
            Ok(())
        }

        fn checkout_fork(
            &self,
            fork_full_name: &str,
            remote_branch: &str,
            local_branch: &str,
            _depth: u32,
            _token: &str,
        ) -> Result<()> {
            let mut world = self.world.borrow_mut();
            world.branch = local_branch.to_string();
            world.checkouts.push(format!(
                "fork:{fork_full_name}:{remote_branch}:{local_branch}"
            ));
            Ok(())
        }

        fn snapshot(&self) -> Result<Snapshot> {
            let world = self.world.borrow();
            Ok(Snapshot {
                branch: world.branch.clone(),
                head: world.head.clone(),
                dirty: world.dirty,
            })
        }

        fn commit_changes(&self, message: &str, _coauthor: Option<&str>) -> Result<bool> {
            let mut world = self.world.borrow_mut();
            if !world.dirty {
                return Ok(false);
            }
            world.dirty = false;
            world.head = "committed".into();
            world.commits.push(message.to_string());
            Ok(true)
        }

        fn push(&self, full_name: &str, branch: &str, _token: &str) -> Result<()> {
            let mut world = self.world.borrow_mut();
            if world.fail_push {
                return Err(GithubError::new("推送被拒绝"));
            }
            world.pushes.push(format!("{full_name}:{branch}"));
            Ok(())
        }

        fn diff_stat(&self) -> Result<String> {
            Ok("src.txt | 1 +".into())
        }
    }

    struct FakeHttp {
        calls: RefCell<Vec<(String, String)>>,
        permission: String,
        pull_head: String,
        pull_base: String,
    }

    impl Http for FakeHttp {
        fn send(&self, request: &HttpRequest<'_>) -> Result<HttpResponse> {
            self.calls
                .borrow_mut()
                .push((request.method.to_string(), request.url.to_string()));
            let url = request.url.split('?').next().unwrap_or(request.url);
            let body: Vec<u8> = if url.ends_with("/collaborators/alice/permission")
                || url.contains("/collaborators/")
            {
                format!(r#"{{"permission":"{}"}}"#, self.permission).into_bytes()
            } else if url.ends_with("/repos/acme/app") {
                br#"{"default_branch":"main"}"#.to_vec()
            } else if url.contains("/issues/4/comments") && request.method == "GET" {
                r#"[{"id":11,"body":"/blora 只解释","user":{"login":"alice"},"created_at":"2026-09-26"}]"#
                    .as_bytes()
                    .to_vec()
            } else if url.ends_with("/issues/4") && request.method == "GET" {
                r#"{"title":"登录失败","body":"无法登录","state":"open","user":{"login":"bob"},"created_at":"2026-09-26"}"#
                    .as_bytes()
                    .to_vec()
            } else if url.contains("/pulls/8/files") {
                br#"[{"filename":"src/main.rs","status":"modified","additions":2,"deletions":1}]"#
                    .to_vec()
            } else if url.contains("/pulls/8/comments")
                || url.contains("/pulls/8/reviews")
                || url.contains("/issues/8/comments") && request.method == "GET"
            {
                b"[]".to_vec()
            } else if url.ends_with("/pulls/8") {
                format!(
                    r#"{{"title":"按钮","body":"改按钮","state":"open","user":{{"login":"alice"}},"created_at":"2026-09-26","commits":2,"base":{{"ref":"main","repo":{{"full_name":"{}"}}}},"head":{{"ref":"feature","repo":{{"full_name":"{}"}}}}}}"#,
                    self.pull_base, self.pull_head
                )
                .into_bytes()
            } else if request.method == "POST" && url.contains("/reactions") {
                br#"{"id":9}"#.to_vec()
            } else if request.method == "POST" && url.contains("/comments") {
                br#"{"id":20}"#.to_vec()
            } else if request.method == "DELETE" {
                b"{}".to_vec()
            } else if url.contains("user-attachments") {
                b"image-bytes".to_vec()
            } else {
                b"{}".to_vec()
            };
            Ok(HttpResponse { status: 200, body })
        }
    }

    struct FakeAgent {
        world: Rc<RefCell<World>>,
        text: String,
        dirty: bool,
        switch_to: Option<String>,
        prompts: Rc<RefCell<Vec<String>>>,
    }

    impl AgentDriver for FakeAgent {
        fn run(&self, request: &AgentRequest<'_>) -> Result<AgentResponse> {
            self.prompts.borrow_mut().push(request.prompt.to_string());
            if self.prompts.borrow().len() == 1 {
                let mut world = self.world.borrow_mut();
                if self.dirty {
                    world.dirty = true;
                }
                if let Some(branch) = &self.switch_to {
                    world.branch = branch.clone();
                }
            }
            Ok(AgentResponse {
                session_id: "ses_test".into(),
                text: if request.read_only {
                    "修好登录".into()
                } else {
                    self.text.clone()
                },
            })
        }
    }

    fn world() -> Rc<RefCell<World>> {
        Rc::new(RefCell::new(World {
            branch: "main".into(),
            head: "abc".into(),
            dirty: false,
            checkouts: Vec::new(),
            commits: Vec::new(),
            pushes: Vec::new(),
            fail_push: false,
        }))
    }

    fn issue_event(body: &str) -> Value {
        serde_json::json!({
            "eventName": "issue_comment",
            "actor": "alice",
            "repo": {"owner": "acme", "repo": "app"},
            "payload": {
                "issue": {"number": 4, "title": "登录失败", "body": "无法登录"},
                "comment": {"id": 11, "body": body}
            }
        })
    }

    struct CaseResult {
        report: Report,
        world: Rc<RefCell<World>>,
        prompts: Rc<RefCell<Vec<String>>>,
        http: FakeHttp,
    }

    fn run_case(
        event: Value,
        agent_text: &str,
        dirty: bool,
        switch_to: Option<&str>,
        http: FakeHttp,
    ) -> CaseResult {
        let world = world();
        let prompts = Rc::new(RefCell::new(Vec::new()));
        let repo = FakeRepo {
            world: Rc::clone(&world),
        };
        let agent = FakeAgent {
            world: Rc::clone(&world),
            text: agent_text.into(),
            dirty,
            switch_to: switch_to.map(str::to_owned),
            prompts: Rc::clone(&prompts),
        };
        let mut config = Config::new(event);
        config.attachment_dir =
            std::env::temp_dir().join(format!("blora-attachments-test-{}", std::process::id()));
        let report = execute(&config, &http, &repo, &agent).unwrap();
        CaseResult {
            report,
            world,
            prompts,
            http,
        }
    }

    fn http() -> FakeHttp {
        FakeHttp {
            calls: RefCell::new(Vec::new()),
            permission: "write".into(),
            pull_head: "acme/app".into(),
            pull_base: "acme/app".into(),
        }
    }

    #[test]
    fn explain_does_not_push_or_open_a_pull_request() {
        let CaseResult {
            report,
            world,
            prompts,
            http,
        } = run_case(
            issue_event("/blora 只解释原因"),
            "这是总结",
            false,
            None,
            http(),
        );
        assert_eq!(report.status, Status::Completed);
        assert!(!report.pushed);
        assert!(world.borrow().commits.is_empty());
        assert!(world.borrow().pushes.is_empty());
        assert!(report.detail.contains("这是总结"));
        assert!(!report.detail.contains("Pull Request #"));
        let prompt = &prompts.borrow()[0];
        assert!(prompt.contains("登录失败"));
        assert!(prompt.contains("只解释原因"));
        assert!(prompt.contains("工作流不会自动打开 Pull Request"));
        assert!(!prompt.contains("alice 于"));
        assert!(
            http.calls
                .borrow()
                .iter()
                .all(|(_, url)| !url.contains("/pulls"))
        );
        assert!(world.borrow().checkouts[0].starts_with("new:blora/issue4-"));
    }

    #[test]
    fn edits_are_pushed_without_opening_a_pull_request() {
        let CaseResult {
            report,
            world,
            prompts,
            http,
        } = run_case(
            issue_event("@blora 请修好登录"),
            "已经修好",
            true,
            None,
            http(),
        );
        assert_eq!(report.status, Status::Completed);
        assert!(report.pushed);
        assert_eq!(world.borrow().commits.len(), 1);
        assert!(world.borrow().pushes[0].contains("acme/app:blora/issue4-"));
        assert!(report.detail.contains("已推送到"));
        assert_eq!(prompts.borrow().len(), 2);
        assert!(
            http.calls
                .borrow()
                .iter()
                .all(|(method, url)| !(method == "POST" && url.contains("/pulls")))
        );
    }

    #[test]
    fn switched_branch_is_left_alone() {
        let CaseResult { report, world, .. } = run_case(
            issue_event("/blora 修好"),
            "自己开了分支",
            true,
            Some("feature/mine"),
            http(),
        );
        assert!(!report.pushed);
        assert!(world.borrow().pushes.is_empty());
        assert!(world.borrow().commits.is_empty());
    }

    #[test]
    fn read_permission_does_not_run_the_agent() {
        let mut api = http();
        api.permission = "read".into();
        let CaseResult {
            report, prompts, ..
        } = run_case(issue_event("/blora 修好"), "不会运行", true, None, api);
        assert_eq!(report.status, Status::Failed);
        assert!(report.detail.contains("没有写权限"));
        assert!(prompts.borrow().is_empty());
    }

    #[test]
    fn partial_mention_skips_without_calling_github() {
        let http = http();
        let CaseResult {
            report,
            prompts,
            http,
            ..
        } = run_case(issue_event("see /blora-docs"), "no", false, None, http);
        assert_eq!(report.status, Status::Skipped);
        assert!(prompts.borrow().is_empty());
        assert!(http.calls.borrow().is_empty());
    }

    #[test]
    fn at_mention_inside_a_sentence_runs() {
        let CaseResult {
            report, prompts, ..
        } = run_case(
            issue_event("你好 @blora 看一下这个报错"),
            "看过了",
            false,
            None,
            http(),
        );
        assert_eq!(report.status, Status::Completed);
        assert!(prompts.borrow()[0].contains("看一下这个报错"));
        let _ = report;
    }

    #[test]
    fn schedule_without_prompt_fails_and_with_prompt_pushes() {
        let event = serde_json::json!({
            "eventName": "schedule",
            "repo": "acme/app",
            "payload": {"schedule": "0 0 * * *"}
        });
        let err = execute(
            &Config::new(event.clone()),
            &http(),
            &FakeRepo { world: world() },
            &FakeAgent {
                world: world(),
                text: "x".into(),
                dirty: false,
                switch_to: None,
                prompts: Rc::new(RefCell::new(Vec::new())),
            },
        );
        assert!(err.unwrap_err().message.contains("prompt"));

        let mut config = Config::new(event);
        config.prompt_override = Some("检查依赖更新".into());
        let world = world();
        let prompts = Rc::new(RefCell::new(Vec::new()));
        let agent = FakeAgent {
            world: Rc::clone(&world),
            text: "没有需要更新的".into(),
            dirty: true,
            switch_to: None,
            prompts,
        };
        let report = execute(
            &config,
            &http(),
            &FakeRepo {
                world: Rc::clone(&world),
            },
            &agent,
        )
        .unwrap();
        assert_eq!(report.status, Status::Completed);
        assert!(report.pushed);
        assert!(world.borrow().checkouts[0].contains("schedule"));
    }

    #[test]
    fn local_pull_request_checks_out_head_and_fork_checks_out_fork() {
        let local = serde_json::json!({
            "eventName": "issue_comment",
            "actor": "alice",
            "repo": "acme/app",
            "payload": {
                "issue": {"number": 8, "pull_request": {"url": "https://example.test"}},
                "comment": {"id": 3, "body": "/blora 总结这个 PR"}
            }
        });
        let CaseResult {
            report,
            world,
            prompts,
            ..
        } = run_case(local, "总结", false, None, http());
        assert_eq!(report.status, Status::Completed);
        assert_eq!(world.borrow().checkouts, vec!["local:feature".to_string()]);
        assert!(prompts.borrow()[0].contains("src/main.rs"));

        let mut fork_http = http();
        fork_http.pull_head = "alice/app".into();
        let fork_event = serde_json::json!({
            "eventName": "pull_request_review_comment",
            "actor": "alice",
            "repo": "acme/app",
            "payload": {
                "pull_request": {"number": 8},
                "comment": {
                    "id": 5,
                    "body": "@blora",
                    "path": "src/main.rs",
                    "line": 12,
                    "diff_hunk": "@@ -1 +1 @@\n-old\n+new"
                }
            }
        });
        let CaseResult {
            report,
            world,
            prompts,
            ..
        } = run_case(fork_event, "审查意见", false, None, fork_http);
        assert_eq!(report.status, Status::Completed);
        assert!(world.borrow().checkouts[0].starts_with("fork:alice/app:feature:"));
        assert!(prompts.borrow()[0].contains("src/main.rs"));
        assert!(prompts.borrow()[0].contains("不要修改文件"));
    }

    #[test]
    fn dry_run_prints_prompt_without_the_agent() {
        let mut config = Config::new(issue_event("/ba 解释一下"));
        config.dry_run = true;
        config.token = None;
        let prompts = Rc::new(RefCell::new(Vec::new()));
        let report = execute(
            &config,
            &http(),
            &FakeRepo { world: world() },
            &FakeAgent {
                world: world(),
                text: "no".into(),
                dirty: false,
                switch_to: None,
                prompts: Rc::clone(&prompts),
            },
        )
        .unwrap();
        assert_eq!(report.status, Status::DryRun);
        assert!(report.prompt.contains("解释一下"));
        assert!(report.prompt.contains("登录失败"));
        assert!(prompts.borrow().is_empty());
    }

    #[test]
    fn push_failure_is_reported() {
        let world = world();
        world.borrow_mut().fail_push = true;
        let prompts = Rc::new(RefCell::new(Vec::new()));
        let agent = FakeAgent {
            world: Rc::clone(&world),
            text: "改了".into(),
            dirty: true,
            switch_to: None,
            prompts,
        };
        let report = execute(
            &Config::new(issue_event("/blora 修好")),
            &http(),
            &FakeRepo { world },
            &agent,
        )
        .unwrap();
        assert_eq!(report.status, Status::Failed);
        assert!(report.detail.contains("推送被拒绝"));
    }
}
