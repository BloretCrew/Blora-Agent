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
    Permission,
    Harness,
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

pub const TRANSITION_DURATION: std::time::Duration = std::time::Duration::from_millis(180);
pub const ANIMATION_FRAME: std::time::Duration = std::time::Duration::from_millis(33);

#[derive(Debug)]
pub struct VisualDialog {
    pub id: DialogId,
    from: f64,
    target: f64,
    started: std::time::Instant,
    pub layer: Option<ratatui::buffer::Buffer>,
    pub theme: Option<crate::theme::Theme>,
}

impl VisualDialog {
    pub fn weight(&self, now: std::time::Instant) -> f64 {
        let t = (now.saturating_duration_since(self.started).as_secs_f64()
            / TRANSITION_DURATION.as_secs_f64())
        .clamp(0.0, 1.0);
        self.from + (self.target - self.from) * t * t * (3.0 - 2.0 * t)
    }

    pub fn live(&self) -> bool {
        self.target == 1.0
    }

    fn retarget(&mut self, target: f64, now: std::time::Instant) {
        if self.target != target {
            self.from = self.weight(now);
            self.target = target;
            self.started = now;
        }
    }

    fn animating(&self, now: std::time::Instant) -> bool {
        self.from != self.target
            && now.saturating_duration_since(self.started) < TRANSITION_DURATION
    }
}

/// Visual lifetimes outlast business state, but never cross session boundaries.
#[derive(Default, Debug)]
pub struct DialogVisuals {
    pub layers: Vec<VisualDialog>,
}

impl DialogVisuals {
    pub fn sync(&mut self, live: &[DialogId], now: std::time::Instant) {
        for layer in &mut self.layers {
            layer.retarget(if live.contains(&layer.id) { 1.0 } else { 0.0 }, now);
        }
        self.layers
            .retain(|layer| layer.live() || layer.weight(now) > 0.0);
        for id in live {
            if !self.layers.iter().any(|layer| layer.id == *id) {
                self.layers.push(VisualDialog {
                    id: *id,
                    from: 0.0,
                    target: 1.0,
                    started: now,
                    layer: None,
                    theme: None,
                });
            }
        }
        // Reorder live slots without moving closing barriers through other windows.
        let mut order = live.iter();
        for index in 0..self.layers.len() {
            if self.layers[index].live() {
                let id = *order.next().expect("live visual slot");
                let other = self.layers.iter().position(|layer| layer.id == id).unwrap();
                self.layers.swap(index, other);
            }
        }
    }

    pub fn active(&self, now: std::time::Instant) -> bool {
        self.layers.iter().any(|layer| layer.animating(now))
    }

    pub fn closing_barrier(&self) -> bool {
        self.layers.last().is_some_and(|layer| !layer.live())
    }

    pub fn clear(&mut self) {
        self.layers.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{DialogId::*, DialogStack};

    #[test]
    fn transition_clock_endpoints_and_idle() {
        use super::*;
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut visuals = DialogVisuals::default();
        assert!(!visuals.active(start));
        visuals.sync(&[Settings], start);
        assert_eq!(visuals.layers[0].weight(start), 0.0);
        assert_eq!(
            visuals.layers[0].weight(start + Duration::from_millis(90)),
            0.5
        );
        assert!(visuals.active(start + Duration::from_millis(179)));
        assert_eq!(visuals.layers[0].weight(start + TRANSITION_DURATION), 1.0);
        assert!(!visuals.active(start + TRANSITION_DURATION));
        assert_eq!(
            visuals.layers[0].weight(start - Duration::from_millis(1)),
            0.0
        );
    }

    #[test]
    fn reversing_and_repeated_sync_preserve_current_weight_and_single_layer() {
        use super::*;
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mid = start + Duration::from_millis(90);
        let mut visuals = DialogVisuals::default();
        visuals.sync(&[Theme], start);
        visuals.sync(&[Theme], mid);
        assert_eq!(visuals.layers[0].weight(mid), 0.5);
        visuals.sync(&[], mid);
        assert!(visuals.closing_barrier());
        assert_eq!(visuals.layers[0].weight(mid), 0.5);
        let reverse = mid + Duration::from_millis(90);
        assert_eq!(visuals.layers[0].weight(reverse), 0.25);
        visuals.sync(&[Theme], reverse);
        assert_eq!(visuals.layers.len(), 1);
        assert_eq!(visuals.layers[0].weight(reverse), 0.25);
        assert!(!visuals.closing_barrier());
        assert_eq!(visuals.layers[0].weight(reverse + TRANSITION_DURATION), 1.0);
    }

    #[test]
    fn closing_layers_keep_order_cache_and_barrier_until_completion() {
        use super::*;
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let settled = start + TRANSITION_DURATION;
        let mut visuals = DialogVisuals::default();
        visuals.sync(&[Settings, Provider, AddProvider], start);
        visuals.layers[2].layer = Some(ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(
            1, 1, 2, 2,
        )));
        visuals.sync(&[Settings, Provider], settled);
        assert!(visuals.closing_barrier());
        assert!(visuals.layers[2].layer.is_some());
        assert_eq!(
            visuals
                .layers
                .iter()
                .map(|l| l.weight(settled + Duration::from_millis(90)))
                .sum::<f64>(),
            2.5
        );
        visuals.sync(&[Settings, Provider], settled + TRANSITION_DURATION);
        assert_eq!(visuals.layers.len(), 2);
        assert!(!visuals.closing_barrier());
        assert!(!visuals.active(settled + TRANSITION_DURATION));
        visuals.sync(&[Provider, Settings], settled + TRANSITION_DURATION);
        assert_eq!(visuals.layers[1].id, Settings);
        visuals.clear();
        assert!(visuals.layers.is_empty());
    }

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
