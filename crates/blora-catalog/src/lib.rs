// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Remote provider catalog (models.dev) and locally saved credentials.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

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
    /// Context window in tokens (`limit.context` on models.dev), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
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
                        let context = model
                            .get("limit")
                            .and_then(|limit| limit.get("context"))
                            .and_then(serde_json::Value::as_u64)
                            .filter(|tokens| *tokens > 0);
                        Some(CatalogModel {
                            id: mid,
                            name,
                            context,
                        })
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

/// Context window for `model` from the cached catalog, saved providers, or
/// well-known model families. `None` when nothing is known.
#[must_use]
pub fn context_limit(model: &str) -> Option<u64> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    let bare = model.rsplit('/').next().unwrap_or(model);
    let from_list = |models: &[CatalogModel]| {
        models
            .iter()
            .find(|candidate| candidate.id == model || candidate.id == bare)
            .and_then(|candidate| candidate.context)
    };
    cached_catalog()
        .iter()
        .find_map(|entry| from_list(&entry.models))
        .or_else(|| {
            load_saved()
                .iter()
                .find_map(|saved| from_list(&saved.models))
        })
        .or_else(|| family_context(bare))
}

/// Conservative windows for common model families when no catalog is loaded.
fn family_context(model: &str) -> Option<u64> {
    let id = model.to_ascii_lowercase();
    let table: &[(&str, u64)] = &[
        ("claude", 200_000),
        ("gemini", 1_000_000),
        ("gpt-4.1", 1_000_000),
        ("gpt-5", 400_000),
        ("gpt-4o", 128_000),
        ("o3", 200_000),
        ("o4", 200_000),
        ("grok-4", 256_000),
        ("grok-code", 256_000),
        ("deepseek", 128_000),
        ("qwen", 128_000),
        ("kimi", 128_000),
        ("glm", 128_000),
    ];
    table
        .iter()
        .find(|(prefix, _)| id.starts_with(prefix) || id.contains(&format!("-{prefix}")))
        .map(|(_, tokens)| *tokens)
}

/// Where saved providers live: `dirs::config_dir()/blora/providers.json`.
#[must_use]
pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("blora")
        .join("providers.json")
}

static SAVED_LOCK: Mutex<()> = Mutex::new(());

/// Lenient read for display paths. A missing, unreadable, or malformed file
/// yields an empty list; writes go through [`save_provider`], which refuses to
/// overwrite a file it could not parse.
#[must_use]
pub fn load_saved() -> Vec<SavedProvider> {
    read_saved(&config_path()).unwrap_or_default()
}

#[must_use]
pub fn find_saved(id: &str) -> Option<SavedProvider> {
    load_saved()
        .into_iter()
        .find(|item| item.id.eq_ignore_ascii_case(id))
}

/// Read saved providers from `path`.
///
/// A missing file is an empty list. An unreadable, symlinked, or malformed file
/// is an error so a writer never replaces data it could not see.
fn read_saved(path: &Path) -> Result<Vec<SavedProvider>> {
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(BloraError::Other(format!(
            "{} 是符号链接，拒绝读写供应商配置",
            path.display()
        )));
    }
    if path.is_dir() {
        return Err(BloraError::Other(format!(
            "{} 是一个目录，无法作为供应商配置；请移走该目录后重试",
            path.display()
        )));
    }
    match fs::read_to_string(path) {
        Ok(raw) if raw.trim().is_empty() => Ok(Vec::new()),
        Ok(raw) => serde_json::from_str(&raw).map_err(|err| {
            BloraError::Other(format!(
                "无法解析 {}：{err}；为避免覆盖已保存的供应商，请修正或删除该文件后重试",
                path.display()
            ))
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(io_error("读取", path, &err)),
    }
}

fn io_error(action: &str, path: &Path, err: &std::io::Error) -> BloraError {
    let mut message = format!("{action} {} 失败：{err}", path.display());
    match err.kind() {
        std::io::ErrorKind::PermissionDenied => {
            message.push_str("；请确认该文件不是只读、不是同名目录，且所在目录可写");
        }
        // `create_dir_all` only reports this when a file blocks the path.
        std::io::ErrorKind::AlreadyExists => {
            message.push_str("；同名路径已存在但不是目录，请移走后重试");
        }
        _ => {}
    }
    BloraError::Other(message)
}

/// Add or replace one provider and persist atomically.
pub fn save_provider(provider: SavedProvider) -> Result<()> {
    save_provider_at(&config_path(), provider)
}

fn save_provider_at(path: &Path, provider: SavedProvider) -> Result<()> {
    let _lock = SAVED_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut all = read_saved(path)?;
    if let Some(existing) = all
        .iter_mut()
        .find(|item| item.id.eq_ignore_ascii_case(&provider.id))
    {
        *existing = provider;
    } else {
        all.push(provider);
    }
    let raw = serde_json::to_vec_pretty(&all).map_err(|err| BloraError::Other(err.to_string()))?;
    write_atomic(path, &raw)
}

/// Write `bytes` through a same-directory temp file and a rename, so a failed
/// write never truncates the previous configuration.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| BloraError::Other(format!("{} 没有父目录", path.display())))?;
    fs::create_dir_all(parent).map_err(|err| io_error("创建目录", parent, &err))?;
    if path.is_dir() {
        return Err(BloraError::Other(format!(
            "{} 是一个目录，无法写入供应商配置；请移走该目录后重试",
            path.display()
        )));
    }
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(BloraError::Other(format!(
            "{} 是符号链接，拒绝覆盖",
            path.display()
        )));
    }
    prepare_replace(path)?;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let temp = parent.join(format!(".providers-{}-{unique}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .map_err(|err| io_error("创建临时文件", &temp, &err))?;
    use std::io::Write;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = written
        .map_err(|err| io_error("写入临时文件", &temp, &err))
        .and_then(|()| fs::rename(&temp, path).map_err(|err| io_error("替换", path, &err)));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Windows refuses to replace a read-only file and reports os error 5. This
/// file belongs to Blora, so clear the attribute before the rename.
///
/// The lint's Unix caveat does not apply: this is Windows-only, where clearing
/// read-only simply drops `FILE_ATTRIBUTE_READONLY`.
#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false)]
fn prepare_replace(path: &Path) -> Result<()> {
    let Ok(meta) = fs::metadata(path) else {
        return Ok(());
    };
    if !meta.permissions().readonly() {
        return Ok(());
    }
    let mut permissions = meta.permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).map_err(|err| io_error("清除只读属性", path, &err))
}

#[cfg(not(windows))]
fn prepare_replace(_path: &Path) -> Result<()> {
    Ok(())
}

/// OpenAI-compatible bases usually live under `/v1`. Users often paste the
/// host without that suffix; keep both candidates so `/models` still works.
#[must_use]
pub fn openai_base_candidates(base_url: &str) -> Vec<String> {
    let base = base_url.trim().trim_end_matches('/').to_owned();
    if base.is_empty() {
        return Vec::new();
    }
    let mut out = vec![base.clone()];
    let already_versioned = base.ends_with("/v1")
        || base.ends_with("/v1beta")
        || base.contains("/v1/")
        || base.contains("/v1beta/");
    if !already_versioned {
        out.push(format!("{base}/v1"));
    }
    out
}

/// List models from an OpenAI-compatible `/models` endpoint.
pub fn fetch_openai_models(base_url: &str, api_key: &str) -> Result<Vec<CatalogModel>> {
    let mut last_err: Option<BloraError> = None;
    for base in openai_base_candidates(base_url) {
        let url = format!("{base}/models");
        match fetch_openai_models_url(&url, api_key) {
            Ok(models) => return Ok(models),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.unwrap_or_else(|| BloraError::provider("缺少 API 基址")))
}

fn fetch_openai_models_url(url: &str, api_key: &str) -> Result<Vec<CatalogModel>> {
    let body: serde_json::Value = ureq::get(url)
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
            let name = item
                .get("name")
                .or_else(|| item.get("description"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(id.as_str())
                .to_owned();
            models.push(CatalogModel {
                name,
                id,
                context: None,
            });
        }
    }
    Ok(models)
}

/// Merge catalog models with a live `/models` listing when the format is OpenAI-like.
/// Empty live listings still keep catalog models. A failed live fetch with no
/// catalog models is an error so the add-provider wizard can show it.
pub fn resolve_models(
    entry: &CatalogEntry,
    format: MessageFormat,
    api_key: &str,
    base_url: &str,
) -> Result<Vec<CatalogModel>> {
    let mut models = entry.models.clone();
    if matches!(format, MessageFormat::Openai | MessageFormat::Responses)
        && !api_key.is_empty()
        && !base_url.is_empty()
    {
        match fetch_openai_models(base_url, api_key) {
            Ok(live) => {
                let mut seen: BTreeMap<String, CatalogModel> = BTreeMap::new();
                for model in models.into_iter().chain(live) {
                    seen.entry(model.id.clone()).or_insert(model);
                }
                models = seen.into_values().collect();
            }
            Err(err) if models.is_empty() => return Err(err),
            Err(_) => {}
        }
    }
    Ok(models)
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
                    "gpt-4o": { "id": "gpt-4o", "name": "GPT-4o", "limit": { "context": 128000, "output": 16384 } }
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
        assert_eq!(openai.models[0].context, Some(128_000));
        assert_eq!(openai.format(), MessageFormat::Openai);
        let anthropic = list.iter().find(|p| p.id == "anthropic").unwrap();
        assert_eq!(anthropic.format(), MessageFormat::Anthropic);
    }

    #[test]
    fn family_context_covers_common_models() {
        assert_eq!(family_context("claude-sonnet-4-5"), Some(200_000));
        assert_eq!(family_context("gemini-2.5-pro"), Some(1_000_000));
        assert_eq!(family_context("mystery"), None);
        assert_eq!(context_limit(""), None);
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

    #[test]
    fn openai_base_adds_v1_when_missing() {
        assert_eq!(
            openai_base_candidates("https://router.bloret.net"),
            vec![
                "https://router.bloret.net".to_owned(),
                "https://router.bloret.net/v1".to_owned(),
            ]
        );
        assert_eq!(
            openai_base_candidates("https://api.openai.com/v1/"),
            vec!["https://api.openai.com/v1".to_owned()]
        );
    }

    fn provider(id: &str) -> SavedProvider {
        SavedProvider {
            id: id.to_owned(),
            name: format!("{id} name"),
            api_key: format!("{id}-key"),
            base_url: "https://example.com/v1".to_owned(),
            format: MessageFormat::Openai,
            models: Vec::new(),
        }
    }

    #[test]
    fn save_provider_creates_file_and_replaces_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blora/providers.json");

        save_provider_at(&path, provider("alpha")).unwrap();
        save_provider_at(&path, provider("beta")).unwrap();
        let mut updated = provider("alpha");
        updated.api_key = "rotated".to_owned();
        save_provider_at(&path, updated).unwrap();

        let saved = read_saved(&path).unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(
            saved
                .iter()
                .find(|item| item.id == "alpha")
                .unwrap()
                .api_key,
            "rotated"
        );
    }

    #[test]
    fn malformed_file_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("providers.json");
        fs::write(&path, b"{not json").unwrap();

        let err = save_provider_at(&path, provider("alpha"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("无法解析"), "unexpected error: {err}");
        // The original bytes survive so the user can repair them.
        assert_eq!(fs::read_to_string(&path).unwrap(), "{not json");
    }

    #[test]
    fn directory_at_config_path_is_reported_without_deleting_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("providers.json");
        fs::create_dir(&path).unwrap();

        let err = save_provider_at(&path, provider("alpha"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("是一个目录"), "unexpected error: {err}");
        assert!(path.is_dir());
    }

    #[test]
    fn atomic_write_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("providers.json");
        save_provider_at(&path, provider("alpha")).unwrap();

        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
    }

    /// Reproduces the reported `os error 5`: Windows refuses to rename over a
    /// read-only file. Writing must clear the attribute instead of failing.
    #[cfg(windows)]
    #[test]
    fn read_only_config_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("providers.json");
        save_provider_at(&path, provider("alpha")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();

        save_provider_at(&path, provider("beta")).unwrap();

        let saved = read_saved(&path).unwrap();
        assert_eq!(saved.len(), 2);
    }

    #[test]
    fn empty_file_reads_as_no_providers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("providers.json");
        fs::write(&path, b"  \n").unwrap();
        assert!(read_saved(&path).unwrap().is_empty());
    }

    #[test]
    fn missing_file_reads_as_no_providers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing/providers.json");
        assert!(read_saved(&path).unwrap().is_empty());
    }
}
