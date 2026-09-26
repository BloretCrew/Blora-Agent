// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! GitHub Actions entry for Blora Agent.
//!
//! A repository collaborator mentions `/blora`, `/ba`, or `@blora`. The workflow
//! runs `blora github run`, which reads the comment and lets that request decide
//! whether to reply, edit, push, or open a pull request.

mod api;
mod attach;
mod context;
mod error;
mod event;
mod git;
mod http;
mod mention;
mod prompt;
mod run;
mod workflow;

pub use error::GithubError;
pub use git::{CommandRepo, Repo};
pub use http::{Http, UreqHttp};
pub use mention::DEFAULT_MENTIONS;
pub use run::{AgentDriver, AgentRequest, AgentResponse, Config, Report, Status, execute};
pub use workflow::{WorkflowOptions, install_instructions, install_workflow, render_workflow};
