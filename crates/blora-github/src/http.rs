// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::io::Read;
use std::time::Duration;

use ureq::OrAnyStatus;

use crate::error::{GithubError, Result};

pub struct HttpRequest<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub token: &'a str,
    pub body: Option<&'a [u8]>,
}

pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait Http {
    fn send(&self, request: &HttpRequest<'_>) -> Result<HttpResponse>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UreqHttp;

impl Http for UreqHttp {
    fn send(&self, request: &HttpRequest<'_>) -> Result<HttpResponse> {
        let mut call = ureq::request(request.method, request.url)
            .set("User-Agent", "blora-agent")
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
            .timeout(Duration::from_secs(60));
        if !request.token.is_empty() {
            let header = format!("Bearer {}", request.token);
            call = call.set("Authorization", &header);
        }
        let response = if let Some(body) = request.body {
            call.set("Content-Type", "application/json")
                .send_bytes(body)
                .or_any_status()
                .map_err(|err| GithubError::new(format!("访问 GitHub 失败：{err}")))?
        } else {
            call.call()
                .or_any_status()
                .map_err(|err| GithubError::new(format!("访问 GitHub 失败：{err}")))?
        };
        let status = response.status();
        let mut body = Vec::new();
        response
            .into_reader()
            .take(8 * 1024 * 1024)
            .read_to_end(&mut body)
            .map_err(|err| GithubError::new(format!("读取 GitHub 响应失败：{err}")))?;
        Ok(HttpResponse { status, body })
    }
}
