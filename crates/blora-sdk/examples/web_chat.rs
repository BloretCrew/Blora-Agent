// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::sync::Arc;

use blora_model::{Completion, MockProvider};
use blora_sdk::{Agent, Profile, ProviderConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = Arc::new(MockProvider::new());
    provider.push(Completion {
        text: "Open Settings, then choose Reset password.".to_owned(),
        finish_reason: "stop".to_owned(),
        ..Completion::default()
    });

    let agent = Agent::builder()
        .profile(Profile::web())
        .provider(ProviderConfig::custom(provider))
        .build()?;
    let session = agent.create_session()?;
    let outcome = session.run_blocking("How do I reset my password?")?;
    println!("{:?}: {}", outcome.status, outcome.final_text);
    Ok(())
}
