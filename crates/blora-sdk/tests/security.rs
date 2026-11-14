// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::sync::Arc;

use blora_model::{Completion, MockProvider, ToolCall};
use blora_sdk::{Agent, Profile, ProviderConfig, SessionId};
use serde_json::json;

fn tool_completion(name: &str, arguments: serde_json::Value) -> Completion {
    Completion {
        tool_calls: vec![ToolCall {
            id: format!("call_{name}"),
            name: name.to_owned(),
            arguments: arguments.to_string(),
        }],
        finish_reason: "tool_calls".to_owned(),
        ..Completion::default()
    }
}

#[test]
fn sdk_default_rejects_environment_provider_without_opt_in() {
    let result = Agent::builder()
        .profile(Profile::web())
        .provider(ProviderConfig::from_env())
        .build();
    assert!(result.is_err());
}

#[test]
fn profile_web_cannot_reenable_filesystem_tools_via_builder_config() {
    let provider = Arc::new(MockProvider::new());
    provider.push(tool_completion(
        "write_file",
        json!({"path": "secret.txt", "contents": "no"}),
    ));
    provider.push(Completion {
        text: "blocked".to_owned(),
        finish_reason: "stop".to_owned(),
        ..Completion::default()
    });
    let agent = Agent::builder()
        .profile(Profile::web())
        .configure(|config| {
            config.context.git_snapshot = true;
        })
        .provider(ProviderConfig::custom(provider))
        .build()
        .unwrap();
    let session = agent.create_session().unwrap();
    let result = session.run_blocking("write").unwrap();
    assert_eq!(result.final_text, "blocked");
    assert!(!agent.workspace().join("secret.txt").exists());
    assert!(!agent.workspace().join(".blora").exists());
}

#[test]
fn sessions_cannot_escape_the_web_profile_workspace() {
    let provider = Arc::new(MockProvider::new());
    let agent = Agent::builder()
        .profile(Profile::web())
        .provider(ProviderConfig::custom(provider))
        .build()
        .unwrap();
    let outside = tempfile::tempdir().unwrap();
    let result = agent.create_session_with(blora_sdk::SessionSpec {
        workspace: Some(outside.path().to_path_buf()),
        ..Default::default()
    });
    assert!(result.is_err());
}

#[test]
fn sdk_store_trait_is_implemented_by_sqlite() {
    let store = blora_storage::SqliteStore::open_in_memory().unwrap();
    let store: Arc<dyn blora_sdk::Store> = Arc::new(store);
    let workspace = tempfile::tempdir().unwrap();
    let id = store
        .create_session(blora_storage::CreateSession {
            title: Some("store contract".to_owned()),
            workspace_path: workspace.path().display().to_string(),
            mode: blora_types::Mode::Code,
            parent_session_id: None,
        })
        .unwrap();
    assert_eq!(store.last_sequence(&id).unwrap(), 1);
    assert_eq!(store.load_projection(&id).unwrap().last_sequence, 1);
}

#[test]
fn session_id_type_is_available_in_facade() {
    let id = SessionId::generate();
    assert!(id.as_str().starts_with("ses_"));
}
