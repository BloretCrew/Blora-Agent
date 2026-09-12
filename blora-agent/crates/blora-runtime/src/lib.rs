// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! In-process runtime. TUI and Web will share this crate.

mod cancel;
mod coordinator;

pub use cancel::CancelToken;
pub use coordinator::{MockRunOptions, Runtime};
