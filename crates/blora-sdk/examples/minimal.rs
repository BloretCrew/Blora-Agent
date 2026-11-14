// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_sdk::{Agent, Profile, ProviderConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .profile(Profile::full())
        .workspace(".")
        .memory()
        .provider(ProviderConfig::mock())
        .auto_title(false)
        .build()?;
    let session = agent.create_session()?;
    let outcome = session.run_blocking("Say hello.")?;
    println!("{:?}: {}", outcome.status, outcome.final_text);
    Ok(())
}
