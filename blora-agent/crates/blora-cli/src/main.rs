// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Blora Agent command-line interface.

use std::path::PathBuf;

use blora_runtime::{CancelToken, MockRunOptions, Runtime};
use blora_session::TranscriptItem;
use blora_storage::{CreateSession, SqliteStore};
use blora_types::{Mode, SessionId};
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
    about = "Blora Agent: a local-first Code, Work, and Agent harness.",
    after_help = LICENSE_NOTICE
)]
struct Cli {
    /// Override Blora home directory (default: $BLORA_HOME or ~/.blora).
    #[arg(long, global = true, env = "BLORA_HOME")]
    home: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print license information.
    License,
    /// Session commands.
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },
    /// Run a mock agent turn against a session.
    Run {
        /// Session identifier (`ses_…`).
        #[arg(long)]
        session: String,
        /// User prompt.
        prompt: String,
    },
}

#[derive(Subcommand)]
enum SessionCommands {
    /// Create a new session.
    Create {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value = "code")]
        mode: String,
    },
    /// List sessions.
    List,
    /// Show a reconstructed session projection.
    Show { id: String },
    /// Replay a session from its event log.
    Replay { id: String },
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
        Commands::License => {
            println!("{LICENSE_NOTICE}");
            Ok(())
        }
        other => {
            let runtime = Runtime::new(open_store(cli.home.as_deref())?);
            dispatch(runtime, other)
        }
    }
}

fn dispatch(runtime: Runtime, command: Commands) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Commands::License => unreachable!(),
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
                        "{}  {}  {}  seq={}",
                        session.id,
                        session.mode.as_str(),
                        session.title.as_deref().unwrap_or("-"),
                        session.last_sequence
                    );
                }
            }
            SessionCommands::Show { id } | SessionCommands::Replay { id } => {
                print_projection(&runtime, &SessionId::parse(&id)?)?;
            }
        },
        Commands::Run { session, prompt } => {
            let session_id = SessionId::parse(&session)?;
            let run_id = runtime.run_mock(
                &session_id,
                &prompt,
                &CancelToken::new(),
                &MockRunOptions::default(),
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
            TranscriptItem::Tool { name, status, .. } => {
                println!("  tool {name}: {status}");
            }
            TranscriptItem::System { summary, .. } => println!("  system: {summary}"),
        }
    }
    Ok(())
}

fn open_store(home: Option<&std::path::Path>) -> Result<SqliteStore, Box<dyn std::error::Error>> {
    let home = match home {
        Some(path) => path.to_path_buf(),
        None => match std::env::var_os("BLORA_HOME") {
            Some(value) => PathBuf::from(value),
            None => dirs::home_dir()
                .ok_or("cannot determine home directory")?
                .join(".blora"),
        },
    };
    Ok(SqliteStore::open(home.join("state.sqlite"))?)
}
