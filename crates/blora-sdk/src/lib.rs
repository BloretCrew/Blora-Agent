// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Embeddable Blora Agent SDK.
//!
//! The facade runs the existing `blora-runtime` Agent Loop in-process and
//! deliberately does not depend on the CLI, TUI, or web server.
//!
//! ```no_run
//! use blora_sdk::{Agent, Profile, ProviderConfig, Storage};
//!
//! let agent = Agent::builder()
//!     .profile(Profile::full())
//!     .workspace(".")
//!     .storage(Storage::memory())
//!     .provider(ProviderConfig::mock())
//!     .build()?;
//! let session = agent.create_session()?;
//! let outcome = session.run_blocking("hello")?;
//! println!("{:?}: {}", outcome.status, outcome.final_text);
//! # Ok::<(), blora_sdk::SdkError>(())
//! ```

mod agent;
mod error;
mod events;
mod extensions;
mod profile;
mod provider;
mod run;
mod tool;

pub use agent::{Agent, AgentBuilder, Session, SessionSpec, Storage};
pub use error::{Result, SdkError};
pub use events::EventStream;
pub use extensions::{
    HostToolTable, ReadOnlyBackend, Store, check_cancel, listen, text_tool_result, tool_metadata,
};
pub use profile::Profile;
pub use provider::ProviderConfig;
pub use run::{RunHandle, RunOutcome, RunRequest, Usage};
pub use tool::{FnTool, Tool, ToolCallContext, ToolDefinition, ToolError, tool};

/// Re-exported canonical types used by SDK consumers.
pub use blora_content::{AttachmentRef, ContentPart, ToolOutcome};
pub use blora_events::{EventEnvelope, KnownPayload};
pub use blora_runtime::PermissionMode;
pub use blora_runtime::{
    ApprovalDecision, ApprovalHandler, ApprovalPolicy, ApprovalRequest, CallbackConfig,
    ContextConfig, Features, HomeMode, Limits, PromptConfig, ProviderSource, QuestionAnswer,
    QuestionHandler, QuestionPolicy, QuestionRequest, RuntimeConfig, ToolPolicy,
};
pub use blora_session::{SessionProjection, TranscriptItem};
pub use blora_types::{ApprovalId, BloraError, CancelToken, Mode, RunId, RunStatus, SessionId};
pub use blora_web::{FetchOptions, SearchBackend, SearchConfig};

/// SDK protocol version understood by the Rust facade.
pub const SDK_PROTOCOL_VERSION: &str = "1.0";
