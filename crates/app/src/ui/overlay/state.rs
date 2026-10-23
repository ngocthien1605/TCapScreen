//! Pure selection-overlay state machine — no Win32, fully unit-testable.
//!
//! All coordinates here are **physical pixels in virtual-desktop space** (the same space the
//! capture backend uses). The Win32 plumbing in [`super::win`] converts client coordinates to
//! this space (by adding the overlay's bounding-box origin) before feeding events in, and reads
//! [`SelectionState::selection_rect`] / [`SelectionState::hovered_window`] back out to paint.

use capture_core::{Point, Rect, WindowId, WindowInfo};

/// Minimum drag distance (in px, Chebyshev) before a press+release counts as a region drag
/// rather than a click. Below this, a release is treated as a window pick.
pub const MIN_DRAG: i32 = 4;

/// What the user is currently doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No button down: hovering (used to highlight the window under the cursor).
    Hovering,
    /// Left button down: rubber-banding a region from `anchor` to `cursor`.
    Dragging,
}

/// The result of interacting with the overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A rectangular region was chosen (virtual-desktop physical px).
    Region(Rect),
    /// A whole window was chosen.
    Window(WindowId),
    /// The user cancelled (Esc or right-click).
    Cancel,
}

/// Virtual key codes the overlay reacts to (kept as plain `u32` so this module stays
/// Win32-free; the plumbing passes `wparam` through).
pub mod vk {
    pub const ESCAPE: u32 = 0x1B;
    pub const RETURN: u32 = 0x0D;
}

/// The interactive state of the selection overlay.
#[derive(Debug)]
pub struct SelectionState {
    phase: Phase,
    anchor: Point,
    cursor: Point,
    /// Candidate windows, in top-to-bottom z-order (as `EnumWindows` yields them).
    windows: Vec<WindowInfo>,
}

impl SelectionState {
    /// Create a fresh overlay state for the given candidate windows (top-most first).
    pub fn new(windows: Vec<WindowInfo>) -> Self {
        Self {
            phase: Phase::Hovering,
            anchor: Point::new(0, 0),
            cursor: Point::new(0, 0),
            windows,
        }
    }

    /// Left mouse button pressed at `p` (virtual-desktop px).
    pub fn on_down(&mut self, p: Point) {
        self.phase = Phase::Dragging;
        self.anchor = p;
        self.cursor = p;
    }

    /// Mouse moved to `p` (virtual-desktop px).
    pub fn on_move(&mut self, p: Point) {
        self.cursor = p;
    }

    /// Left mouse button released at `p`. Returns a terminal [`Outcome`] when the gesture
    /// resolves, or `None` to keep the overlay open (e.g. a click on empty space).
    pub fn on_up(&mut self, p: Point) -> Option<Outcome> {
        self.cursor = p;
        let was_dragging = self.phase == Phase::Dragging;
        self.phase = Phase::Hovering;
        if !was_dragging {
            return None;
        }
        if self.is_click() {
            // Press+release without meaningful movement → pick the window under the cursor.
            return pick_window(&self.windows, p).map(Outcome::Window);
        }
        let rect = Rect::from_points(self.anchor, p);
        if rect.is_empty() {
            None
        } else {
            Some(Outcome::Region(rect))
        }
    }

    /// A key was pressed (virtual key code). Returns a terminal [`Outcome`] for Esc/Enter.
    pub fn on_key(&mut self, key: u32) -> Option<Outcome> {
        match key {
            vk::ESCAPE => Some(Outcome::Cancel),
            vk::RETURN => self.selection_rect().map(Outcome::Region),
            _ => None,
        }
    }

    /// Right mouse button (or any secondary action) cancels.
    pub fn on_cancel(&mut self) -> Outcome {
        Outcome::Cancel
    }

    /// The rectangle to paint for the current drag, if any (empty drags excluded).
    pub fn selection_rect(&self) -> Option<Rect> {
        if self.phase == Phase::Dragging {
            let r = Rect::from_points(self.anchor, self.cursor);
            if !r.is_empty() {
                return Some(r);
            }
        }
        None
    }

    /// The window to highlight while hovering (not during a drag), if any.
    pub fn hovered_window(&self) -> Option<&WindowInfo> {
        if self.phase == Phase::Hovering {
            let p = self.cursor;
            self.windows.iter().find(|w| w.bounds.contains(p))
        } else {
            None
        }
    }

    /// The live cursor position (for crosshair / label placement).
    pub fn cursor(&self) -> Point {
        self.cursor
    }

    fn is_click(&self) -> bool {
        (self.cursor.x - self.anchor.x).abs() < MIN_DRAG
            && (self.cursor.y - self.anchor.y).abs() < MIN_DRAG
    }
}

/// Pick the top-most window whose bounds contain `p`. `windows` must be in top-to-bottom
/// z-order; the first containing match wins.
pub fn pick_window(windows: &[WindowInfo], p: Point) -> Option<WindowId> {
    windows.iter().find(|w| w.bounds.contains(p)).map(|w| w.id)
}

/// The two crosshair bars through `cursor` (client coordinates) on a `width × height` surface:
/// a horizontal bar spanning the full width and a vertical bar spanning the full height. Each is
/// `1 + 2 * grow` px thick, centred on the cursor — `grow = 0` gives the 1 px line, `grow = 1`
/// the 3 px dark edge drawn beneath it. Pure, for unit testing.
pub fn crosshair_bars(cursor: Point, width: i32, height: i32, grow: i32) -> [Rect; 2] {
    let thick = 1 + 2 * grow;
    [
        Rect::new(0, cursor.y - grow, width, thick),
        Rect::new(cursor.x - grow, 0, thick, height),
    ]
}

/// Decide where to draw the `W × H` dimension label so it stays on-screen: normally just
/// below-right of the cursor, but flipped inside when near the right/bottom edge of `bounds`.
/// Returns the top-left corner for a label of `label_w × label_h` px. Pure, for unit testing.
pub fn label_position(cursor: Point, label_w: i32, label_h: i32, bounds: Rect) -> Point {
    const GAP: i32 = 12;
    let mut x = cursor.x + GAP;
    let mut y = cursor.y + GAP;
    if x + label_w > bounds.right() {
        x = cursor.x - GAP - label_w;
    }
    if y + label_h > bounds.bottom() {
        y = cursor.y - GAP - label_h;
    }
    x = x.max(bounds.left());
    y = y.max(bounds.top());
    Point::new(x, y)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn crosshair_bars_span_the_surface_through_the_cursor() {
        let [h, v] = crosshair_bars(Point::new(300, 200), 1920, 1080, 0);
        assert_eq!(h, Rect::new(0, 200, 1920, 1));
        assert_eq!(v, Rect::new(300, 0, 1, 1080));
    }

    #[test]
    fn crosshair_edge_is_one_pixel_wider_each_side() {
        let [h, v] = crosshair_bars(Point::new(300, 200), 1920, 1080, 1);
        assert_eq!(h, Rect::new(0, 199, 1920, 3));
        assert_eq!(v, Rect::new(299, 0, 3, 1080));
    }

    fn win(id: isize, x: i32, y: i32, w: i32, h: i32) -> WindowInfo {
        WindowInfo {
            id: WindowId(id),
            title: format!("win{id}"),
            bounds: Rect::new(x, y, w, h),
        }
    }

    #[test]
    fn drag_produces_region() {
        let mut s = SelectionState::new(vec![]);
        s.on_down(Point::new(10, 20));
        s.on_move(Point::new(110, 220));
        let out = s.on_up(Point::new(110, 220));
        assert_eq!(out, Some(Outcome::Region(Rect::new(10, 20, 100, 200))));
    }

    #[test]
    fn reverse_drag_is_normalized() {
        let mut s = SelectionState::new(vec![]);
        s.on_down(Point::new(110, 220));
        s.on_move(Point::new(10, 20));
        let out = s.on_up(Point::new(10, 20));
        assert_eq!(out, Some(Outcome::Region(Rect::new(10, 20, 100, 200))));
    }

    #[test]
    fn click_without_drag_picks_window() {
        let windows = vec![win(1, 0, 0, 500, 500), win(2, 100, 100, 50, 50)];
        let mut s = SelectionState::new(windows);
        s.on_down(Point::new(120, 120));
        // tiny jitter below MIN_DRAG
        s.on_move(Point::new(121, 121));
        let out = s.on_up(Point::new(121, 121));
        // Top-most (first) containing window wins.
        assert_eq!(out, Some(Outcome::Window(WindowId(1))));
    }

    #[test]
    fn click_on_empty_space_keeps_overlay_open() {
        let mut s = SelectionState::new(vec![win(1, 0, 0, 10, 10)]);
        s.on_down(Point::new(500, 500));
        let out = s.on_up(Point::new(500, 500));
        assert_eq!(out, None);
    }

    #[test]
    fn escape_cancels_enter_confirms() {
        let mut s = SelectionState::new(vec![]);
        assert_eq!(s.on_key(vk::ESCAPE), Some(Outcome::Cancel));

        let mut s = SelectionState::new(vec![]);
        s.on_down(Point::new(10, 10));
        s.on_move(Point::new(60, 40));
        assert_eq!(
            s.on_key(vk::RETURN),
            Some(Outcome::Region(Rect::new(10, 10, 50, 30)))
        );
    }

    #[test]
    fn enter_without_selection_does_nothing() {
        let mut s = SelectionState::new(vec![]);
        assert_eq!(s.on_key(vk::RETURN), None);
    }

    #[test]
    fn selection_rect_only_while_dragging() {
        let mut s = SelectionState::new(vec![]);
        assert_eq!(s.selection_rect(), None);
        s.on_down(Point::new(0, 0));
        s.on_move(Point::new(5, 5));
        assert_eq!(s.selection_rect(), Some(Rect::new(0, 0, 5, 5)));
    }

    #[test]
    fn hovered_window_tracks_top_most() {
        let windows = vec![win(1, 0, 0, 500, 500), win(2, 100, 100, 50, 50)];
        let mut s = SelectionState::new(windows);
        s.on_move(Point::new(120, 120));
        assert_eq!(s.hovered_window().map(|w| w.id), Some(WindowId(1)));
        // No highlight during a drag.
        s.on_down(Point::new(120, 120));
        s.on_move(Point::new(300, 300));
        assert!(s.hovered_window().is_none());
    }

    #[test]
    fn pick_window_first_match_wins() {
        let windows = vec![win(1, 0, 0, 500, 500), win(2, 100, 100, 50, 50)];
        assert_eq!(
            pick_window(&windows, Point::new(120, 120)),
            Some(WindowId(1))
        );
        assert_eq!(pick_window(&windows, Point::new(600, 600)), None);
    }

    #[test]
    fn label_flips_near_edges() {
        let bounds = Rect::new(0, 0, 1000, 1000);
        // Middle: below-right.
        let p = label_position(Point::new(500, 500), 80, 20, bounds);
        assert_eq!(p, Point::new(512, 512));
        // Near right/bottom: flips to above-left.
        let p = label_position(Point::new(990, 995), 80, 20, bounds);
        assert_eq!(p, Point::new(990 - 12 - 80, 995 - 12 - 20));
    }
}
