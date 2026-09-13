// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::path::{Path, PathBuf};

use blora_types::{BloraError, Result};
use serde::Deserialize;

use crate::plugins::PluginSpec;

#[derive(Clone, Debug, Deserialize)]
pub struct Marketplace {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub plugins: Vec<PluginSpec>,
}

pub fn index_path() -> PathBuf {
    if let Ok(path) = std::env::var("BLORA_MARKETPLACE") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../marketplace/index.json")
}

pub fn load_index() -> Result<Marketplace> {
    let path = index_path();
    let text = std::fs::read_to_string(&path).map_err(BloraError::storage)?;
    serde_json::from_str(&text).map_err(|err| BloraError::event(err.to_string()))
}

pub fn install(workspace: &Path, name: &str) -> Result<PathBuf> {
    let market = load_index()?;
    let spec = market
        .plugins
        .iter()
        .find(|plugin| plugin.name == name)
        .ok_or_else(|| BloraError::Other(format!("plugin {name} is not in the marketplace")))?;
    let dir = workspace.join(".blora/plugins");
    std::fs::create_dir_all(&dir).map_err(BloraError::storage)?;
    let path = dir.join(format!("{name}.json"));
    let body = serde_json::json!({
        "name": spec.name,
        "description": spec.description,
        "command": spec.command,
        "parameters": spec.parameters,
    });
    std::fs::write(&path, serde_json::to_string_pretty(&body).unwrap())
        .map_err(BloraError::storage)?;
    Ok(path)
}

pub fn uninstall(workspace: &Path, name: &str) -> Result<()> {
    let path = workspace
        .join(".blora/plugins")
        .join(format!("{name}.json"));
    if path.exists() {
        std::fs::remove_file(path).map_err(BloraError::storage)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ships_builtin_catalog() {
        let market = load_index().unwrap();
        assert!(market.plugins.iter().any(|plugin| plugin.name == "echo"));
    }
}
