// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Outbound web access and document reading for the Work and Agent modes.
//!
//! - [`search`]: query a configured search backend (SearXNG, Brave, Tavily, Exa).
//! - [`fetch`]: download one page and turn HTML into readable text, refusing
//!   private and loopback addresses so the agent cannot probe the local network.
//! - [`read_document`]: extract text from PDF and Office files.
//!
//! Policy (whether the network may be used at all) is decided by the caller.

mod document;
mod fetch;
mod html;
mod search;

pub use document::{DocumentKind, read_document};
pub use fetch::{FetchOptions, FetchedPage, fetch};
pub use html::html_to_text;
pub use search::{SearchBackend, SearchConfig, SearchHit, format_hits, search};
