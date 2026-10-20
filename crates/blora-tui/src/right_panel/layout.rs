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
    if count == 0 || inner.width == 0 {
        return Vec::new();
    }
    // Reserve a dedicated final cell for the collapse control. Divide the
    // remaining width evenly so no tab's close hitbox overlaps its neighbor.
    let available = inner.width.saturating_sub(2);
    let width = (available / count as u16).max(1);
    (0..count)
        .map(|i| {
            let x = inner.x.saturating_add((i as u16).saturating_mul(width));
            let right = if i + 1 == count {
                inner.right().saturating_sub(2)
            } else {
                x.saturating_add(width)
            };
            Rect::new(x, inner.y, right.saturating_sub(x), 1)
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
