// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::sync::Arc;

use blora_model::{
    AnthropicProvider, GeminiProvider, MockProvider, OpenAiProvider, Provider, ResponsesProvider,
};
use blora_runtime::ProviderSource;

/// How the SDK obtains a model provider.
#[derive(Clone)]
pub enum ProviderConfig {
    /// Read `BLORA_*` / API-key environment variables. Explicit opt-in only.
    FromEnvironment,
    /// Use a caller-supplied provider.
    Custom(Arc<dyn Provider>),
    /// Deterministic provider for tests and examples.
    Mock,
    /// Any OpenAI Chat Completions compatible endpoint.
    OpenAiCompatible {
        base_url: String,
        api_key: String,
        model: String,
    },
    Anthropic {
        base_url: String,
        api_key: String,
        model: String,
    },
    Responses {
        base_url: String,
        api_key: String,
        model: String,
    },
    Gemini {
        base_url: String,
        api_key: String,
        model: String,
    },
}

impl ProviderConfig {
    #[must_use]
    pub fn from_env() -> Self {
        Self::FromEnvironment
    }

    #[must_use]
    pub fn custom(provider: Arc<dyn Provider>) -> Self {
        Self::Custom(provider)
    }

    #[must_use]
    pub fn mock() -> Self {
        Self::Mock
    }

    #[must_use]
    pub fn openai_compatible(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self::OpenAiCompatible {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    #[must_use]
    pub fn anthropic(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::Anthropic {
            base_url: "https://api.anthropic.com".to_owned(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    #[must_use]
    pub fn openai(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::openai_compatible("https://api.openai.com/v1", api_key, model)
    }

    #[must_use]
    pub fn responses(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::Responses {
            base_url: "https://api.openai.com/v1".to_owned(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    #[must_use]
    pub fn gemini(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::Gemini {
            base_url: "https://generativelanguage.googleapis.com".to_owned(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    pub(crate) fn into_source(self) -> ProviderSource {
        match self {
            Self::FromEnvironment => ProviderSource::FromEnvironment,
            Self::Custom(provider) => ProviderSource::Explicit(provider),
            Self::Mock => ProviderSource::Explicit(Arc::new(MockProvider::new())),
            Self::OpenAiCompatible {
                base_url,
                api_key,
                model,
            } => ProviderSource::Explicit(Arc::new(OpenAiProvider {
                name: "openai-compatible".to_owned(),
                base_url,
                api_key,
                model,
            })),
            Self::Anthropic {
                base_url,
                api_key,
                model,
            } => ProviderSource::Explicit(Arc::new(AnthropicProvider {
                base_url,
                api_key,
                model,
            })),
            Self::Responses {
                base_url,
                api_key,
                model,
            } => ProviderSource::Explicit(Arc::new(ResponsesProvider {
                base_url,
                api_key,
                model,
            })),
            Self::Gemini {
                base_url,
                api_key,
                model,
            } => ProviderSource::Explicit(Arc::new(GeminiProvider {
                base_url,
                api_key,
                model,
            })),
        }
    }

    pub(crate) fn is_explicit(&self) -> bool {
        !matches!(self, Self::FromEnvironment)
    }

    pub(crate) fn model(&self) -> Option<String> {
        match self {
            Self::OpenAiCompatible { model, .. }
            | Self::Anthropic { model, .. }
            | Self::Responses { model, .. }
            | Self::Gemini { model, .. } => Some(model.clone()),
            Self::FromEnvironment | Self::Custom(_) | Self::Mock => None,
        }
    }
}
