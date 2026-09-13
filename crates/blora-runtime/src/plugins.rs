// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::path::Path;

use blora_exec::LocalBackend;
use blora_model::ToolDeclaration;
use blora_types::{BloraError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginSpec {
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub parameters: Value,
}

impl PluginSpec {
    #[must_use]
    pub fn tool_name(&self) -> String {
        format!("plugin__{}", self.name)
    }

    #[must_use]
    pub fn declaration(&self) -> ToolDeclaration {
        ToolDeclaration {
            name: self.tool_name(),
            description: self.description.clone(),
            parameters: if self.parameters.is_null() {
                json!({"type": "object", "properties": {}})
            } else {
                self.parameters.clone()
            },
        }
    }
}

pub fn load(workspace: &Path) -> Vec<PluginSpec> {
    let mut dirs = vec![workspace.join(".blora/plugins")];
    if let Ok(extra) = std::env::var("BLORA_PLUGINS_DIR") {
        dirs.push(Path::new(&extra).to_path_buf());
    }
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            match serde_json::from_str::<PluginSpec>(&text) {
                Ok(spec) if !spec.name.is_empty() && !spec.command.is_empty() => out.push(spec),
                Ok(_) => tracing::warn!("plugin {} is missing name or command", path.display()),
                Err(err) => tracing::warn!("plugin {} is invalid: {err}", path.display()),
            }
        }
    }
    out
}

pub fn execute(
    backend: &LocalBackend,
    plugins: &[PluginSpec],
    name: &str,
    arguments: &Value,
) -> Result<String> {
    let raw = name.strip_prefix("plugin__").unwrap_or(name);
    let spec = plugins
        .iter()
        .find(|plugin| plugin.name == raw)
        .ok_or_else(|| BloraError::Other(format!("unknown plugin {name}")))?;
    let mut command = spec.command.clone();
    if let Some(object) = arguments.as_object() {
        for (key, value) in object {
            if !key
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            {
                continue;
            }
            let needle = format!("{{{key}}}");
            let replacement = match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            command = command.replace(&needle, &replacement);
        }
    }
    backend.shell(&command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_json_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join(".blora/plugins");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("echo.json"),
            r#"{"name":"echo","description":"echo","command":"printf %s {text}"}"#,
        )
        .unwrap();
        let plugins = load(dir.path());
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].tool_name(), "plugin__echo");
    }
}
