// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

pub mod browser;
pub mod computer;

pub use browser::{BrowserOutput, BrowserRequest, action, validate_url};
pub use computer::{ComputerOutput, ComputerRequest};
