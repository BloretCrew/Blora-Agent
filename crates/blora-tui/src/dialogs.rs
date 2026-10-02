// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogId {
    Project,
    Folder,
    Session,
    Mode,
    Git,
    ToolDetail,
    Tool,
    Passport,
    AddProvider,
    Theme,
    Provider,
    Context,
    Settings,
    Onboarding,
}

/// Dialog order from bottom to top, with one entry per live dialog.
#[derive(Default, Debug)]
pub struct DialogStack {
    order: Vec<DialogId>,
}

impl DialogStack {
    pub fn activate(&mut self, id: DialogId) {
        self.order.retain(|entry| *entry != id);
        self.order.push(id);
    }

    pub fn sync(&mut self, live: &[DialogId]) {
        self.order.retain(|entry| live.contains(entry));
        for id in live {
            if !self.order.contains(id) {
                self.order.push(*id);
            }
        }
    }

    pub fn top(&self) -> Option<DialogId> {
        self.order.last().copied()
    }

    pub fn as_slice(&self) -> &[DialogId] {
        &self.order
    }
}

#[cfg(test)]
mod tests {
    use super::{DialogId::*, DialogStack};

    #[test]
    fn repeated_activation_moves_to_top_without_duplicates() {
        let mut stack = DialogStack::default();
        stack.activate(Provider);
        stack.activate(Theme);
        stack.activate(Provider);
        assert_eq!(stack.as_slice(), &[Theme, Provider]);
    }

    #[test]
    fn closing_child_restores_parent_and_sync_preserves_order() {
        let mut stack = DialogStack::default();
        stack.activate(Settings);
        stack.activate(Provider);
        stack.activate(AddProvider);
        stack.sync(&[Settings, AddProvider, Provider]);
        assert_eq!(stack.top(), Some(AddProvider));
        stack.sync(&[Settings, Provider]);
        assert_eq!(stack.top(), Some(Provider));
        stack.sync(&[Settings]);
        assert_eq!(stack.top(), Some(Settings));
        stack.sync(&[]);
        assert_eq!(stack.top(), None);
    }

    #[test]
    fn newly_live_dialogs_are_registered_once() {
        let mut stack = DialogStack::default();
        stack.sync(&[Project, Folder]);
        stack.sync(&[Project, Folder]);
        assert_eq!(stack.as_slice(), &[Project, Folder]);
    }
}
