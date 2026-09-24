// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Blora Agent command-line interface.

mod acp;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use blora_runtime::{CancelToken, PermissionMode, RunOptions, Runtime};
use blora_session::TranscriptItem;
use blora_storage::{CreateSession, CreateTask, SqliteStore};
use blora_types::{Mode, SessionId, TaskId};
use chrono::{Duration, Utc};
use clap::{Parser, Subcommand};

const LICENSE_NOTICE: &str = "\
Blora Agent  Copyright (C) 2026  Blora Agent contributors
This program comes with ABSOLUTELY NO WARRANTY.
This is free software, and you are welcome to redistribute it
under the terms of the GNU General Public License version 3 or later.
See LICENSE or https://www.gnu.org/licenses/ for details.";

#[derive(Parser)]
#[command(
    name = "blora",
    version,
    about = "Blora Agent. Run `blora` for the TUI, `blora --web` or `blora web` for the browser UI.",
    after_help = LICENSE_NOTICE,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// Override Blora home directory (default: $BLORA_HOME or ~/.blora).
    #[arg(long, global = true, env = "BLORA_HOME")]
    home: Option<PathBuf>,
    /// Workspace for TUI or Web (default: current directory).
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    /// Open the local Web UI instead of the TUI.
    #[arg(long)]
    web: bool,
    /// Bind address used with `--web` (default: 127.0.0.1:8787).
    #[arg(long, default_value = "127.0.0.1:8787")]
    bind: String,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Print license information.
    License,
    /// Session commands.
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },
    /// Run an agent turn against a session.
    Run {
        /// Session identifier (`ses_…`).
        #[arg(long)]
        session: String,
        /// User prompt.
        prompt: String,
        /// Force the mock provider.
        #[arg(long)]
        mock: bool,
        /// Auto-approve write_file and shell (same as `--permission yolo`).
        #[arg(long)]
        yes: bool,
        /// Permission mode: plan, ask, auto-edit, or yolo. Overrides --yes.
        #[arg(long, value_parser = parse_permission)]
        permission: Option<PermissionMode>,
        /// Model name override.
        #[arg(long)]
        model: Option<String>,
        /// Provider: openai, responses, anthropic, or mock.
        #[arg(long)]
        provider: Option<String>,
        /// Run tools in a detached git worktree.
        #[arg(long)]
        worktree: bool,
    },
    /// Agent Client Protocol over stdin/stdout.
    Acp,
    /// Open the local Web UI.
    Web {
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Alias for the default TUI entry. Prefer `blora`.
    #[command(hide = true)]
    Tui {
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Alias for `blora web`.
    #[command(hide = true)]
    Serve {
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Durable background tasks.
    Task {
        #[command(subcommand)]
        command: TaskCommands,
    },
    /// Copy the SQLite state file (and WAL) to a backup path.
    Backup { path: Option<PathBuf> },
    /// Restore state from a backup SQLite file.
    Restore { path: PathBuf },
    /// Show recorded token usage.
    Usage {
        #[arg(long)]
        session: Option<String>,
    },
    /// Multi-user gateway (token auth required for /api).
    Gateway {
        #[arg(long, default_value = "0.0.0.0:8787")]
        bind: String,
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// Gateway users.
    User {
        #[command(subcommand)]
        command: UserCommands,
    },
    /// Plugin marketplace.
    Plugin {
        #[command(subcommand)]
        command: PluginCommands,
    },
    /// Distill long-term memories from a session.
    Memory {
        #[command(subcommand)]
        command: MemoryCommands,
    },
}

#[derive(Debug, Subcommand)]
enum UserCommands {
    Add { name: String },
    List,
}

#[derive(Debug, Subcommand)]
enum PluginCommands {
    List,
    Install { name: String },
    Remove { name: String },
}

#[derive(Debug, Subcommand)]
enum MemoryCommands {
    Distill { session: String },
}

#[derive(Debug, Subcommand)]
enum SessionCommands {
    Create {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value = "code")]
        mode: String,
    },
    List,
    Show {
        id: String,
    },
    Replay {
        id: String,
    },
    Compact {
        id: String,
    },
    Fork {
        id: String,
    },
    Export {
        id: String,
    },
    Archive {
        id: String,
    },
    Resume {
        id: String,
    },
    Search {
        query: String,
    },
}

#[derive(Debug, Subcommand)]
enum TaskCommands {
    Create {
        #[arg(long)]
        session: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        prompt: String,
        /// Delay such as 10s, 5m, or 1h.
        #[arg(long)]
        r#in: Option<String>,
        /// 5-field cron expression, for example `0 * * * *`.
        #[arg(long)]
        cron: Option<String>,
        #[arg(long)]
        mock: bool,
        #[arg(long)]
        yes: bool,
    },
    List {
        #[arg(long)]
        session: Option<String>,
    },
    Show {
        id: String,
    },
    Cancel {
        id: String,
    },
    Pause {
        id: String,
    },
    Resume {
        id: String,
    },
    /// Run due tasks once.
    Pump,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match cli.command {
        None if cli.web => serve_web(cli.home.as_deref(), &cli.bind, cli.workspace, false),
        None => {
            let runtime = Runtime::new(open_store(cli.home.as_deref())?);
            let workspace = cli.workspace.unwrap_or(std::env::current_dir()?);
            blora_tui::run(&runtime, &workspace)?;
            Ok(())
        }
        Some(Commands::License) => {
            println!("{LICENSE_NOTICE}");
            Ok(())
        }
        Some(Commands::Web { bind, workspace } | Commands::Serve { bind, workspace }) => {
            let workspace = workspace.or(cli.workspace);
            serve_web(cli.home.as_deref(), &bind, workspace, false)
        }
        Some(Commands::Gateway { bind, workspace }) => {
            let workspace = workspace.or(cli.workspace);
            serve_web(cli.home.as_deref(), &bind, workspace, true)
        }
        Some(Commands::Acp) => {
            let runtime = Runtime::new(open_store(cli.home.as_deref())?);
            let workspace = cli.workspace.unwrap_or(std::env::current_dir()?);
            acp::serve(&runtime, &workspace)
        }
        Some(Commands::Backup { path }) => backup_state(cli.home.as_deref(), path),
        Some(Commands::Restore { path }) => restore_state(cli.home.as_deref(), &path),
        Some(Commands::User { command }) => {
            let runtime = Runtime::new(open_store(cli.home.as_deref())?);
            match command {
                UserCommands::Add { name } => {
                    let (user, token) = runtime.create_user(&name)?;
                    println!("{}  token={token}", user.id);
                }
                UserCommands::List => {
                    for user in runtime.list_users()? {
                        println!("{}  {}", user.id, user.name);
                    }
                }
            }
            Ok(())
        }
        Some(Commands::Plugin { command }) => {
            let workspace = cli.workspace.unwrap_or(std::env::current_dir()?);
            match command {
                PluginCommands::List => {
                    let market = blora_runtime::load_index()?;
                    for plugin in market.plugins {
                        println!("{}  {}", plugin.name, plugin.description);
                    }
                }
                PluginCommands::Install { name } => {
                    let path = blora_runtime::install_plugin(&workspace, &name)?;
                    println!("installed {}", path.display());
                }
                PluginCommands::Remove { name } => {
                    blora_runtime::uninstall_plugin(&workspace, &name)?;
                    println!("removed {name}");
                }
            }
            Ok(())
        }
        Some(Commands::Memory { command }) => {
            let runtime = Runtime::new(open_store(cli.home.as_deref())?);
            match command {
                MemoryCommands::Distill { session } => {
                    let count = runtime.distill_memories(&SessionId::parse(&session)?)?;
                    println!("stored {count} memories");
                }
            }
            Ok(())
        }
        Some(Commands::Usage { session }) => {
            let runtime = Runtime::new(open_store(cli.home.as_deref())?);
            let session_id = session.as_deref().map(SessionId::parse).transpose()?;
            let totals = runtime.usage(session_id.as_ref())?;
            println!(
                "input={} output={} cached={}",
                totals.input_tokens, totals.output_tokens, totals.cached_tokens
            );
            Ok(())
        }
        Some(other) => {
            let runtime = Runtime::new(open_store(cli.home.as_deref())?);
            dispatch(runtime, other, cli.workspace)
        }
    }
}

fn dispatch(
    runtime: Runtime,
    command: Commands,
    global_workspace: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Commands::License
        | Commands::Web { .. }
        | Commands::Serve { .. }
        | Commands::Acp
        | Commands::Backup { .. }
        | Commands::Restore { .. }
        | Commands::Usage { .. }
        | Commands::Gateway { .. }
        | Commands::User { .. }
        | Commands::Plugin { .. }
        | Commands::Memory { .. } => unreachable!(),
        Commands::Task { command } => match command {
            TaskCommands::Create {
                session,
                title,
                prompt,
                r#in,
                cron,
                mock,
                yes,
            } => {
                let delay_until = r#in.as_deref().map(parse_delay).transpose()?;
                let id = runtime.create_task(CreateTask {
                    session_id: SessionId::parse(&session)?,
                    title,
                    prompt,
                    delay_until,
                    max_attempts: 3,
                    auto_approve: yes,
                    mock,
                    cron,
                })?;
                println!("{id}");
            }
            TaskCommands::List { session } => {
                let session_id = session.as_deref().map(SessionId::parse).transpose()?;
                for task in runtime.list_tasks(session_id.as_ref())? {
                    println!(
                        "{}  {}  {}  attempt={}/{}",
                        task.id,
                        task.status.as_str(),
                        task.title,
                        task.attempt,
                        task.max_attempts
                    );
                }
            }
            TaskCommands::Show { id } => {
                let task = runtime.get_task(&TaskId::parse(&id)?)?;
                println!(
                    "{}  {}  session={}  delay={:?}\n{}",
                    task.id,
                    task.status.as_str(),
                    task.session_id,
                    task.delay_until,
                    task.prompt
                );
            }
            TaskCommands::Cancel { id } => {
                runtime.cancel_task(&TaskId::parse(&id)?)?;
                println!("cancelled {id}");
            }
            TaskCommands::Pause { id } => {
                runtime.pause_task(&TaskId::parse(&id)?)?;
                println!("paused {id}");
            }
            TaskCommands::Resume { id } => {
                runtime.resume_task(&TaskId::parse(&id)?)?;
                println!("resumed {id}");
            }
            TaskCommands::Pump => {
                let finished = runtime.pump()?;
                if finished.is_empty() {
                    println!("no due tasks");
                } else {
                    for id in finished {
                        println!("pumped {id}");
                    }
                }
            }
        },
        Commands::Tui { workspace } => {
            let workspace = workspace
                .or(global_workspace)
                .unwrap_or(std::env::current_dir()?);
            blora_tui::run(&runtime, &workspace)?;
        }
        Commands::Session { command } => match command {
            SessionCommands::Create {
                workspace,
                title,
                mode,
            } => {
                let id = runtime.create_session(CreateSession {
                    title,
                    workspace_path: workspace
                        .canonicalize()
                        .unwrap_or(workspace)
                        .display()
                        .to_string(),
                    mode: Mode::parse(&mode)?,
                    parent_session_id: None,
                })?;
                println!("{id}");
            }
            SessionCommands::List => {
                for session in runtime.list_sessions()? {
                    println!(
                        "{}  {}  {}  {}  seq={}",
                        session.id,
                        session.status.as_str(),
                        session.mode.as_str(),
                        session.title.as_deref().unwrap_or("-"),
                        session.last_sequence
                    );
                }
            }
            SessionCommands::Show { id } | SessionCommands::Replay { id } => {
                print_projection(&runtime, &SessionId::parse(&id)?)?;
            }
            SessionCommands::Compact { id } => {
                let summary = runtime.compact(&SessionId::parse(&id)?)?;
                println!("{summary}");
            }
            SessionCommands::Fork { id } => {
                let child = runtime.fork_session(&SessionId::parse(&id)?)?;
                println!("{child}");
            }
            SessionCommands::Export { id } => {
                let value = runtime.export_session(&SessionId::parse(&id)?)?;
                println!("{}", serde_json::to_string_pretty(&value)?);
            }
            SessionCommands::Archive { id } => {
                runtime.archive_session(&SessionId::parse(&id)?)?;
                println!("archived {id}");
            }
            SessionCommands::Resume { id } => {
                runtime.resume_session(&SessionId::parse(&id)?)?;
                println!("resumed {id}");
            }
            SessionCommands::Search { query } => {
                for session in runtime.search_sessions(&query)? {
                    println!(
                        "{}  {}  {}",
                        session.id,
                        session.status.as_str(),
                        session.title.as_deref().unwrap_or("-")
                    );
                }
            }
        },
        Commands::Run {
            session,
            prompt,
            mock,
            yes,
            permission,
            model,
            provider,
            worktree,
        } => {
            let session_id = SessionId::parse(&session)?;
            let auto_approve = permission.map_or(yes, PermissionMode::auto_approve);
            let run_id = runtime.run(
                &session_id,
                &prompt,
                &CancelToken::new(),
                &RunOptions {
                    model: model.unwrap_or_default(),
                    mock,
                    auto_approve,
                    interactive: false,
                    max_turns: 12,
                    provider: provider.unwrap_or_default(),
                    read_only: false,
                    worktree,
                    passport_user_token: None,
                    permission,
                },
            )?;
            println!("run {run_id}");
            print_projection(&runtime, &session_id)?;
        }
    }
    Ok(())
}

fn print_projection(
    runtime: &Runtime,
    session_id: &SessionId,
) -> Result<(), Box<dyn std::error::Error>> {
    let projection = runtime.show_session(session_id)?;
    if let Some(session) = projection.session {
        println!(
            "session {}  mode={}  workspace={}  seq={}",
            session.id,
            session.mode.as_str(),
            session.workspace_path,
            projection.last_sequence
        );
    }
    for task in &projection.tasks {
        println!(
            "  task {}  {}  {}",
            task.id,
            task.status.as_str(),
            task.title
        );
    }
    for agent in &projection.subagents {
        println!("  subagent {}  {}  {}", agent.id, agent.role, agent.status);
    }
    for run in projection.runs {
        println!(
            "  run {}  status={}  cancel={}",
            run.id,
            run.status.as_str(),
            run.cancel_requested
        );
    }
    for item in projection.transcript {
        match item {
            TranscriptItem::User { text, .. } => println!("  user: {text}"),
            TranscriptItem::Assistant { text, .. } => println!("  assistant: {text}"),
            TranscriptItem::Reasoning { text, .. } => println!("  reasoning: {text}"),
            TranscriptItem::Tool {
                name,
                status,
                arguments,
                output,
                ..
            } => {
                println!("  tool {name}: {status}");
                if let Some(arguments) = arguments {
                    println!("    {arguments}");
                }
                if let Some(output) = output {
                    for line in output.lines().take(12) {
                        println!("    {line}");
                    }
                }
            }
            TranscriptItem::System { summary, .. } => println!("  system: {summary}"),
            TranscriptItem::Routing {
                from_provider,
                to_provider,
                from_model,
                to_model,
                ..
            } => {
                println!(
                    "  routing: provider {} -> {to_provider}, model {} -> {to_model}",
                    from_provider.as_deref().unwrap_or("-"),
                    from_model.as_deref().unwrap_or("-"),
                );
            }
        }
    }
    if !projection.plan.is_empty() {
        println!("  plan:");
        for step in projection.plan {
            let mark = match step.status.as_str() {
                "done" => "x",
                "in_progress" => ">",
                _ => " ",
            };
            println!("    [{mark}] {}", step.title);
        }
    }
    Ok(())
}

fn parse_permission(text: &str) -> Result<PermissionMode, String> {
    PermissionMode::parse(text)
        .ok_or_else(|| format!("expected plan, ask, auto-edit, or yolo (got {text})"))
}

fn serve_web(
    home: Option<&std::path::Path>,
    bind: &str,
    workspace: Option<PathBuf>,
    require_auth: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = Arc::new(Runtime::new(open_store(home)?));
    let workspace = workspace.unwrap_or(std::env::current_dir()?);
    let addr: SocketAddr = bind.parse()?;
    let url = format!("http://{addr}");
    if require_auth {
        println!("Blora Agent gateway: {url}  (token auth required)");
    } else {
        println!("Blora Agent web: {url}");
        open_browser(&url);
    }
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(blora_server::serve(runtime, addr, workspace, require_auth))?;
    Ok(())
}

fn open_browser(url: &str) {
    let candidates = ["xdg-open", "open", "gio"];
    for command in candidates {
        if std::process::Command::new(command)
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
        {
            return;
        }
    }
}

fn parse_delay(spec: &str) -> Result<chrono::DateTime<Utc>, Box<dyn std::error::Error>> {
    let amount = spec
        .trim()
        .trim_end_matches(|ch: char| ch.is_ascii_alphabetic())
        .parse::<i64>()?;
    let duration = if spec.ends_with('s') {
        Duration::seconds(amount)
    } else if spec.ends_with('m') {
        Duration::minutes(amount)
    } else if spec.ends_with('h') {
        Duration::hours(amount)
    } else {
        return Err("delay must end with s, m, or h".into());
    };
    Ok(Utc::now() + duration)
}

fn backup_state(
    home: Option<&std::path::Path>,
    dest: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let home = resolve_home(home)?;
    let src = home.join("state.sqlite");
    if !src.exists() {
        return Err("no state.sqlite to back up".into());
    }
    let dest = dest.unwrap_or_else(|| {
        home.join("backups").join(format!(
            "state-{}.sqlite",
            Utc::now().format("%Y%m%dT%H%M%SZ")
        ))
    });
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    copy_sqlite(&src, &dest)?;
    println!("{}", dest.display());
    Ok(())
}

fn restore_state(
    home: Option<&std::path::Path>,
    src: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let dest = resolve_home(home)?.join("state.sqlite");
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    copy_sqlite(src, &dest)?;
    println!("restored {}", dest.display());
    Ok(())
}

fn copy_sqlite(
    src: &std::path::Path,
    dest: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::copy(src, dest)?;
    for suffix in ["-wal", "-shm"] {
        let extra = PathBuf::from(format!("{}{suffix}", src.display()));
        if extra.exists() {
            let target = PathBuf::from(format!("{}{suffix}", dest.display()));
            std::fs::copy(extra, target)?;
        }
    }
    Ok(())
}

fn resolve_home(home: Option<&std::path::Path>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(match home {
        Some(path) => path.to_path_buf(),
        None => match std::env::var_os("BLORA_HOME") {
            Some(value) => PathBuf::from(value),
            None => dirs::home_dir()
                .ok_or("cannot determine home directory")?
                .join(".blora"),
        },
    })
}

fn open_store(home: Option<&std::path::Path>) -> Result<SqliteStore, Box<dyn std::error::Error>> {
    Ok(SqliteStore::open(resolve_home(home)?.join("state.sqlite"))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn run_accepts_permission_mode() {
        let cli = Cli::try_parse_from([
            "blora",
            "run",
            "--session",
            "ses_x",
            "--permission",
            "auto-edit",
            "hi",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Run { permission, .. }) => {
                assert_eq!(permission, Some(PermissionMode::AutoEdit));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(
            Cli::try_parse_from(["blora", "run", "--session", "s", "--permission", "x", "hi"])
                .is_err()
        );
    }

    #[test]
    fn bare_blora_has_no_subcommand() {
        let cli = Cli::try_parse_from(["blora"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn web_subcommand_parses() {
        let cli = Cli::try_parse_from(["blora", "web", "--bind", "127.0.0.1:9000"]).unwrap();
        match cli.command {
            Some(Commands::Web { bind, .. }) => assert_eq!(bind, "127.0.0.1:9000"),
            other => panic!("expected web, got {other:?}"),
        }
    }

    #[test]
    fn web_flag_parses() {
        let cli = Cli::try_parse_from(["blora", "--web"]).unwrap();
        assert!(cli.web);
        assert!(cli.command.is_none());
        assert_eq!(cli.bind, "127.0.0.1:8787");
    }

    #[test]
    fn web_flag_accepts_bind() {
        let cli = Cli::try_parse_from(["blora", "--web", "--bind", "127.0.0.1:9000"]).unwrap();
        assert!(cli.web);
        assert_eq!(cli.bind, "127.0.0.1:9000");
    }

    #[test]
    fn backup_command_parses() {
        let cli = Cli::try_parse_from(["blora", "backup", "/tmp/state.sqlite"]).unwrap();
        match cli.command {
            Some(Commands::Backup { path }) => {
                assert_eq!(path.unwrap(), PathBuf::from("/tmp/state.sqlite"));
            }
            other => panic!("expected backup, got {other:?}"),
        }
    }
}
