// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Runtime UI translations backed by the Bloret Translation Collector API.

use std::collections::BTreeMap;
use std::env;
use std::sync::{OnceLock, RwLock};

const BASE_URL: &str = "https://tr.bloret.net";
const ORG: &str = "bloret";
const PROJECT: &str = "blora-agent";

static CATALOG: OnceLock<RwLock<Catalog>> = OnceLock::new();

#[derive(Clone, Debug)]
struct Catalog {
    locale: String,
    values: BTreeMap<String, String>,
}

pub fn init() -> String {
    let locale = env::var("BLORA_LOCALE").unwrap_or_else(|_| "zh-CN".to_owned());
    let mut values = embedded_source();
    if locale == "en" {
        values.extend(
            serde_json::from_str::<BTreeMap<String, String>>(include_str!("../i18n/en.json"))
                .unwrap_or_default(),
        );
    }
    if locale != "zh-CN" {
        if let Ok(remote) = fetch_translation(&locale) {
            values.extend(
                remote
                    .into_iter()
                    .filter(|(_, value)| !value.trim().is_empty()),
            );
        }
    }
    let catalog = CATALOG.get_or_init(|| {
        RwLock::new(Catalog {
            locale: "zh-CN".to_owned(),
            values: embedded_source(),
        })
    });
    if let Ok(mut current) = catalog.write() {
        current.locale = locale.clone();
        current.values = values;
    }
    locale
}

#[allow(dead_code)]
pub fn locale() -> String {
    CATALOG
        .get()
        .and_then(|catalog| catalog.read().ok().map(|value| value.locale.clone()))
        .unwrap_or_else(|| "zh-CN".to_owned())
}

#[allow(dead_code)]
pub fn tr(key: &str, fallback: &str) -> String {
    CATALOG
        .get()
        .and_then(|catalog| {
            catalog
                .read()
                .ok()
                .and_then(|value| value.values.get(key).cloned())
        })
        .unwrap_or_else(|| fallback.to_owned())
}

fn embedded_source() -> BTreeMap<String, String> {
    serde_json::from_str(include_str!("../i18n/zh-CN.json")).unwrap_or_default()
}

fn fetch_manifest_file_id(base: &str, org: &str, project: &str) -> Result<String, String> {
    let url = format!("{base}/api/v1/orgs/{org}/projects/{project}/manifest");
    let response = ureq::get(&url).call().map_err(|error| error.to_string())?;
    let body: serde_json::Value = response.into_json().map_err(|error| error.to_string())?;
    body.get("project")
        .and_then(|project| project.get("defaultFileId"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            body.get("files")
                .and_then(serde_json::Value::as_array)
                .and_then(|files| files.first())
                .and_then(|file| file.get("id"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .ok_or_else(|| "translation project has no source file".to_owned())
}

fn fetch_translation(locale: &str) -> Result<BTreeMap<String, String>, String> {
    let base = env::var("BLORA_TRANSLATION_BASE_URL").unwrap_or_else(|_| BASE_URL.to_owned());
    let org = env::var("BLORA_TRANSLATION_ORG").unwrap_or_else(|_| ORG.to_owned());
    let project = env::var("BLORA_TRANSLATION_PROJECT").unwrap_or_else(|_| PROJECT.to_owned());
    let file_id = match env::var("BLORA_TRANSLATION_FILE_ID") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => fetch_manifest_file_id(&base, &org, &project)?,
    };
    let url = format!(
        "{base}/api/v1/orgs/{org}/projects/{project}/files/{file_id}/translated?locale={locale}&mode=top_voted&fallbackMt=1"
    );
    let response = ureq::get(&url)
        .set("Accept", "application/json")
        .call()
        .map_err(|error| error.to_string())?;
    let body: serde_json::Value = response.into_json().map_err(|error| error.to_string())?;
    let object = body
        .get("translations")
        .or_else(|| body.get("texts"))
        .unwrap_or(&body);
    serde_json::from_value(object.clone()).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_translations_cover_both_locales_and_preserve_placeholders() {
        let source = embedded_source();
        let english: BTreeMap<String, String> =
            serde_json::from_str(include_str!("../i18n/en.json")).unwrap();
        for (key, value) in &source {
            if key.starts_with("permission.") || key == "dialog.permission" {
                let translated = english.get(key).expect("English permission translation");
                assert!(!value.trim().is_empty());
                assert!(!translated.trim().is_empty());
                assert_eq!(value.contains("{mode}"), translated.contains("{mode}"));
            }
        }
        assert_eq!(source["permission.default"], "默认确认");
        assert_eq!(source["permission.accept_edits"], "自动接受编辑");
        assert_eq!(source["permission.plan"], "计划");
        assert_eq!(source["permission.bypass"], "跳过权限确认");
    }

    #[test]
    fn source_catalog_contains_stable_ui_keys() {
        let values = embedded_source();
        assert_eq!(
            values.get("dialog.context_usage").map(String::as_str),
            Some("上下文用量")
        );
        assert_eq!(values.get("action.close").map(String::as_str), Some("关闭"));
        assert_eq!(
            values.get("dialog.mode").map(String::as_str),
            Some("运行模式")
        );
    }
}

pub fn permission_name(mode: blora_policy::PermissionMode) -> String {
    use blora_policy::PermissionMode;
    let (key, fallback) = match mode {
        PermissionMode::Ask => ("permission.default", "默认确认"),
        PermissionMode::AutoEdit => ("permission.accept_edits", "自动接受编辑"),
        PermissionMode::Plan => ("permission.plan", "计划"),
        PermissionMode::Yolo => ("permission.bypass", "跳过权限确认"),
    };
    tr(key, fallback)
}

pub fn permission_changed(mode: blora_policy::PermissionMode) -> String {
    tr("permission.changed", "权限模式：{mode}").replace("{mode}", &permission_name(mode))
}
