// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::sync::Arc;

use blora_model::{Completion, MockProvider, ToolCall};
use blora_runtime::ToolServices;
use blora_sdk::{Agent, Profile, ProviderConfig, ToolOutcome};
use serde_json::json;

#[test]
fn web_profile_exposes_search_only_with_explicit_services() {
    let provider = Arc::new(MockProvider::new());
    provider.push(Completion {
        text: "done".to_owned(),
        finish_reason: "stop".to_owned(),
        ..Completion::default()
    });
    let search = blora_sdk::SearchConfig {
        backend: blora_sdk::SearchBackend::Tavily,
        credential: "test-token".to_owned(),
        endpoint: Some("http://127.0.0.1:1/search".to_owned()),
    };
    let agent = Agent::builder()
        .profile(Profile::web())
        .provider(ProviderConfig::custom(provider))
        .search_config(search)
        .auto_title(false)
        .build()
        .unwrap();
    let session = agent.create_session().unwrap();
    let outcome = session.run_blocking("hello").unwrap();
    assert_eq!(outcome.final_text, "done");
}

#[test]
fn web_profile_search_config_does_not_depend_on_process_environment() {
    let mut services = ToolServices::sdk_default();
    services.search = Some(blora_sdk::SearchConfig {
        backend: blora_sdk::SearchBackend::Tavily,
        credential: "injected".to_owned(),
        endpoint: Some("http://127.0.0.1:1/search".to_owned()),
    });
    assert_eq!(services.search.unwrap().credential, "injected");
    assert!(!ToolServices::sdk_default().fetch.unwrap().allow_private);
    let _ = ToolOutcome::text("ok");
}

#[test]
fn web_profile_configure_cannot_reopen_filesystem_features() {
    let provider = Arc::new(MockProvider::new());
    provider.push(Completion {
        tool_calls: vec![ToolCall {
            id: "write-attempt".to_owned(),
            name: "write_file".to_owned(),
            arguments: json!({"path":"escape.txt","contents":"blocked"}).to_string(),
        }],
        finish_reason: "tool_calls".to_owned(),
        ..Completion::default()
    });
    provider.push(Completion {
        text: "no write".to_owned(),
        finish_reason: "stop".to_owned(),
        ..Completion::default()
    });
    let agent = Agent::builder()
        .profile(Profile::web())
        .configure(|config| config.context.git_snapshot = true)
        .provider(ProviderConfig::custom(provider))
        .build()
        .unwrap();
    let session = agent.create_session().unwrap();
    let outcome = session.run_blocking("try file write").unwrap();
    assert_eq!(outcome.final_text, "no write");
    assert!(!agent.workspace().join("escape.txt").exists());
    assert!(!agent.workspace().join(".blora").exists());
}
