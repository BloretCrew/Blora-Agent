// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Small, server-side Bloret PassPort OAuth client.
//!
//! The default app credential is only used for the server-side verification request;
//! it is never placed in authorization URLs or browser-facing values.

use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use ureq::OrAnyStatus;

pub const DEFAULT_PASSPORT_URL: &str = "https://passport.bloret.net";
pub const DEFAULT_PASSPORT_APP_ID: &str = "bp_98a98eeb52be6618";
pub const DEFAULT_PASSPORT_APP_SECRET: &str = "bs_00c2e065fbcc17f844499c2e9814ea3368c999477c179884";

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
            .unwrap_or_else(|_| DEFAULT_PASSPORT_APP_SECRET.to_owned());
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

    pub fn request_device_code(&self) -> Result<DeviceCode, AuthError> {
        let body = format!(
            "client_id={}&scope={}",
            encode_form(&self.app_id),
            encode_form("user:name user:head user:email app:usertoken"),
        );
        let response = ureq::post(&format!("{}/oauth/device/code", self.base_url))
            .set("Content-Type", "application/x-www-form-urlencoded")
            .set("Accept", "application/json")
            .timeout(Duration::from_secs(20))
            .send_string(&body)
            .or_any_status()
            .map_err(|err| AuthError::Network(err.to_string()))?;
        let status = response.status();
        let value: Value = response
            .into_json()
            .map_err(|err| AuthError::Protocol(err.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(AuthError::Rejected(error_message(&value, status)));
        }
        let device_code = value
            .get("device_code")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AuthError::Protocol("Passport response has no device_code".to_owned())
            })?;
        let user_code = value
            .get("user_code")
            .and_then(Value::as_str)
            .ok_or_else(|| AuthError::Protocol("Passport response has no user_code".to_owned()))?;
        let verification_uri = value
            .get("verification_uri_complete")
            .or_else(|| value.get("verification_uri"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AuthError::Protocol("Passport response has no verification_uri".to_owned())
            })?;
        Ok(DeviceCode {
            device_code: device_code.to_owned(),
            user_code: user_code.to_owned(),
            verification_uri: verification_uri.to_owned(),
            expires_in: value
                .get("expires_in")
                .and_then(Value::as_u64)
                .unwrap_or(600),
            interval: value.get("interval").and_then(Value::as_u64).unwrap_or(5),
        })
    }

    pub fn device_code<'a>(&self, device: &'a DeviceCode) -> &'a str {
        &device.device_code
    }

    pub fn verification_uri<'a>(&self, device: &'a DeviceCode) -> &'a str {
        &device.verification_uri
    }

    pub fn poll_device(&self, device: &DeviceCode) -> Result<PassportUser, AuthError> {
        let deadline = std::time::Instant::now() + Duration::from_secs(device.expires_in);
        let mut interval = device.interval.max(1);
        loop {
            if std::time::Instant::now() >= deadline {
                return Err(AuthError::Rejected("expired_token".to_owned()));
            }
            thread::sleep(Duration::from_secs(interval));
            let body = format!(
                "client_id={}&grant_type={}&device_code={}",
                encode_form(&self.app_id),
                encode_form("urn:ietf:params:oauth:grant-type:device_code"),
                encode_form(&device.device_code),
            );
            let response = ureq::post(&format!("{}/oauth/token", self.base_url))
                .set("Content-Type", "application/x-www-form-urlencoded")
                .set("Accept", "application/json")
                .timeout(Duration::from_secs(20))
                .send_string(&body)
                .or_any_status()
                .map_err(|err| AuthError::Network(err.to_string()))?;
            let status = response.status();
            let value: Value = response
                .into_json()
                .map_err(|err| AuthError::Protocol(err.to_string()))?;
            if let Some(access_token) = value.get("access_token").and_then(Value::as_str) {
                let mut user = self.userinfo(access_token)?;
                // PassPort AI accepts the OAuth access_token as Bearer. Store
                // that credential rather than a profile-only apptoken.
                user.apptoken = Some(access_token.to_owned());
                return Ok(user);
            }
            let error = value.get("error").and_then(Value::as_str).unwrap_or("");
            match error {
                "authorization_pending" => continue,
                "slow_down" => {
                    interval = interval.saturating_add(5);
                }
                "access_denied" | "expired_token" | "invalid_client" | "invalid_scope" => {
                    return Err(AuthError::Rejected(error_message(&value, status)));
                }
                _ if !(200..300).contains(&status) => {
                    return Err(AuthError::Rejected(error_message(&value, status)));
                }
                _ => {
                    return Err(AuthError::Protocol(
                        "Passport token response has no access_token".to_owned(),
                    ));
                }
            }
        }
    }

    fn userinfo(&self, access_token: &str) -> Result<PassportUser, AuthError> {
        let response = ureq::get(&format!("{}/oauth/userinfo", self.base_url))
            .set("Accept", "application/json")
            .set("Authorization", &format!("Bearer {access_token}"))
            .timeout(Duration::from_secs(20))
            .call()
            .or_any_status()
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
            .or_any_status()
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceCode {
    pub(crate) device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

impl DeviceCode {
    #[must_use]
    pub fn from_public(
        device_code: &str,
        user_code: &str,
        verification_uri: &str,
        expires_in: u64,
        interval: u64,
    ) -> Self {
        Self {
            device_code: device_code.to_owned(),
            user_code: user_code.to_owned(),
            verification_uri: verification_uri.to_owned(),
            expires_in,
            interval,
        }
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
        apptoken: first_string_field(&value, &["apptoken", "app_token", "usertoken", "user_token"]),
    })
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn first_string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| string_field(value, key))
}

fn error_message(value: &Value, status: u16) -> String {
    value
        .get("error")
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("HTTP {status}"))
}

fn encode_form(value: &str) -> String {
    value.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
        out
    })
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
    fn defaults_include_the_registered_app() {
        let config = PassportConfig::from_env().unwrap();
        assert_eq!(config.app_id, DEFAULT_PASSPORT_APP_ID);
        assert_eq!(config.app_secret, DEFAULT_PASSPORT_APP_SECRET);
    }

    #[test]
    fn poll_device_keeps_polling_through_authorization_pending() {
        use std::io::Read;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut pending = true;
            loop {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let mut stream = stream;
                let mut buffer = [0u8; 4096];
                let Ok(read) = Read::read(&mut stream, &mut buffer) else {
                    continue;
                };
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                if request.starts_with("POST /oauth/token") {
                    if pending {
                        pending = false;
                        write_response(
                            &mut stream,
                            400,
                            "Bad Request",
                            br#"{"error":"authorization_pending"}"#,
                        );
                    } else {
                        write_response(
                            &mut stream,
                            200,
                            "OK",
                            br#"{"access_token":"t","token_type":"bearer"}"#,
                        );
                    }
                    continue;
                }
                if request.starts_with("GET /oauth/userinfo") {
                    write_response(
                        &mut stream,
                        200,
                        "OK",
                        br#"{"username":"alice","nickname":"Alice"}"#,
                    );
                    return;
                }
                write_response(
                    &mut stream,
                    404,
                    "Not Found",
                    br#"{"error":"unsupported_endpoint"}"#,
                );
            }
        });

        let config = PassportConfig::new("bp_app", "bs_secret", format!("http://{addr}")).unwrap();
        let device = DeviceCode::from_public("dc", "ABCD-EFGH", "https://passport.example", 60, 1);
        let user = config.poll_device(&device).unwrap();
        assert_eq!(user.username, "alice");
        assert_eq!(user.display_name(), "Alice");
        assert_eq!(
            user.apptoken.as_deref(),
            Some("t"),
            "device login stores the OAuth access_token for the AI API"
        );
        server.join().unwrap();
    }

    #[test]
    fn parse_user_reads_usertoken_alias() {
        let user = parse_user(serde_json::json!({
            "username": "jiedi",
            "usertoken": "tok-1"
        }))
        .unwrap();
        assert_eq!(user.apptoken.as_deref(), Some("tok-1"));
    }

    fn write_response(stream: &mut std::net::TcpStream, status: u16, text: &str, body: &[u8]) {
        use std::io::Write;
        let head = format!(
            "HTTP/1.1 {status} {text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
    }

    #[test]
    fn rejects_insecure_config_shape() {
        let err = PassportConfig::new("", "secret", DEFAULT_PASSPORT_URL).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }
}
