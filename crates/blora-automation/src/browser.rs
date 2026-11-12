// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Browser automation through the Chrome DevTools Protocol.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use blora_types::{BloraError, Result};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

#[derive(Clone, Debug, Default)]
pub struct BrowserRequest {
    pub action: String,
    pub url: Option<String>,
    pub selector: Option<String>,
    pub reference: Option<String>,
    pub text: Option<String>,
    pub key: Option<String>,
    pub x: Option<i64>,
    pub y: Option<i64>,
    pub full_page: bool,
    pub expression: Option<String>,
    pub timeout_secs: Option<u64>,
    pub accept: Option<bool>,
    pub prompt_text: Option<String>,
    pub method: Option<String>,
    pub params: Option<Value>,
}

#[derive(Clone, Debug, Default)]
pub struct BrowserOutput {
    pub text: String,
    pub image: Option<Vec<u8>>,
}

struct Browser {
    child: Child,
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: u64,
    session_id: Option<String>,
    user_data: PathBuf,
    dialogs: Vec<Value>,
}

fn browser() -> &'static Mutex<Option<Browser>> {
    static BROWSER: OnceLock<Mutex<Option<Browser>>> = OnceLock::new();
    BROWSER.get_or_init(|| Mutex::new(None))
}

/// Run one browser action. The caller is responsible for URL policy checks.
pub fn action(workspace: &Path, request: &BrowserRequest) -> Result<BrowserOutput> {
    let mut guard = browser()
        .lock()
        .map_err(|_| BloraError::Other("browser lock poisoned".to_owned()))?;
    if request.action == "close" {
        if let Some(mut browser) = guard.take() {
            browser.shutdown();
        }
        return Ok(BrowserOutput {
            text: "browser closed".to_owned(),
            image: None,
        });
    }
    if guard.is_none() {
        *guard = Some(Browser::launch(workspace)?);
    }
    let browser = guard.as_mut().expect("browser initialized");
    browser.dispatch(request)
}

impl Browser {
    fn launch(workspace: &Path) -> Result<Self> {
        let executable = find_browser().ok_or_else(|| {
            BloraError::Other(
                "no Chrome/Chromium found; set BLORA_BROWSER to the executable".to_owned(),
            )
        })?;
        let user_data = workspace.join(".blora/browser/profile");
        std::fs::create_dir_all(&user_data).map_err(BloraError::exec)?;
        let mut command = Command::new(&executable);
        command
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", user_data.display()))
            .arg("--window-size=1280,900")
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if std::env::var("BLORA_BROWSER_NO_SANDBOX").is_ok() {
            command.arg("--no-sandbox");
        }
        let child = command
            .spawn()
            .map_err(|err| BloraError::Exec(format!("start browser: {err}")))?;
        let (port, path) = read_devtools_port(&user_data, Duration::from_secs(15))?;
        let url = format!("ws://127.0.0.1:{port}{path}");
        let (socket, _) = tungstenite::connect(url)
            .map_err(|err| BloraError::Exec(format!("browser websocket: {err}")))?;
        let mut browser = Self {
            child,
            socket,
            next_id: 1,
            session_id: None,
            user_data,
            dialogs: Vec::new(),
        };
        browser.command("Page.enable", json!({}))?;
        browser.command("Runtime.enable", json!({}))?;
        browser.command(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source": console_capture_script()}),
        )?;
        browser.command(
            "Emulation.setDeviceMetricsOverride",
            json!({"width": 1280, "height": 900, "deviceScaleFactor": 1, "mobile": false}),
        )?;
        let target = browser.command("Target.createTarget", json!({"url": "about:blank"}))?;
        let target_id = target
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or_else(|| BloraError::Other("browser target missing".to_owned()))?;
        let attached = browser.command(
            "Target.attachToTarget",
            json!({"targetId": target_id, "flatten": true}),
        )?;
        browser.session_id = attached
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(browser)
    }

    fn dispatch(&mut self, request: &BrowserRequest) -> Result<BrowserOutput> {
        match request.action.as_str() {
            "open" | "navigate" => {
                let url = request
                    .url
                    .as_deref()
                    .ok_or_else(|| BloraError::Other("browser needs url".to_owned()))?;
                self.command("Page.navigate", json!({"url": url}))?;
                self.wait_ready(request.timeout_secs.unwrap_or(15))?;
                Ok(BrowserOutput {
                    text: self.page_info()?,
                    image: None,
                })
            }
            "snapshot" => Ok(BrowserOutput {
                text: self.evaluate(snapshot_script())?,
                image: None,
            }),
            "click" => {
                let selector = self.selector(request)?;
                let script = format!(
                    "(() => {{ const el = document.querySelector({selector}); if (!el) return 'not found'; el.scrollIntoView({{block:'center'}}); el.click(); return 'clicked'; }})()"
                );
                Ok(BrowserOutput {
                    text: self.evaluate(&script)?,
                    image: None,
                })
            }
            "type" => {
                let selector = self.selector(request)?;
                let text = serde_json::to_string(request.text.as_deref().unwrap_or_default())
                    .unwrap_or_else(|_| "\"\"".to_owned());
                let script = format!(
                    "(() => {{ const el = document.querySelector({selector}); if (!el) return 'not found'; el.focus(); const value = {text}; if ('value' in el) {{ const setter = Object.getOwnPropertyDescriptor(el.constructor.prototype, 'value')?.set; setter ? setter.call(el, value) : (el.value = value); }} else {{ el.innerText = value; }} el.dispatchEvent(new Event('input', {{bubbles:true}})); el.dispatchEvent(new Event('change', {{bubbles:true}})); return 'typed'; }})()"
                );
                Ok(BrowserOutput {
                    text: self.evaluate(&script)?,
                    image: None,
                })
            }
            "press" => {
                let key = serde_json::to_string(request.key.as_deref().unwrap_or("Enter"))
                    .unwrap_or_else(|_| "\"Enter\"".to_owned());
                let script = format!(
                    "(() => {{ const el = document.activeElement || document.body; el.dispatchEvent(new KeyboardEvent('keydown', {{key: {key}, bubbles:true}})); el.dispatchEvent(new KeyboardEvent('keyup', {{key: {key}, bubbles:true}})); return 'pressed'; }})()"
                );
                Ok(BrowserOutput {
                    text: self.evaluate(&script)?,
                    image: None,
                })
            }
            "scroll" => {
                let x = request.x.unwrap_or(0);
                let y = request.y.unwrap_or(600);
                Ok(BrowserOutput {
                    text: self.evaluate(&format!(
                        "(() => {{ window.scrollBy({x}, {y}); return 'scrolled'; }})()"
                    ))?,
                    image: None,
                })
            }
            "screenshot" => {
                let result = self.command(
                    "Page.captureScreenshot",
                    json!({"format": "png", "captureBeyondViewport": request.full_page}),
                )?;
                let data = result
                    .get("data")
                    .and_then(Value::as_str)
                    .ok_or_else(|| BloraError::Other("screenshot data missing".to_owned()))?;
                let bytes =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)
                        .map_err(|err| BloraError::Other(format!("screenshot base64: {err}")))?;
                Ok(BrowserOutput {
                    text: format!("screenshot {} bytes", bytes.len()),
                    image: Some(bytes),
                })
            }
            "evaluate" => {
                let expression = request.expression.as_deref().ok_or_else(|| {
                    BloraError::Other("browser evaluate needs expression".to_owned())
                })?;
                Ok(BrowserOutput {
                    text: self.evaluate(expression)?,
                    image: None,
                })
            }
            "wait_for" => {
                let selector = request
                    .selector
                    .clone()
                    .or_else(|| {
                        request
                            .reference
                            .as_ref()
                            .map(|reference| format!("[data-blora-ref=\"{reference}\"]"))
                    })
                    .ok_or_else(|| {
                        BloraError::Other("browser wait_for needs selector".to_owned())
                    })?;
                let timeout = Duration::from_secs(request.timeout_secs.unwrap_or(10));
                let deadline = Instant::now() + timeout;
                loop {
                    let found = self.evaluate(&format!(
                        "Boolean(document.querySelector({}))",
                        serde_json::to_string(&selector).unwrap_or_else(|_| "\"\"".to_owned())
                    ))?;
                    if found == "true" {
                        return Ok(BrowserOutput {
                            text: format!("found {selector}"),
                            image: None,
                        });
                    }
                    if Instant::now() >= deadline {
                        return Ok(BrowserOutput {
                            text: format!("timeout waiting for {selector}"),
                            image: None,
                        });
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            "back" | "forward" => {
                let direction = if request.action == "back" {
                    "back"
                } else {
                    "forward"
                };
                self.evaluate(&format!("history.{direction}(); 'ok'"))?;
                self.wait_ready(request.timeout_secs.unwrap_or(10))?;
                Ok(BrowserOutput {
                    text: self.page_info()?,
                    image: None,
                })
            }
            "console" => Ok(BrowserOutput {
                text: self.evaluate("JSON.stringify(window.__bloraConsole || [])")?,
                image: None,
            }),
            "dialog" => {
                if self.dialogs.is_empty() {
                    return Ok(BrowserOutput {
                        text: "no open dialog".to_owned(),
                        image: None,
                    });
                }
                let dialog = self.dialogs.remove(0);
                let accept = request.accept.unwrap_or(true);
                self.command(
                    "Page.handleJavaScriptDialog",
                    json!({"accept": accept, "promptText": request.prompt_text}),
                )?;
                Ok(BrowserOutput {
                    text: dialog.to_string(),
                    image: None,
                })
            }
            "cdp" => {
                let method = request
                    .method
                    .as_deref()
                    .ok_or_else(|| BloraError::Other("browser cdp needs method".to_owned()))?;
                let params = request.params.clone().unwrap_or_else(|| json!({}));
                let result = self.command(method, params)?;
                Ok(BrowserOutput {
                    text: result.to_string(),
                    image: None,
                })
            }
            "list" => Ok(BrowserOutput {
                text: self.page_info()?,
                image: None,
            }),
            other => Err(BloraError::Other(format!(
                "unknown browser action: {other}"
            ))),
        }
    }

    fn selector(&self, request: &BrowserRequest) -> Result<String> {
        if let Some(selector) = &request.selector {
            return Ok(serde_json::to_string(selector).unwrap_or_else(|_| "\"body\"".to_owned()));
        }
        if let Some(reference) = &request.reference {
            return Ok(
                serde_json::to_string(&format!("[data-blora-ref=\"{reference}\"]"))
                    .unwrap_or_else(|_| "\"body\"".to_owned()),
            );
        }
        Err(BloraError::Other(
            "browser action needs selector or ref".to_owned(),
        ))
    }

    fn page_info(&mut self) -> Result<String> {
        self.evaluate(
            "JSON.stringify({title: document.title, url: location.href, text: (document.body?.innerText || '').slice(0, 8000)})",
        )
    }

    fn wait_ready(&mut self, timeout_secs: u64) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(timeout_secs.clamp(1, 120));
        loop {
            let state = self.evaluate("document.readyState")?;
            if state == "complete" || state == "interactive" {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn evaluate(&mut self, expression: &str) -> Result<String> {
        let result = self.command(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": true,
                "userGesture": true,
            }),
        )?;
        let value = result
            .get("result")
            .and_then(|result| result.get("value"))
            .cloned()
            .unwrap_or(Value::Null);
        Ok(match value {
            Value::String(text) => text,
            other => other.to_string(),
        })
    }

    fn command(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let mut payload = json!({"id": id, "method": method, "params": params});
        if let Some(session) = &self.session_id {
            payload["sessionId"] = json!(session);
        }
        self.socket
            .send(Message::Text(payload.to_string().into()))
            .map_err(|err| BloraError::Exec(format!("browser send: {err}")))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if Instant::now() >= deadline {
                return Err(BloraError::Other(format!("browser {method} timed out")));
            }
            let message = self
                .socket
                .read()
                .map_err(|err| BloraError::Exec(format!("browser read: {err}")))?;
            let Message::Text(text) = message else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                continue;
            };
            if value.get("method").and_then(Value::as_str) == Some("Page.javascriptDialogOpening") {
                self.dialogs.push(value);
                continue;
            }
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = value.get("error") {
                    return Err(BloraError::Other(format!("browser {method}: {error}")));
                }
                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    fn shutdown(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.user_data);
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn read_devtools_port(user_data: &Path, timeout: Duration) -> Result<(u16, String)> {
    let path = user_data.join("DevToolsActivePort");
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                if let Ok(port) = port.trim().parse::<u16>() {
                    return Ok((port, path.trim().to_owned()));
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(BloraError::Other(
                "browser did not expose DevToolsActivePort".to_owned(),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn find_browser() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("BLORA_BROWSER") {
        return Some(PathBuf::from(path));
    }
    for name in [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "microsoft-edge",
        "msedge",
    ] {
        if let Some(path) = find_in_path(name) {
            return Some(path);
        }
    }
    None
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn console_capture_script() -> &'static str {
    "(() => {\
      if (window.__bloraConsole) return;\
      window.__bloraConsole = [];\
      for (const level of ['log','info','warn','error','debug']) {\
        const original = console[level];\
        console[level] = (...args) => {\
          window.__bloraConsole.push({level, text: args.map(a => { try { return typeof a === 'string' ? a : JSON.stringify(a); } catch { return String(a); } }).join(' '), time: Date.now()});\
          if (window.__bloraConsole.length > 200) window.__bloraConsole.shift();\
          original.apply(console, args);\
        };\
      }\
    })()"
}

fn snapshot_script() -> &'static str {
    "(() => {\
      document.querySelectorAll('[data-blora-ref]').forEach(el => el.removeAttribute('data-blora-ref'));\
      const nodes = [...document.querySelectorAll('a,button,input,textarea,select,[role=button]')].slice(0, 300);\
      const items = nodes.map((el, index) => {\
        el.setAttribute('data-blora-ref', String(index));\
        return {ref: index, tag: el.tagName.toLowerCase(), text: (el.innerText || el.value || el.getAttribute('aria-label') || '').trim().slice(0, 160), id: el.id || null, name: el.name || null, type: el.type || null, href: el.href || null};\
      });\
      return JSON.stringify({title: document.title, url: location.href, text: (document.body?.innerText || '').slice(0, 8000), items});\
    })()"
}

/// Validate a browser URL against scheme, private-network, and allowlist rules.
pub fn validate_url(raw: &str) -> Result<url::Url> {
    let url =
        url::Url::parse(raw).map_err(|err| BloraError::Other(format!("invalid URL: {err}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(BloraError::Policy(
            "browser only opens http(s) URLs".to_owned(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| BloraError::Other("URL has no host".to_owned()))?;
    let allow_private = std::env::var("BLORA_BROWSER_ALLOW_PRIVATE").is_ok();
    if !allow_private && private_host(host) {
        return Err(BloraError::Policy(format!(
            "browser refuses private host: {host}"
        )));
    }
    if let Ok(allowlist) = std::env::var("BLORA_BROWSER_ALLOWED_DOMAINS") {
        let allowed = allowlist
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .any(|entry| host == entry || host.ends_with(&format!(".{entry}")));
        if !allowed {
            return Err(BloraError::Policy(format!(
                "browser domain not allowed: {host}"
            )));
        }
    }
    Ok(url)
}

fn private_host(host: &str) -> bool {
    if matches!(host, "localhost" | "::1" | "[::1]") {
        return true;
    }
    let Ok(ip) = host.parse::<std::net::IpAddr>() else {
        return false;
    };
    match ip {
        std::net::IpAddr::V4(ip) => {
            ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
        }
        std::net::IpAddr::V6(ip) => ip.is_loopback() || ip.is_unspecified(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_private_and_non_http_urls() {
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("http://127.0.0.1:8080").is_err());
        assert!(validate_url("http://localhost").is_err());
        assert!(validate_url("https://example.com").is_ok());
    }
}
