// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Bloret PassPort AI API provider.
//!
//! PassPort proxies a fixed upstream model, so the caller-supplied `model` is
//! always reported as `blora` regardless of what the request asks for. Bearer
//! auth accepts `sk-…` keys, the legacy `{AppID};{AppSecret};{UserToken}`
//! triple, or an OAuth `access_token` from the device/authorization-code flow.

use blora_types::{BloraError, CancelToken, Result};

use crate::{
    Completion, CompletionRequest, Provider, StreamEvent,
    openai::{chat_completions_body, post_chat_completions},
};

pub const PASSPORT_API_BASE_URL: &str = "https://passport.bloret.net/v1";
/// Model name shown to users and sent to the PassPort API.
pub const PASSPORT_MODEL_NAME: &str = "blora";
/// Display name shown in the Web UI and docs.
pub const PASSPORT_PROVIDER_DISPLAY_NAME: &str = "Blora";

pub struct PassportProvider {
    pub api_key: String,
}

impl PassportProvider {
    /// Use a PassPort Bearer credential as-is: OAuth access token, `sk-` key,
    /// or a legacy three-part `{AppID};{AppSecret};{UserToken}` string.
    #[must_use]
    pub fn from_bearer(token: &str) -> Self {
        Self {
            api_key: token.trim().to_owned(),
        }
    }

    /// Three-part OAuth key: `{AppID};{AppSecret};{UserToken}`.
    pub fn from_parts(app_id: &str, app_secret: &str, user_token: &str) -> Self {
        Self::from_bearer(&format!("{app_id};{app_secret};{user_token}"))
    }

    #[must_use]
    pub fn display_name() -> &'static str {
        PASSPORT_PROVIDER_DISPLAY_NAME
    }
}

impl Provider for PassportProvider {
    fn name(&self) -> &str {
        "blora"
    }

    fn complete(
        &self,
        request: &CompletionRequest,
        cancel: &CancelToken,
        on_event: &mut dyn FnMut(StreamEvent) -> Result<()>,
    ) -> Result<Completion> {
        if cancel.is_cancelled() {
            return Err(BloraError::Cancelled);
        }
        // PassPort pins a single upstream model and overrides whatever we send;
        // report the canonical public name instead.
        let user_request = CompletionRequest {
            model: PASSPORT_MODEL_NAME.to_owned(),
            messages: request.messages.clone(),
            tools: request.tools.clone(),
            max_output_tokens: request.max_output_tokens,
            cache_key: request.cache_key.clone(),
        };
        let body = chat_completions_body(&user_request, PASSPORT_MODEL_NAME);
        let reader = post_chat_completions(PASSPORT_API_BASE_URL, &self.api_key, body).map_err(
            |err| {
                let text = err.to_string();
                if text.contains("401") {
                    BloraError::provider(
                        "PassPort 认证失败：OAuth access_token 无效或已过期，请使用 /login 重新登录",
                    )
                } else {
                    err
                }
            },
        )?;
        crate::openai::parse_sse(reader, cancel, on_event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_blora_and_key_is_three_part() {
        let provider = PassportProvider::from_parts("bp_app", "bs_secret", "tok");
        assert_eq!(provider.name(), "blora");
        assert_eq!(provider.api_key, "bp_app;bs_secret;tok");
        assert_eq!(PassportProvider::display_name(), "Blora");
    }

    #[test]
    fn oauth_access_token_is_a_plain_bearer() {
        let provider = PassportProvider::from_bearer("oauth-access");
        assert_eq!(provider.api_key, "oauth-access");
    }
}
