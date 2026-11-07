// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cooperative cancellation flag. Cloning shares the same flag.
///
/// [`CancelToken::child`] creates a token that is cancelled either on its own
/// [`cancel`](Self::cancel) or when any ancestor is cancelled. Cancelling a
/// child does not cancel its parent.
#[derive(Clone, Debug)]
pub struct CancelToken {
    inner: Arc<AtomicBool>,
    parent: Option<Box<CancelToken>>,
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancelToken {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicBool::new(false)),
            parent: None,
        }
    }

    /// A token that also observes this one. Used so a parent run can stop every
    /// in-flight subagent without sharing a single flag (which would make
    /// cancelling one child stop the parent).
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            inner: Arc::new(AtomicBool::new(false)),
            parent: Some(Box::new(self.clone())),
        }
    }

    pub fn cancel(&self) {
        self.inner.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inner.load(Ordering::SeqCst)
            || self
                .parent
                .as_ref()
                .is_some_and(|parent| parent.is_cancelled())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_observes_parent_but_not_the_reverse() {
        let parent = CancelToken::new();
        let child = parent.child();
        let grandchild = child.child();
        assert!(!grandchild.is_cancelled());
        parent.cancel();
        assert!(child.is_cancelled());
        assert!(grandchild.is_cancelled());

        let parent = CancelToken::new();
        let child = parent.child();
        child.cancel();
        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());
        assert!(!parent.child().is_cancelled());
    }
}
