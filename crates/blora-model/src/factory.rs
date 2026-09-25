// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use crate::{
    AnthropicProvider, GeminiProvider, MockProvider, OpenAiProvider, PassportProvider, Provider,
    ResponsesProvider,
};

/// Build a provider from `BLORA_PROVIDER` or an explicit name.
/// Comma-separated names become a fallback chain (`openai,anthropic`).
#[must_use]
pub fn make_provider(force_mock: bool, name: &str) -> Box<dyn Provider> {
    let mut chain = make_providers(force_mock, name);
    if chain.len() == 1 {
        chain.remove(0)
    } else {
        Box::new(FallbackProvider { inner: chain })
    }
}

#[must_use]
pub fn make_providers(force_mock: bool, name: &str) -> Vec<Box<dyn Provider>> {
    resolve_provider_chain(force_mock, name, None)
}

/// Whether the TUI/server should force the mock adapter.
///
/// Mock is only the fallback when there is no PassPort user token, no env API
/// key, and no explicit non-mock provider (for example a saved CrewRouter).
#[must_use]
pub fn should_auto_mock(requested: &str, user_token: Option<&str>) -> bool {
    if user_token.is_some_and(|token| !token.trim().is_empty()) {
        return false;
    }
    if env_has_provider_key() {
        return false;
    }
    let name = requested.trim();
    if name.is_empty() || is_passport_alias(name) {
        return true;
    }
    name.eq_ignore_ascii_case("mock")
}

fn is_passport_alias(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "blora" | "passport" | "bloret-passport"
    )
}

fn env_has_provider_key() -> bool {
    [
        "BLORA_API_KEY",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "GEMINI_API_KEY",
    ]
    .iter()
    .any(|key| std::env::var(key).is_ok_and(|value| !value.trim().is_empty()))
}

/// Resolve the provider chain for an optional logged-in PassPort user.
///
/// Default provider is Bloret PassPort (`blora`). An explicit request or
/// `BLORA_PROVIDER` overrides it.
#[must_use]
pub fn resolve_provider_chain(
    force_mock: bool,
    requested: &str,
    user_token: Option<&str>,
) -> Vec<Box<dyn Provider>> {
    if force_mock {
        return vec![Box::new(MockProvider::new())];
    }
    let explicit = !requested.trim().is_empty();
    let spec = if explicit {
        requested.to_owned()
    } else {
        std::env::var("BLORA_PROVIDER").unwrap_or_else(|_| "blora".to_owned())
    };
    let mut out = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.to_ascii_lowercase().as_str() {
            name if is_passport_alias(name) => out.push(
                user_token
                    .and_then(passport_provider)
                    .unwrap_or_else(|| Box::new(MockProvider::new())),
            ),
            _ => out.push(one(part)),
        }
    }
    if out.is_empty() {
        out.push(Box::new(MockProvider::new()));
    }
    out
}

fn passport_provider(user_token: &str) -> Option<Box<dyn Provider>> {
    let user_token = user_token.trim();
    if user_token.is_empty() {
        return None;
    }
    Some(Box::new(PassportProvider::from_bearer(user_token)))
}

fn one(kind: &str) -> Box<dyn Provider> {
    if let Some(saved) = blora_catalog::find_saved(kind) {
        return saved_provider(&saved);
    }
    match kind.to_ascii_lowercase().as_str() {
        "mock" => Box::new(MockProvider::new()),
        "anthropic" | "claude" => AnthropicProvider::from_env()
            .map(|provider| Box::new(provider) as Box<dyn Provider>)
            .unwrap_or_else(|_| Box::new(MockProvider::new())),
        "responses" => ResponsesProvider::from_env()
            .map(|provider| Box::new(provider) as Box<dyn Provider>)
            .unwrap_or_else(|_| Box::new(MockProvider::new())),
        "gemini" | "google" => GeminiProvider::from_env()
            .map(|provider| Box::new(provider) as Box<dyn Provider>)
            .unwrap_or_else(|_| Box::new(MockProvider::new())),
        _ => OpenAiProvider::from_env()
            .map(|provider| Box::new(provider) as Box<dyn Provider>)
            .unwrap_or_else(|_| Box::new(MockProvider::new())),
    }
}

fn saved_provider(saved: &blora_catalog::SavedProvider) -> Box<dyn Provider> {
    let model = saved
        .models
        .first()
        .map(|m| m.id.as_str())
        .unwrap_or("gpt-4o-mini");
    let openai_base = blora_catalog::openai_base_candidates(&saved.base_url)
        .into_iter()
        .find(|base| base.ends_with("/v1"))
        .unwrap_or_else(|| saved.base_url.clone());
    match saved.format {
        blora_catalog::MessageFormat::Anthropic => Box::new(AnthropicProvider {
            base_url: saved.base_url.clone(),
            api_key: saved.api_key.clone(),
            model: model.to_owned(),
        }),
        blora_catalog::MessageFormat::Gemini => Box::new(GeminiProvider {
            api_key: saved.api_key.clone(),
            model: model.to_owned(),
            base_url: saved.base_url.clone(),
        }),
        blora_catalog::MessageFormat::Responses => Box::new(ResponsesProvider {
            base_url: openai_base,
            api_key: saved.api_key.clone(),
            model: model.to_owned(),
        }),
        blora_catalog::MessageFormat::Openai => Box::new(OpenAiProvider::with_identity(
            saved.id.clone(),
            openai_base,
            saved.api_key.clone(),
            model,
        )),
    }
}

struct FallbackProvider {
    inner: Vec<Box<dyn Provider>>,
}

impl Provider for FallbackProvider {
    fn name(&self) -> &str {
        self.inner
            .first()
            .map(AsRef::as_ref)
            .map(Provider::name)
            .unwrap_or("none")
    }

    fn complete(
        &self,
        request: &crate::CompletionRequest,
        cancel: &blora_types::CancelToken,
        on_event: &mut dyn FnMut(crate::StreamEvent) -> blora_types::Result<()>,
    ) -> blora_types::Result<crate::Completion> {
        let mut last = None;
        for (index, provider) in self.inner.iter().enumerate() {
            match provider.complete(request, cancel, on_event) {
                Ok(completion) => return Ok(completion),
                Err(blora_types::BloraError::Cancelled) => {
                    return Err(blora_types::BloraError::Cancelled);
                }
                Err(err) if index + 1 < self.inner.len() => last = Some(err),
                Err(err) => return Err(err),
            }
        }
        Err(last.unwrap_or_else(|| blora_types::BloraError::provider("no providers configured")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_force_wins() {
        assert_eq!(make_provider(true, "openai").name(), "mock");
    }

    #[test]
    fn comma_chain_uses_first_name() {
        assert_eq!(make_provider(true, "openai,anthropic").name(), "mock");
        let chain = make_providers(true, "openai,anthropic");
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn logged_in_user_defaults_to_blora() {
        let chain = resolve_provider_chain(false, "", Some("tok"));
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].name(), "blora");
    }

    #[test]
    fn logged_in_user_keeps_explicit_provider() {
        // Explicit non-blora request wins over the login default.
        let chain = resolve_provider_chain(false, "mock", Some("tok"));
        assert_eq!(chain[0].name(), "mock");
    }

    #[test]
    fn anonymous_session_defaults_to_passport() {
        let chain = resolve_provider_chain(false, "", None);
        // No user token yet, so the PassPort adapter cannot run and falls
        // through to mock — but the requested default is still `blora`.
        assert!(matches!(chain[0].name(), "blora" | "mock"));
        let logged_in = resolve_provider_chain(false, "", Some("tok"));
        assert_eq!(logged_in[0].name(), "blora");
    }

    #[test]
    fn auto_mock_skips_explicit_saved_vendor() {
        assert!(
            should_auto_mock("", None),
            "anonymous default PassPort still mocks"
        );
        assert!(should_auto_mock("blora", None));
        assert!(
            !should_auto_mock("crewrouter", None),
            "an explicit vendor must not be replaced by mock"
        );
        assert!(!should_auto_mock("", Some("tok")));
        assert!(should_auto_mock("mock", None));
    }

    #[test]
    fn blora_alias_names_resolve() {
        for name in ["blora", "passport", "bloret-passport"] {
            let chain = resolve_provider_chain(false, name, Some("tok"));
            assert_eq!(chain[0].name(), "blora", "alias {name}");
        }
    }
}
