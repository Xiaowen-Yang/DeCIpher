//! Renderable trait and composable layout primitives.
//!
//! Renderables are composable widgets that know their desired height and
//! can render themselves into a ratatui `Buffer`. They are used to build
//! the bottom pane (viewport) of the TUI.
//!
//! Layout compositions:
//! - `ColumnRenderable` — vertical stacking with clipping
//! - `FlexRenderable` — Flutter-inspired flex allocation (2-pass: measure then allocate)
//! - `RowRenderable` — horizontal layout
//! - `InsetRenderable` — padding/insets

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

// ── Renderable trait ───────────────────────────────────────────────────────

/// A composable, measurable widget.
///
/// Unlike ratatui's `Widget` trait which consumes self, `Renderable` takes
/// `&self` so it can be measured before rendering (2-pass layout).
pub trait Renderable {
    /// Render into the given buffer area.
    fn render(&self, area: Rect, buf: &mut Buffer);

    /// How many rows this renderable needs at the given width.
    fn desired_height(&self, width: u16) -> u16;

    /// Optional cursor position within the area (for input fields).
    fn cursor_pos(&self, _area: Rect) -> Option<(u16, u16)> {
        None
    }
}

// ── RenderableItem ─────────────────────────────────────────────────────────

/// Wrapper for owned or borrowed renderables in layout compositions.
pub enum RenderableItem<'a> {
    Owned(Box<dyn Renderable + 'a>),
    Borrowed(&'a dyn Renderable),
}

impl<'a> RenderableItem<'a> {
    pub fn as_renderable(&self) -> &dyn Renderable {
        match self {
            RenderableItem::Owned(r) => r.as_ref(),
            RenderableItem::Borrowed(r) => *r,
        }
    }
}

// ── ColumnRenderable ───────────────────────────────────────────────────────

/// Vertical stacking — renders children top to bottom, clipping at area height.
pub struct ColumnRenderable<'a> {
    pub children: Vec<RenderableItem<'a>>,
    pub gap: u16,
}

impl<'a> ColumnRenderable<'a> {
    pub fn new() -> Self {
        Self { children: Vec::new(), gap: 0 }
    }

    pub fn with_gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    pub fn push(mut self, item: RenderableItem<'a>) -> Self {
        self.children.push(item);
        self
    }

    pub fn push_owned(self, item: impl Renderable + 'a) -> Self {
        self.push(RenderableItem::Owned(Box::new(item)))
    }

    pub fn push_borrowed(self, item: &'a dyn Renderable) -> Self {
        self.push(RenderableItem::Borrowed(item))
    }
}

impl Renderable for ColumnRenderable<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let mut y = area.y;
        let bottom = area.y + area.height;

        for (i, child) in self.children.iter().enumerate() {
            if y >= bottom {
                break;
            }
            let r = child.as_renderable();
            let h = r.desired_height(area.width).min(bottom - y);
            let child_area = Rect::new(area.x, y, area.width, h);
            r.render(child_area, buf);
            y += h;
            if i + 1 < self.children.len() {
                y += self.gap.min(bottom.saturating_sub(y));
            }
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        let children_height: u16 = self.children
            .iter()
            .map(|c| c.as_renderable().desired_height(width))
            .sum();
        let gaps = if self.children.len() > 1 {
            self.gap * (self.children.len() as u16 - 1)
        } else {
            0
        };
        children_height + gaps
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        let mut y = area.y;
        let bottom = area.y + area.height;
        for (i, child) in self.children.iter().enumerate() {
            if y >= bottom {
                break;
            }
            let r = child.as_renderable();
            let h = r.desired_height(area.width).min(bottom - y);
            let child_area = Rect::new(area.x, y, area.width, h);
            if let Some(pos) = r.cursor_pos(child_area) {
                return Some(pos);
            }
            y += h;
            if i + 1 < self.children.len() {
                y += self.gap;
            }
        }
        None
    }
}

// ── FlexRenderable ─────────────────────────────────────────────────────────

/// A child in a flex layout — either fixed-size or flexible.
pub struct FlexChild<'a> {
    pub flex: u16, // 0 = fixed (use desired_height), >0 = flex weight
    pub child: RenderableItem<'a>,
}

impl<'a> FlexChild<'a> {
    pub fn fixed(child: impl Renderable + 'a) -> Self {
        Self { flex: 0, child: RenderableItem::Owned(Box::new(child)) }
    }

    pub fn flexible(flex: u16, child: impl Renderable + 'a) -> Self {
        Self { flex, child: RenderableItem::Owned(Box::new(child)) }
    }
}

/// Flutter-inspired flex layout (vertical).
///
/// Two-pass allocation:
/// 1. Measure all fixed children (flex=0), sum their desired_height
/// 2. Distribute remaining space to flex children by weight
pub struct FlexRenderable<'a> {
    pub children: Vec<FlexChild<'a>>,
}

impl<'a> FlexRenderable<'a> {
    pub fn new() -> Self {
        Self { children: Vec::new() }
    }

    pub fn push(mut self, child: FlexChild<'a>) -> Self {
        self.children.push(child);
        self
    }

    /// Compute (y_offset, height) for each child.
    fn layout(&self, area: Rect) -> Vec<(u16, u16)> {
        // Pass 1: measure fixed children
        let mut fixed_total: u16 = 0;
        let mut flex_total: u16 = 0;
        let mut measurements: Vec<u16> = Vec::with_capacity(self.children.len());

        for child in &self.children {
            let desired = child.child.as_renderable().desired_height(area.width);
            if child.flex == 0 {
                fixed_total = fixed_total.saturating_add(desired);
                measurements.push(desired);
            } else {
                flex_total += child.flex;
                measurements.push(0); // placeholder
            }
        }

        // Pass 2: distribute remaining space
        let remaining = area.height.saturating_sub(fixed_total);
        if flex_total > 0 {
            for (i, child) in self.children.iter().enumerate() {
                if child.flex > 0 {
                    measurements[i] = (remaining as u32 * child.flex as u32 / flex_total as u32) as u16;
                }
            }
        }

        // Build layout positions
        let mut result = Vec::with_capacity(self.children.len());
        let mut y = area.y;
        for &h in &measurements {
            let clamped = h.min(area.y + area.height - y);
            result.push((y, clamped));
            y += clamped;
        }
        result
    }
}

impl Renderable for FlexRenderable<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let positions = self.layout(area);
        for (i, child) in self.children.iter().enumerate() {
            if let Some(&(y, h)) = positions.get(i) {
                if h > 0 {
                    let child_area = Rect::new(area.x, y, area.width, h);
                    child.child.as_renderable().render(child_area, buf);
                }
            }
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.children
            .iter()
            .map(|c| {
                if c.flex == 0 {
                    c.child.as_renderable().desired_height(width)
                } else {
                    // Flex children contribute their minimum desired height
                    c.child.as_renderable().desired_height(width)
                }
            })
            .sum()
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        let positions = self.layout(area);
        for (i, child) in self.children.iter().enumerate() {
            if let Some(&(y, h)) = positions.get(i) {
                if h > 0 {
                    let child_area = Rect::new(area.x, y, area.width, h);
                    if let Some(pos) = child.child.as_renderable().cursor_pos(child_area) {
                        return Some(pos);
                    }
                }
            }
        }
        None
    }
}

// ── RowRenderable ──────────────────────────────────────────────────────────

/// Horizontal layout — renders children left to right, dividing width equally
/// (or proportionally when flex weights are added later).
pub struct RowRenderable<'a> {
    pub children: Vec<RenderableItem<'a>>,
    pub gap: u16,
}

impl<'a> RowRenderable<'a> {
    pub fn new() -> Self {
        Self { children: Vec::new(), gap: 0 }
    }

    pub fn with_gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    pub fn push_owned(mut self, item: impl Renderable + 'a) -> Self {
        self.children.push(RenderableItem::Owned(Box::new(item)));
        self
    }
}

impl Renderable for RowRenderable<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if self.children.is_empty() {
            return;
        }
        let total_gap = self.gap * (self.children.len() as u16 - 1);
        let usable = area.width.saturating_sub(total_gap);
        let child_width = usable / self.children.len() as u16;

        let mut x = area.x;
        for (i, child) in self.children.iter().enumerate() {
            let w = if i + 1 == self.children.len() {
                // Last child gets remaining width
                (area.x + area.width).saturating_sub(x)
            } else {
                child_width
            };
            let child_area = Rect::new(x, area.y, w, area.height);
            child.as_renderable().render(child_area, buf);
            x += w + self.gap;
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        if self.children.is_empty() {
            return 0;
        }
        let total_gap = self.gap * (self.children.len() as u16 - 1);
        let usable = width.saturating_sub(total_gap);
        let child_width = usable / self.children.len().max(1) as u16;
        self.children
            .iter()
            .map(|c| c.as_renderable().desired_height(child_width))
            .max()
            .unwrap_or(0)
    }
}

// ── InsetRenderable ────────────────────────────────────────────────────────

/// Wraps a child with padding on all sides.
pub struct InsetRenderable<'a> {
    pub child: RenderableItem<'a>,
    pub top: u16,
    pub bottom: u16,
    pub left: u16,
    pub right: u16,
}

impl<'a> InsetRenderable<'a> {
    pub fn new(child: impl Renderable + 'a) -> Self {
        Self {
            child: RenderableItem::Owned(Box::new(child)),
            top: 0,
            bottom: 0,
            left: 0,
            right: 0,
        }
    }

    pub fn uniform(child: impl Renderable + 'a, padding: u16) -> Self {
        Self {
            child: RenderableItem::Owned(Box::new(child)),
            top: padding,
            bottom: padding,
            left: padding,
            right: padding,
        }
    }

    pub fn horizontal(child: impl Renderable + 'a, h: u16) -> Self {
        Self {
            child: RenderableItem::Owned(Box::new(child)),
            top: 0,
            bottom: 0,
            left: h,
            right: h,
        }
    }

    pub fn vertical(child: impl Renderable + 'a, v: u16) -> Self {
        Self {
            child: RenderableItem::Owned(Box::new(child)),
            top: v,
            bottom: v,
            left: 0,
            right: 0,
        }
    }
}

impl Renderable for InsetRenderable<'_> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let inner_width = area.width.saturating_sub(self.left + self.right);
        let inner_height = area.height.saturating_sub(self.top + self.bottom);
        if inner_width == 0 || inner_height == 0 {
            return;
        }
        let inner = Rect::new(
            area.x + self.left,
            area.y + self.top,
            inner_width,
            inner_height,
        );
        self.child.as_renderable().render(inner, buf);
    }

    fn desired_height(&self, width: u16) -> u16 {
        let inner_width = width.saturating_sub(self.left + self.right);
        self.child.as_renderable().desired_height(inner_width) + self.top + self.bottom
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        let inner_width = area.width.saturating_sub(self.left + self.right);
        let inner_height = area.height.saturating_sub(self.top + self.bottom);
        let inner = Rect::new(
            area.x + self.left,
            area.y + self.top,
            inner_width,
            inner_height,
        );
        self.child.as_renderable().cursor_pos(inner)
    }
}

// ── Empty Renderable ───────────────────────────────────────────────────────

/// A zero-height renderable (placeholder / spacer).
pub struct EmptyRenderable;

impl Renderable for EmptyRenderable {
    fn render(&self, _area: Rect, _buf: &mut Buffer) {}
    fn desired_height(&self, _width: u16) -> u16 { 0 }
}

/// A fixed-height spacer.
pub struct Spacer(pub u16);

impl Renderable for Spacer {
    fn render(&self, _area: Rect, _buf: &mut Buffer) {}
    fn desired_height(&self, _width: u16) -> u16 { self.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestWidget(u16);

    impl Renderable for TestWidget {
        fn render(&self, _area: Rect, _buf: &mut Buffer) {}
        fn desired_height(&self, _width: u16) -> u16 { self.0 }
    }

    #[test]
    fn column_stacks_heights() {
        let col = ColumnRenderable::new()
            .push_owned(TestWidget(3))
            .push_owned(TestWidget(2));
        assert_eq!(col.desired_height(80), 5);
    }

    #[test]
    fn column_with_gap() {
        let col = ColumnRenderable::new()
            .with_gap(1)
            .push_owned(TestWidget(3))
            .push_owned(TestWidget(2));
        assert_eq!(col.desired_height(80), 6); // 3 + 1 + 2
    }

    #[test]
    fn inset_adds_padding() {
        let inset = InsetRenderable::uniform(TestWidget(5), 2);
        assert_eq!(inset.desired_height(80), 9); // 5 + 2 + 2
    }

    #[test]
    fn flex_fixed_children() {
        let flex = FlexRenderable::new()
            .push(FlexChild::fixed(TestWidget(3)))
            .push(FlexChild::fixed(TestWidget(2)));
        assert_eq!(flex.desired_height(80), 5);
    }

    #[test]
    fn row_max_height() {
        let row = RowRenderable::new()
            .push_owned(TestWidget(3))
            .push_owned(TestWidget(5));
        assert_eq!(row.desired_height(80), 5);
    }

    #[test]
    fn empty_renderable() {
        assert_eq!(EmptyRenderable.desired_height(80), 0);
    }

    #[test]
    fn spacer_height() {
        assert_eq!(Spacer(3).desired_height(80), 3);
    }
}
