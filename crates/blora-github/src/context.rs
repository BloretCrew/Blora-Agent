// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

/// A line comment on a pull request diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewAnchor {
    pub path: String,
    pub line: Option<i64>,
    pub diff: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueContext {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub author: String,
    pub state: String,
    pub created_at: String,
    pub comments: Vec<Note>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewSummary {
    pub author: String,
    pub state: String,
    pub body: String,
    pub submitted_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullContext {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub author: String,
    pub state: String,
    pub created_at: String,
    pub base_ref: String,
    pub head_ref: String,
    pub base_repo: String,
    pub head_repo: String,
    pub commits: u64,
    pub files: Vec<FileChange>,
    pub comments: Vec<Note>,
    pub review_comments: Vec<ReviewAnchorNote>,
    pub reviews: Vec<ReviewSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewAnchorNote {
    pub id: u64,
    pub author: String,
    pub body: String,
    pub path: String,
    pub line: Option<i64>,
    pub created_at: String,
}

/// Where a reaction or a failure comment should be attached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReactionSite {
    IssueComment { id: u64 },
    ReviewComment { id: u64 },
    Issue { number: u64 },
}
