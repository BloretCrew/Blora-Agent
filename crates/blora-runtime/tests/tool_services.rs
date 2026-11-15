// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_content::ToolOutcome;
use blora_exec::LocalBackend;
use blora_policy::{PermissionMode, Policy};
use blora_tools::{ToolRegistry, ToolServices};
use serde_json::json;

fn backend(dir: &tempfile::TempDir, services: ToolServices) -> (LocalBackend, ToolServices) {
    let backend = LocalBackend::new(
        Policy::with_mode(dir.path(), PermissionMode::Yolo)
            .unwrap()
            .with_network(Some(true)),
    );
    (backend, services)
}

#[test]
fn sdk_services_do_not_read_search_or_private_network_environment() {
    let mut services = ToolServices::sdk_default();
    services.search = Some(blora_web::SearchConfig {
        backend: blora_web::SearchBackend::Tavily,
        credential: "test-only-token".to_owned(),
        endpoint: Some("http://127.0.0.1:1/search".to_owned()),
    });
    let dir = tempfile::tempdir().unwrap();
    let (backend, services) = backend(&dir, services);
    let error = ToolRegistry::execute_outcome_with_services(
        &backend,
        "ses_test",
        "web_search",
        &json!({"query":"hello"}),
        &services,
    )
    .unwrap_err();
    // The explicit endpoint is intentionally unreachable, proving the request
    // used injected configuration rather than any process environment value.
    assert!(error.to_string().contains("127.0.0.1:1"), "{error}");
    assert!(!services.fetch.unwrap().allow_private);
}

#[test]
fn web_fetch_uses_explicit_public_only_options() {
    let mut services = ToolServices::sdk_default();
    services.search = Some(blora_web::SearchConfig {
        backend: blora_web::SearchBackend::Searxng,
        credential: "http://127.0.0.1:1".to_owned(),
        endpoint: None,
    });
    let dir = tempfile::tempdir().unwrap();
    let (backend, services) = backend(&dir, services);
    let error = ToolRegistry::execute_outcome_with_services(
        &backend,
        "ses_test",
        "web_fetch",
        &json!({"url":"http://127.0.0.1/secret"}),
        &services,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("private or local")
            || error.to_string().contains("local network")
    );
}

#[test]
fn tool_services_require_search_config() {
    let dir = tempfile::tempdir().unwrap();
    let (backend, services) = backend(&dir, ToolServices::sdk_default());
    let error = ToolRegistry::execute_outcome_with_services(
        &backend,
        "ses_test",
        "web_search",
        &json!({"query":"hello"}),
        &services,
    )
    .unwrap_err();
    assert!(error.to_string().contains("not configured"));
    assert_eq!(ToolOutcome::text("ok").as_text(), "ok");
}
