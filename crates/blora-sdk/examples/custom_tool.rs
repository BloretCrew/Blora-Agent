// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::sync::Arc;

use blora_model::{Completion, MockProvider, ToolCall};
use blora_sdk::{Agent, Profile, ProviderConfig, ToolOutcome};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = Arc::new(MockProvider::new());
    provider.push(Completion {
        tool_calls: vec![ToolCall {
            id: "call_echo".to_owned(),
            name: "echo".to_owned(),
            arguments: json!({"text": "hello from the host"}).to_string(),
        }],
        finish_reason: "tool_calls".to_owned(),
        ..Completion::default()
    });
    provider.push(Completion {
        text: "The host tool returned the text.".to_owned(),
        finish_reason: "stop".to_owned(),
        ..Completion::default()
    });

    let agent = Agent::builder()
        .profile(Profile::web())
        .provider(ProviderConfig::custom(provider))
        .tool(blora_sdk::tool(
            "echo",
            "Echo text back",
            json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"]
            }),
            true,
            |_ctx, arguments| {
                let text = arguments
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                Ok(ToolOutcome::text(text))
            },
        ))
        .build()?;
    let session = agent.create_session()?;
    let outcome = session.run_blocking("Use the echo tool.")?;
    println!("{:?}: {}", outcome.status, outcome.final_text);
    Ok(())
}
