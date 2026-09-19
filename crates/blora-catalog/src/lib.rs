// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Remote provider catalog (models.dev) and locally saved credentials.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use blora_types::{BloraError, Result};
use serde::{Deserialize, Serialize};

pub const MODELS_DEV_URL: &str = "https://models.dev/api.json";
pub const ADD_PROVIDER_ID: &str = "+";
pub const CUSTOM_PROVIDER_ID: &str = "__custom__";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageFormat {
    Openai,
    Anthropic,
    Gemini,
    Responses,
}

impl MessageFormat {
    pub const ALL: [Self; 4] = [Self::Openai, Self::Anthropic, Self::Gemini, Self::Responses];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Openai => "OpenAI Chat Completions",
            Self::Anthropic => "Anthropic Messages",
            Self::Gemini => "Google Gemini",
            Self::Responses => "OpenAI Responses",
        }
    }

    #[must_use]
    pub fn from_npm(npm: &str) -> Self {
        let n = npm.to_ascii_lowercase();
        if n.contains("anthropic") {
            Self::Anthropic
        } else if n.contains("google") || n.contains("gemini") {
            Self::Gemini
        } else if n.contains("responses") {
            Self::Responses
        } else {
            Self::Openai
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub api: Option<String>,
    pub env: Vec<String>,
    pub npm: Option<String>,
    pub models: Vec<CatalogModel>,
}

impl CatalogEntry {
    #[must_use]
    pub fn format(&self) -> MessageFormat {
        self.npm
            .as_deref()
            .map(MessageFormat::from_npm)
            .unwrap_or(MessageFormat::Openai)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedProvider {
    pub id: String,
    pub name: String,
    pub api_key: String,
    pub base_url: String,
    pub format: MessageFormat,
    #[serde(default)]
    pub models: Vec<CatalogModel>,
}

static CATALOG_CACHE: Mutex<Option<Vec<CatalogEntry>>> = Mutex::new(None);

/// Parse a models.dev `api.json` object (provider id → record).
pub fn parse_models_dev(value: &serde_json::Value) -> Vec<CatalogEntry> {
    let Some(map) = value.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(map.len());
    for (id, rec) in map {
        let models = rec
            .get("models")
            .and_then(|m| m.as_object())
            .map(|models| {
                models
                    .values()
                    .filter_map(|model| {
                        let mid = model.get("id")?.as_str()?.to_owned();
                        let name = model
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or(&mid)
                            .to_owned();
                        Some(CatalogModel { id: mid, name })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let env = rec
            .get("env")
            .and_then(|e| e.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        out.push(CatalogEntry {
            id: rec
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or(id)
                .to_owned(),
            name: rec
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(id)
                .to_owned(),
            api: rec
                .get("api")
                .and_then(|v| v.as_str())
                .map(|s| s.trim_end_matches('/').to_owned()),
            env,
            npm: rec
                .get("npm")
                .and_then(|v| v.as_str())
                .map(ToOwned::to_owned),
            models,
        });
    }
    out.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    out
}

pub fn fetch_catalog() -> Result<Vec<CatalogEntry>> {
    if let Ok(guard) = CATALOG_CACHE.lock()
        && let Some(cached) = guard.as_ref()
    {
        return Ok(cached.clone());
    }
    let body: serde_json::Value = ureq::get(MODELS_DEV_URL)
        .call()
        .map_err(|err| BloraError::provider(err.to_string()))?
        .into_json()
        .map_err(|err| BloraError::provider(err.to_string()))?;
    let list = parse_models_dev(&body);
    if let Ok(mut guard) = CATALOG_CACHE.lock() {
        *guard = Some(list.clone());
    }
    Ok(list)
}

pub fn cached_catalog() -> Vec<CatalogEntry> {
    CATALOG_CACHE
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default()
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("blora")
        .join("providers.json")
}

pub fn load_saved() -> Vec<SavedProvider> {
    let path = config_path();
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn find_saved(id: &str) -> Option<SavedProvider> {
    load_saved()
        .into_iter()
        .find(|item| item.id.eq_ignore_ascii_case(id))
}

pub fn save_provider(provider: SavedProvider) -> Result<()> {
    let mut all = load_saved();
    if let Some(existing) = all
        .iter_mut()
        .find(|item| item.id.eq_ignore_ascii_case(&provider.id))
    {
        *existing = provider;
    } else {
        all.push(provider);
    }
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| BloraError::Other(err.to_string()))?;
    }
    let json =
        serde_json::to_string_pretty(&all).map_err(|err| BloraError::Other(err.to_string()))?;
    fs::write(path, json).map_err(|err| BloraError::Other(err.to_string()))?;
    Ok(())
}

/// List models from an OpenAI-compatible `/models` endpoint.
pub fn fetch_openai_models(base_url: &str, api_key: &str) -> Result<Vec<CatalogModel>> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let body: serde_json::Value = ureq::get(&url)
        .set("Authorization", &format!("Bearer {api_key}"))
        .call()
        .map_err(|err| BloraError::provider(err.to_string()))?
        .into_json()
        .map_err(|err| BloraError::provider(err.to_string()))?;
    let mut models = Vec::new();
    if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
        for item in arr {
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            if id.is_empty() {
                continue;
            }
            models.push(CatalogModel {
                name: id.clone(),
                id,
            });
        }
    }
    Ok(models)
}

/// Merge catalog models with a live `/models` listing when the format is OpenAI-like.
pub fn resolve_models(
    entry: &CatalogEntry,
    format: MessageFormat,
    api_key: &str,
    base_url: &str,
) -> Vec<CatalogModel> {
    let mut models = entry.models.clone();
    if matches!(format, MessageFormat::Openai | MessageFormat::Responses)
        && !api_key.is_empty()
        && !base_url.is_empty()
        && let Ok(live) = fetch_openai_models(base_url, api_key)
    {
        let mut seen: BTreeMap<String, CatalogModel> = BTreeMap::new();
        for model in models.into_iter().chain(live) {
            seen.entry(model.id.clone()).or_insert(model);
        }
        models = seen.into_values().collect();
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_models_dev_fixture() {
        let raw = serde_json::json!({
            "openai": {
                "id": "openai",
                "name": "OpenAI",
                "npm": "@ai-sdk/openai",
                "env": ["OPENAI_API_KEY"],
                "models": {
                    "gpt-4o": { "id": "gpt-4o", "name": "GPT-4o" }
                }
            },
            "anthropic": {
                "id": "anthropic",
                "name": "Anthropic",
                "npm": "@ai-sdk/anthropic",
                "models": {
                    "claude": { "id": "claude-3-5-sonnet-latest", "name": "Claude" }
                }
            }
        });
        let list = parse_models_dev(&raw);
        assert_eq!(list.len(), 2);
        let openai = list.iter().find(|p| p.id == "openai").unwrap();
        assert_eq!(openai.models[0].id, "gpt-4o");
        assert_eq!(openai.format(), MessageFormat::Openai);
        let anthropic = list.iter().find(|p| p.id == "anthropic").unwrap();
        assert_eq!(anthropic.format(), MessageFormat::Anthropic);
    }

    #[test]
    fn npm_maps_google_to_gemini() {
        assert_eq!(
            MessageFormat::from_npm("@ai-sdk/google"),
            MessageFormat::Gemini
        );
        assert_eq!(
            MessageFormat::from_npm("@ai-sdk/openai-compatible"),
            MessageFormat::Openai
        );
    }
}
