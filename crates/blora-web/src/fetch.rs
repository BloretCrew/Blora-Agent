// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Fetch one web page with SSRF protection.

use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use blora_types::{BloraError, Result};

/// Limits for [`fetch`].
#[derive(Clone, Debug)]
pub struct FetchOptions {
    /// Largest response body read, in bytes.
    pub max_bytes: usize,
    pub timeout: Duration,
    /// Permit loopback and private addresses. Only tests and explicit
    /// operator opt-in (`BLORA_WEB_ALLOW_PRIVATE=1`) set this.
    pub allow_private: bool,
}

impl Default for FetchOptions {
    fn default() -> Self {
        Self {
            max_bytes: 2_000_000,
            timeout: Duration::from_secs(20),
            allow_private: std::env::var("BLORA_WEB_ALLOW_PRIVATE").as_deref() == Ok("1"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FetchedPage {
    /// URL after redirects.
    pub url: String,
    pub content_type: String,
    /// Readable text: HTML converted, JSON and plain text passed through.
    pub text: String,
    pub truncated: bool,
}

/// Resolver that drops non-public addresses, so redirects and DNS rebinding
/// cannot reach the local network.
struct PublicOnly {
    allow_private: bool,
}

impl ureq::Resolver for PublicOnly {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
        let addrs: Vec<SocketAddr> = netloc.to_socket_addrs()?.collect();
        if self.allow_private {
            return Ok(addrs);
        }
        let public: Vec<SocketAddr> = addrs
            .into_iter()
            .filter(|addr| is_public(addr.ip()))
            .collect();
        if public.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("{netloc} resolves only to private or local addresses"),
            ));
        }
        Ok(public)
    }
}

/// True for globally routable unicast addresses.
#[must_use]
pub(crate) fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (b == 18 || b == 19))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
        }
    }
}

/// Download `url` and return readable text.
pub fn fetch(url: &str, options: &FetchOptions) -> Result<FetchedPage> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(BloraError::Other(
            "web_fetch only supports http:// and https:// URLs".to_owned(),
        ));
    }
    let agent = ureq::AgentBuilder::new()
        .resolver(PublicOnly {
            allow_private: options.allow_private,
        })
        .timeout(options.timeout)
        .redirects(5)
        .user_agent(concat!("BloraAgent/", env!("CARGO_PKG_VERSION")))
        .build();
    let response = agent
        .get(url)
        .set(
            "Accept",
            "text/html,application/xhtml+xml,text/plain,application/json;q=0.9,*/*;q=0.5",
        )
        .call()
        .map_err(|err| match err {
            ureq::Error::Status(code, _) => BloraError::Other(format!("HTTP {code} for {url}")),
            ureq::Error::Transport(transport) => {
                BloraError::Other(format!("could not fetch {url}: {transport}"))
            }
        })?;
    let final_url = response.get_url().to_owned();
    let content_type = response
        .header("content-type")
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(options.max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| BloraError::Other(format!("reading {url}: {err}")))?;
    let truncated = bytes.len() > options.max_bytes;
    bytes.truncate(options.max_bytes);
    if !(content_type.is_empty()
        || content_type.contains("text/")
        || content_type.contains("json")
        || content_type.contains("xml"))
    {
        return Ok(FetchedPage {
            url: final_url,
            content_type: content_type.clone(),
            text: format!(
                "[binary content: {content_type}, {} bytes; download it with shell if the user needs the file]",
                bytes.len()
            ),
            truncated,
        });
    }
    let raw = String::from_utf8_lossy(&bytes).into_owned();
    let looks_html = content_type.contains("html")
        || (content_type.is_empty() && raw.trim_start().starts_with('<'));
    let text = if looks_html {
        crate::html::html_to_text(&raw)
    } else {
        raw
    };
    Ok(FetchedPage {
        url: final_url,
        content_type,
        text,
        truncated,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    /// Serve canned responses on 127.0.0.1; `handler` maps the request line to a response.
    pub(crate) fn serve(count: usize, handler: impl Fn(&str) -> String + Send + 'static) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(count) {
                let mut stream = stream.unwrap();
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                let _ = stream.write_all(handler(&request).as_bytes());
            }
        });
        format!("http://{addr}")
    }

    pub(crate) fn http(status: &str, content_type: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn local() -> FetchOptions {
        FetchOptions {
            allow_private: true,
            ..FetchOptions::default()
        }
    }

    #[test]
    fn classifies_public_addresses() {
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "0.0.0.0",
        ] {
            assert!(!is_public(private.parse().unwrap()), "{private}");
        }
        for public in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(is_public(public.parse().unwrap()), "{public}");
        }
    }

    #[test]
    fn refuses_loopback_by_default() {
        let base = serve(1, |_| http("200 OK", "text/plain", "secret"));
        let err = fetch(
            &base,
            &FetchOptions {
                allow_private: false,
                ..FetchOptions::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("private or local"), "{err}");
    }

    #[test]
    fn converts_html_and_reports_truncation() {
        let base = serve(2, |_| {
            http(
                "200 OK",
                "text/html; charset=utf-8",
                "<title>T</title><p>Hello <b>world</b></p>",
            )
        });
        let page = fetch(&base, &local()).unwrap();
        assert!(page.text.contains("# T") && page.text.contains("Hello world"));
        assert!(!page.truncated);
        let small = FetchOptions {
            max_bytes: 8,
            ..local()
        };
        assert!(fetch(&base, &small).unwrap().truncated);
    }

    #[test]
    fn rejects_other_schemes_and_reports_status() {
        assert!(fetch("file:///etc/passwd", &local()).is_err());
        let base = serve(1, |_| http("404 Not Found", "text/plain", "no"));
        assert!(
            fetch(&base, &local())
                .unwrap_err()
                .to_string()
                .contains("404")
        );
    }
}
