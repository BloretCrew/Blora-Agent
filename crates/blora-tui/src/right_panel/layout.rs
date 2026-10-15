// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use unicode_width::UnicodeWidthChar;

pub(super) fn clip(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out
}
pub(super) fn tabs(inner: Rect, count: usize) -> Vec<Rect> {
    let available = inner.width.saturating_sub(2);
    (0..count)
        .map(|i| {
            let left = available as usize * i / count;
            let right = available as usize * (i + 1) / count;
            Rect::new(inner.x + left as u16, inner.y, (right - left) as u16, 1)
        })
        .collect()
}
pub(super) fn body(inner: Rect) -> Rect {
    Rect::new(
        inner.x,
        inner.y.saturating_add(2),
        inner.width,
        inner.height.saturating_sub(3),
    )
}
pub(super) fn hovered(rect: Rect, pointer: Option<(u16, u16)>) -> bool {
    pointer.is_some_and(|p| rect.contains(p.into()))
}
