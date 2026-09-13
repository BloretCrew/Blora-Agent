// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Small, server-side Bloret PassPort OAuth client.
//!
//! The app secret is intentionally supplied by the caller at runtime and is never
//! represented in configuration files or browser-facing values.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_PASSPORT_URL: &str = "https://passport.bloret.net";
pub const DEFAULT_PASSPORT_APP_ID: &str = "bp_98a98eeb52be6618";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassportConfig {
    pub app_id: String,
    pub app_secret: String,
    pub base_url: String,
}

impl PassportConfig {
    pub fn from_env() -> Result<Self, AuthError> {
        let app_id = std::env::var("BLORA_PASSPORT_APP_ID")
            .unwrap_or_else(|_| DEFAULT_PASSPORT_APP_ID.to_owned());
        let app_secret = std::env::var("BLORA_PASSPORT_APP_SECRET")
            .map_err(|_| AuthError::Config("BLORA_PASSPORT_APP_SECRET is not set".to_owned()))?;
        let base_url =
            std::env::var("BLORA_PASSPORT_URL").unwrap_or_else(|_| DEFAULT_PASSPORT_URL.to_owned());
        Self::new(app_id, app_secret, base_url)
    }

    pub fn new(
        app_id: impl Into<String>,
        app_secret: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, AuthError> {
        let app_id = app_id.into();
        let app_secret = app_secret.into();
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        if app_id.trim().is_empty() || app_secret.trim().is_empty() {
            return Err(AuthError::Config(
                "Passport app credentials are empty".to_owned(),
            ));
        }
        if !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
            return Err(AuthError::Config(
                "Passport URL must use http or https".to_owned(),
            ));
        }
        Ok(Self {
            app_id,
            app_secret,
            base_url,
        })
    }

    #[must_use]
    pub fn authorize_url(&self, redirect_uri: &str) -> String {
        format!(
            "{}/app/oauth?app_id={}&redirect_uri={}",
            self.base_url,
            encode(&self.app_id),
            encode(redirect_uri),
        )
    }

    pub fn verify_code(&self, code: &str) -> Result<PassportUser, AuthError> {
        if code.trim().is_empty() {
            return Err(AuthError::Protocol(
                "Passport callback code is empty".to_owned(),
            ));
        }
        let body = serde_json::json!({"code": code});
        let response = ureq::post(&format!("{}/app/verify", self.base_url))
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .set("X-App-Id", &self.app_id)
            .set("X-App-Secret", &self.app_secret)
            .timeout(Duration::from_secs(20))
            .send_json(body)
            .map_err(|err| AuthError::Network(err.to_string()))?;
        let status = response.status();
        let value: Value = response
            .into_json()
            .map_err(|err| AuthError::Protocol(err.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(AuthError::Rejected(error_message(&value, status)));
        }
        parse_user(value)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PassportUser {
    pub username: String,
    pub nickname: Option<String>,
    pub avatar: Option<String>,
    pub email: Option<String>,
    pub apptoken: Option<String>,
}

impl PassportUser {
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.nickname
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.username)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("authentication configuration error: {0}")]
    Config(String),
    #[error("authentication network error: {0}")]
    Network(String),
    #[error("authentication protocol error: {0}")]
    Protocol(String),
    #[error("Passport rejected authentication: {0}")]
    Rejected(String),
}

fn parse_user(value: Value) -> Result<PassportUser, AuthError> {
    let username = value
        .get("username")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| AuthError::Protocol("Passport response has no username".to_owned()))?;
    Ok(PassportUser {
        username: username.to_owned(),
        nickname: string_field(&value, "nickname"),
        avatar: string_field(&value, "avatar"),
        email: string_field(&value, "email"),
        apptoken: string_field(&value, "apptoken"),
    })
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn error_message(value: &Value, status: u16) -> String {
    value
        .get("error")
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("HTTP {status}"))
}

fn encode(value: &str) -> String {
    value.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_authorize_url_without_secret() {
        let config = PassportConfig::new("bp_app", "bs_secret", DEFAULT_PASSPORT_URL).unwrap();
        let url = config.authorize_url("http://127.0.0.1:4000/auth/callback");
        assert!(url.contains("app_id=bp_app"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A4000%2Fauth%2Fcallback"));
        assert!(!url.contains("bs_secret"));
    }

    #[test]
    fn display_name_falls_back_to_username() {
        let user = PassportUser {
            username: "alice".to_owned(),
            ..PassportUser::default()
        };
        assert_eq!(user.display_name(), "alice");
    }

    #[test]
    fn rejects_insecure_config_shape() {
        let err = PassportConfig::new("", "secret", DEFAULT_PASSPORT_URL).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }
}
