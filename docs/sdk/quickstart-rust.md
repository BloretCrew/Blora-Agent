# Rust SDK quickstart

`blora-sdk` embeds the existing Blora Agent runtime in a Rust process. The crate
is GPL-3.0-or-later; use the stdio sidecar protocol for Node.js and Python.

## Add the dependency

```toml
[dependencies]
blora-sdk = { git = "https://github.com/BloretCrew/Blora-Agent" }
```

The first release is a workspace-coupled SDK; its internal crates are not yet
published independently to crates.io.

## Web/chat profile

```rust,no_run
use blora_sdk::{Agent, Profile, ProviderConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .profile(Profile::web())
        .provider(ProviderConfig::openai(
            std::env::var("MODEL_KEY")?,
            "chat-model",
        ))
        .build()?;

    let session = agent.create_session()?;
    let outcome = session.run_blocking("How do I reset my password?")?;
    println!("{}", outcome.final_text);
    agent.shutdown()?;
    Ok(())
}
```

The web profile creates a private temporary workspace, defaults to in-memory
SQLite, disables shell/filesystem/process tools and global `~/.blora` discovery,
and uses `AllowReadOnly` approvals plus dismissed questions. Provider credentials
must be supplied explicitly; the SDK never silently falls back to a mock provider.

## Register a host tool

```rust,no_run
use blora_sdk::{tool, Agent, Profile, ProviderConfig, ToolOutcome};
use serde_json::json;
use std::sync::Arc;

let lookup = tool(
    "lookup_order",
    "Look up an order by ID",
    json!({
        "type": "object",
        "properties": {"id": {"type": "string"}},
        "required": ["id"]
    }),
    true,
    |_ctx, args| {
        let id = args["id"].as_str().unwrap_or_default();
        Ok(ToolOutcome::text(format!("Order {id}: shipped")))
    },
);

let agent = Agent::builder()
    .profile(Profile::web())
    .provider(ProviderConfig::openai("key", "model"))
    .tool(lookup)
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Tools are registered before the first run. `read_only` tools can run without
approval; state-changing tools default to requiring approval.

## Events and cancellation

`Session::run()` returns a `RunHandle` on a worker thread. `RunHandle::events()`
replays the canonical event log by sequence and fills gaps from storage. Call
`cancel()`, `approve()`, or `answer()` while the run is waiting. Dropping the
handle cancels the run unless `detach()` was called.

## Storage and licensing

`Storage::memory()` and `Storage::file(path)` use SQLite. The separate
`Store` trait describes the first custom-storage seam; custom stores are not yet
wired into the Agent builder in this release. Rust linking and distribution are
subject to GPL-3.0-or-later; Node/Python use a process boundary instead.
