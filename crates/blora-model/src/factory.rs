// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use crate::{AnthropicProvider, MockProvider, OpenAiProvider, Provider, ResponsesProvider};

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
    if force_mock {
        return vec![Box::new(MockProvider::new())];
    }
    let spec = if name.is_empty() {
        std::env::var("BLORA_PROVIDER").unwrap_or_else(|_| "openai".to_owned())
    } else {
        name.to_owned()
    };
    let mut out = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        out.push(one(part));
    }
    if out.is_empty() {
        out.push(Box::new(MockProvider::new()));
    }
    out
}

fn one(kind: &str) -> Box<dyn Provider> {
    match kind.to_ascii_lowercase().as_str() {
        "mock" => Box::new(MockProvider::new()),
        "anthropic" | "claude" => AnthropicProvider::from_env()
            .map(|provider| Box::new(provider) as Box<dyn Provider>)
            .unwrap_or_else(|_| Box::new(MockProvider::new())),
        "responses" => ResponsesProvider::from_env()
            .map(|provider| Box::new(provider) as Box<dyn Provider>)
            .unwrap_or_else(|_| Box::new(MockProvider::new())),
        _ => OpenAiProvider::from_env()
            .map(|provider| Box::new(provider) as Box<dyn Provider>)
            .unwrap_or_else(|_| Box::new(MockProvider::new())),
    }
}

struct FallbackProvider {
    inner: Vec<Box<dyn Provider>>,
}

impl Provider for FallbackProvider {
    fn name(&self) -> &'static str {
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
}
