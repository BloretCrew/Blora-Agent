// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Desktop automation through platform command-line tools.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use blora_types::{BloraError, Result};

#[derive(Clone, Debug, Default)]
pub struct ComputerRequest {
    pub action: String,
    pub x: Option<i64>,
    pub y: Option<i64>,
    pub text: Option<String>,
    pub key: Option<String>,
    pub amount: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct ComputerOutput {
    pub text: String,
    pub image: Option<Vec<u8>>,
}

const FAKE_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x62, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

pub fn action(workspace: &Path, request: &ComputerRequest) -> Result<ComputerOutput> {
    let fake = std::env::var("BLORA_COMPUTER_BACKEND").as_deref() == Ok("fake");
    if !fake && std::env::var_os("BLORA_COMPUTER_USE").is_none() {
        return Err(BloraError::Policy(
            "computer use is disabled; set BLORA_COMPUTER_USE=1".to_owned(),
        ));
    }
    action_with(workspace, request, fake)
}

fn action_with(_workspace: &Path, request: &ComputerRequest, fake: bool) -> Result<ComputerOutput> {
    match request.action.as_str() {
        "screenshot" => {
            let bytes = if fake {
                FAKE_PNG.to_vec()
            } else {
                screenshot()?
            };
            Ok(ComputerOutput {
                text: format!("screenshot {} bytes", bytes.len()),
                image: Some(bytes),
            })
        }
        "click" => {
            let (x, y) = point(request)?;
            if fake {
                return Ok(ComputerOutput {
                    text: format!("clicked {x},{y}"),
                    image: None,
                });
            }
            xdotool(&["mousemove", &x.to_string(), &y.to_string()])?;
            xdotool(&["click", "1"])?;
            Ok(ComputerOutput {
                text: format!("clicked {x},{y}"),
                image: None,
            })
        }
        "move" => {
            let (x, y) = point(request)?;
            if !fake {
                xdotool(&["mousemove", &x.to_string(), &y.to_string()])?;
            }
            Ok(ComputerOutput {
                text: format!("moved to {x},{y}"),
                image: None,
            })
        }
        "type" => {
            let text = request.text.as_deref().unwrap_or_default();
            if !fake {
                xdotool(&["type", "--clearmodifiers", "--", text])?;
            }
            Ok(ComputerOutput {
                text: format!("typed {} characters", text.len()),
                image: None,
            })
        }
        "key" => {
            let key = request.key.as_deref().unwrap_or("Return");
            if !fake {
                xdotool(&["key", key])?;
            }
            Ok(ComputerOutput {
                text: format!("pressed {key}"),
                image: None,
            })
        }
        "scroll" => {
            let amount = request.amount.unwrap_or(3);
            let button = if amount >= 0 { "5" } else { "4" };
            if !fake {
                for _ in 0..amount.unsigned_abs() {
                    xdotool(&["click", button])?;
                }
            }
            Ok(ComputerOutput {
                text: format!("scrolled {amount}"),
                image: None,
            })
        }
        "wait" => {
            let millis = request.amount.unwrap_or(500).clamp(0, 30_000);
            std::thread::sleep(Duration::from_millis(millis as u64));
            Ok(ComputerOutput {
                text: format!("waited {millis} ms"),
                image: None,
            })
        }
        other => Err(BloraError::Other(format!(
            "unknown computer action: {other}"
        ))),
    }
}

fn point(request: &ComputerRequest) -> Result<(i64, i64)> {
    let x = request
        .x
        .ok_or_else(|| BloraError::Other("computer action needs x".to_owned()))?;
    let y = request
        .y
        .ok_or_else(|| BloraError::Other("computer action needs y".to_owned()))?;
    if x < 0 || y < 0 {
        return Err(BloraError::Other(
            "computer coordinates must be non-negative".to_owned(),
        ));
    }
    Ok((x, y))
}

fn screenshot() -> Result<Vec<u8>> {
    #[cfg(target_os = "linux")]
    {
        for (program, args) in [
            ("grim", vec!["-"]),
            ("scrot", vec!["-o", "-"]),
            ("import", vec!["-window", "root", "png:-"]),
        ] {
            if let Ok(output) = Command::new(program).args(&args).output() {
                if output.status.success() && !output.stdout.is_empty() {
                    return Ok(output.stdout);
                }
            }
        }
        Err(BloraError::Exec(
            "screenshot needs grim, scrot, or ImageMagick import".to_owned(),
        ))
    }
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("screencapture")
            .args(["-x", "-t", "png", "-"])
            .output()
            .map_err(BloraError::exec)?;
        if output.status.success() {
            return Ok(output.stdout);
        }
        Err(BloraError::Exec("screencapture failed".to_owned()))
    }
    #[cfg(target_os = "windows")]
    {
        Err(BloraError::Exec(
            "computer screenshot is not implemented on Windows".to_owned(),
        ))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err(BloraError::Exec(
            "computer screenshot is not supported on this platform".to_owned(),
        ))
    }
}

#[cfg(target_os = "linux")]
fn xdotool(args: &[&str]) -> Result<()> {
    let output = Command::new("xdotool")
        .args(args)
        .output()
        .map_err(|err| BloraError::Exec(format!("xdotool: {err}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(BloraError::Exec(format!(
            "xdotool failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

#[cfg(not(target_os = "linux"))]
fn xdotool(_args: &[&str]) -> Result<()> {
    Err(BloraError::Exec(
        "computer input is only implemented on Linux/X11".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_backend_handles_actions() {
        let output = action_with(
            Path::new("/tmp"),
            &ComputerRequest {
                action: "click".to_owned(),
                x: Some(10),
                y: Some(20),
                ..ComputerRequest::default()
            },
            true,
        )
        .unwrap();
        assert!(output.text.contains("10,20"));
        let screenshot = action_with(
            Path::new("/tmp"),
            &ComputerRequest {
                action: "screenshot".to_owned(),
                ..ComputerRequest::default()
            },
            true,
        )
        .unwrap();
        assert!(screenshot.image.is_some());
    }
}
