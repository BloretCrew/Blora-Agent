// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Tool discovery catalog.
//!
//! Built-ins are always visible; MCP and plugin tools participate in
//! `tool_search` and become declared only after the model enables them for the
//! session.

use blora_model::ToolDeclaration;
use blora_tools::ToolRegistry;
use blora_types::Mode;

use crate::plugins::PluginSpec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolSource {
    Builtin,
    Mcp,
    Plugin,
    Host,
}

impl ToolSource {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Mcp => "mcp",
            Self::Plugin => "plugin",
            Self::Host => "host",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ToolEntry {
    pub name: String,
    pub description: String,
    pub source: ToolSource,
    pub read_only: bool,
    #[allow(dead_code)]
    pub declaration: ToolDeclaration,
}

#[must_use]
pub fn entries(
    mode: Mode,
    read_only_run: bool,
    mcp_tools: &[ToolDeclaration],
    plugins: &[PluginSpec],
) -> Vec<ToolEntry> {
    let mut out = Vec::new();
    for spec in ToolRegistry::specs_for(mode) {
        if read_only_run && !spec.read_only {
            continue;
        }
        out.push(ToolEntry {
            name: spec.name.to_owned(),
            description: spec.description.to_owned(),
            source: ToolSource::Builtin,
            read_only: spec.read_only,
            declaration: ToolDeclaration {
                name: spec.name.to_owned(),
                description: spec.description.to_owned(),
                parameters: spec.parameters,
            },
        });
    }
    if mode != Mode::Imagine {
        for tool in mcp_tools {
            if read_only_run {
                continue;
            }
            out.push(ToolEntry {
                name: tool.name.clone(),
                description: tool.description.clone(),
                source: ToolSource::Mcp,
                read_only: false,
                declaration: tool.clone(),
            });
        }
        for plugin in plugins {
            if read_only_run {
                continue;
            }
            let declaration = plugin.declaration();
            out.push(ToolEntry {
                name: declaration.name.clone(),
                description: declaration.description.clone(),
                source: ToolSource::Plugin,
                read_only: false,
                declaration,
            });
        }
    }
    out
}

#[must_use]
pub fn search(
    entries: &[ToolEntry],
    query: Option<&str>,
    select: &[String],
    source: Option<&str>,
    limit: usize,
) -> Vec<ToolEntry> {
    let limit = limit.clamp(1, 50);
    if !select.is_empty() {
        return entries
            .iter()
            .filter(|entry| {
                select
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(&entry.name))
            })
            .filter(|entry| source.is_none_or(|filter| entry.source.as_str() == filter))
            .take(limit)
            .cloned()
            .collect();
    }
    let needle = query.map(str::to_ascii_lowercase).unwrap_or_default();
    let mut scored = entries
        .iter()
        .filter(|entry| source.is_none_or(|filter| entry.source.as_str() == filter))
        .filter_map(|entry| {
            if needle.is_empty() {
                return Some((0usize, entry));
            }
            let name = entry.name.to_ascii_lowercase();
            let description = entry.description.to_ascii_lowercase();
            if name == needle {
                Some((0, entry))
            } else if name.contains(&needle) {
                Some((1, entry))
            } else if description.contains(&needle) {
                Some((2, entry))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, entry)| entry.clone())
        .collect()
}

#[must_use]
pub fn format_matches(matches: &[ToolEntry]) -> String {
    if matches.is_empty() {
        return "no matching tools".to_owned();
    }
    matches
        .iter()
        .map(|entry| {
            format!(
                "{} [{}] {}{}",
                entry.name,
                entry.source.as_str(),
                entry.description.lines().next().unwrap_or(""),
                if entry.read_only { " (read-only)" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_by_name_and_source() {
        let plugins = vec![PluginSpec {
            name: "echo".to_owned(),
            description: "Echo a value".to_owned(),
            command: "echo {value}".to_owned(),
            parameters: serde_json::json!({"type": "object"}),
        }];
        let entries = entries(Mode::Code, false, &[], &plugins);
        let hits = search(&entries, Some("echo"), &[], Some("plugin"), 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "plugin__echo");
        assert!(hits[0].declaration.description.contains("Echo"));
    }

    #[test]
    fn select_is_exact() {
        let entries = entries(Mode::Code, false, &[], &[]);
        let hits = search(&entries, None, &["read_file".to_owned()], None, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "read_file");
    }
}
