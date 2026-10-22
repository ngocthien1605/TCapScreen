//! The annotation editor's pure model: tools, committed items, in-progress drag / text entry,
//! and the image⇄view coordinate mapping. No Win32 here, so it is fully unit-tested; the window
//! in [`super::win`] only forwards input and paints.
//!
//! All coordinates in the model are **image pixels** (top-left origin).

use capture_core::{Point, Rect};

/// A drag must cover at least this many image pixels on both axes to count as a shape.
pub const MIN_SIZE: i32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// Draw a red rectangle outline.
    Rect,
    /// Draw a red rectangle, then type text inside it.
    Text,
}

/// A committed annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Rect(Rect),
    Text { rect: Rect, text: String },
}

/// A text box that has been drawn and is currently being typed into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typing {
    pub rect: Rect,
    pub text: String,
}

/// What `Esc` did, so the window knows whether to close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeResult {
    /// Cancelled an in-progress drag or text entry.
    Cancelled,
    /// Nothing was in progress — the caller should close the editor.
    Nothing,
}

#[derive(Debug)]
pub struct Editor {
    width: i32,
    height: i32,
    tool: Tool,
    items: Vec<Item>,
    drag: Option<(Point, Point)>,
    typing: Option<Typing>,
}

impl Editor {
    pub fn new(width: i32, height: i32) -> Self {
        Editor {
            width,
            height,
            tool: Tool::Rect,
            items: Vec::new(),
            drag: None,
            typing: None,
        }
    }

    #[cfg(test)]
    pub fn tool(&self) -> Tool {
        self.tool
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn typing(&self) -> Option<&Typing> {
        self.typing.as_ref()
    }

    pub fn is_typing(&self) -> bool {
        self.typing.is_some()
    }

    /// The rectangle being rubber-banded right now, if any.
    pub fn drag_rect(&self) -> Option<Rect> {
        self.drag.map(|(a, b)| self.normalized(a, b))
    }

    /// Switch tool. Any text being typed is committed first (or dropped if empty).
    pub fn set_tool(&mut self, tool: Tool) {
        self.commit_typing();
        self.drag = None;
        self.tool = tool;
    }

    pub fn on_down(&mut self, p: Point) {
        // Clicking away from a text box finishes it, as in most editors.
        self.commit_typing();
        let p = self.clamp(p);
        self.drag = Some((p, p));
    }

    pub fn on_move(&mut self, p: Point) {
        let p = self.clamp(p);
        if let Some((_, end)) = self.drag.as_mut() {
            *end = p;
        }
    }

    /// Finish the drag. A big-enough rectangle is committed (Rect tool) or turned into a text
    /// box awaiting typing (Text tool); a tiny one — an accidental click — is discarded.
    pub fn on_up(&mut self, p: Point) {
        self.on_move(p);
        let Some((a, b)) = self.drag.take() else {
            return;
        };
        let r = self.normalized(a, b);
        if r.width < MIN_SIZE || r.height < MIN_SIZE {
            return;
        }
        match self.tool {
            Tool::Rect => self.items.push(Item::Rect(r)),
            Tool::Text => {
                self.typing = Some(Typing {
                    rect: r,
                    text: String::new(),
                })
            }
        }
    }

    /// A typed character. Control characters are ignored (Enter/Backspace have own methods).
    pub fn on_char(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        if let Some(t) = self.typing.as_mut() {
            t.text.push(c);
        }
    }

    pub fn on_backspace(&mut self) {
        if let Some(t) = self.typing.as_mut() {
            t.text.pop();
        }
    }

    /// `Enter`: save the text box. Returns true if there was a text box being typed.
    pub fn on_enter(&mut self) -> bool {
        let was_typing = self.typing.is_some();
        self.commit_typing();
        was_typing
    }

    pub fn on_escape(&mut self) -> EscapeResult {
        if self.drag.take().is_some() || self.typing.take().is_some() {
            EscapeResult::Cancelled
        } else {
            EscapeResult::Nothing
        }
    }

    /// Undo: cancel an in-progress shape, otherwise remove the last committed one.
    pub fn undo(&mut self) {
        if self.drag.take().is_some() || self.typing.take().is_some() {
            return;
        }
        self.items.pop();
    }

    fn commit_typing(&mut self) {
        if let Some(t) = self.typing.take() {
            if !t.text.trim().is_empty() {
                self.items.push(Item::Text {
                    rect: t.rect,
                    text: t.text,
                });
            }
        }
    }

    fn clamp(&self, p: Point) -> Point {
        Point::new(
            p.x.clamp(0, (self.width - 1).max(0)),
            p.y.clamp(0, (self.height - 1).max(0)),
        )
    }

    fn normalized(&self, a: Point, b: Point) -> Rect {
        let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
        let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }
}

/// Font height (image px) for a text box: scales with the box but stays readable.
pub fn font_px(rect: Rect) -> i32 {
    (rect.height / 2).clamp(14, 32)
}

/// Maps between image pixels and window (client) pixels: the image is shown at `scale`, with its
/// top-left at `(ox, oy)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub scale: f64,
    pub ox: i32,
    pub oy: i32,
}

impl View {
    /// 1:1, no offset — what the exporter uses.
    pub const IDENTITY: View = View {
        scale: 1.0,
        ox: 0,
        oy: 0,
    };

    /// Fit an `iw × ih` image into the `area` rectangle (never upscaling past 1:1), centred.
    pub fn fit(area: Rect, iw: i32, ih: i32) -> View {
        let iw = iw.max(1);
        let ih = ih.max(1);
        let scale = (area.width as f64 / iw as f64)
            .min(area.height as f64 / ih as f64)
            .clamp(0.01, 1.0);
        let dw = (iw as f64 * scale).round() as i32;
        let dh = (ih as f64 * scale).round() as i32;
        View {
            scale,
            ox: area.x + (area.width - dw) / 2,
            oy: area.y + (area.height - dh) / 2,
        }
    }

    pub fn px(self, v: i32) -> i32 {
        (v as f64 * self.scale).round() as i32
    }

    #[cfg(test)]
    pub fn to_view(self, p: Point) -> Point {
        Point::new(self.ox + self.px(p.x), self.oy + self.px(p.y))
    }

    pub fn to_image(self, x: i32, y: i32) -> Point {
        Point::new(
            ((x - self.ox) as f64 / self.scale).round() as i32,
            ((y - self.oy) as f64 / self.scale).round() as i32,
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn drag(e: &mut Editor, a: (i32, i32), b: (i32, i32)) {
        e.on_down(Point::new(a.0, a.1));
        e.on_move(Point::new((a.0 + b.0) / 2, (a.1 + b.1) / 2));
        e.on_up(Point::new(b.0, b.1));
    }

    fn type_str(e: &mut Editor, s: &str) {
        for c in s.chars() {
            e.on_char(c);
        }
    }

    #[test]
    fn rect_tool_commits_a_normalized_rectangle() {
        let mut e = Editor::new(200, 100);
        drag(&mut e, (80, 60), (20, 10)); // dragged up-left
        assert_eq!(e.items(), &[Item::Rect(Rect::new(20, 10, 60, 50))]);
    }

    #[test]
    fn tiny_drag_is_an_accidental_click_and_is_discarded() {
        let mut e = Editor::new(200, 100);
        drag(&mut e, (50, 50), (53, 52));
        assert!(e.items().is_empty());
        assert!(!e.is_typing());
    }

    #[test]
    fn drag_is_clamped_to_the_image() {
        let mut e = Editor::new(200, 100);
        drag(&mut e, (-30, -30), (500, 500));
        assert_eq!(e.items(), &[Item::Rect(Rect::new(0, 0, 199, 99))]);
    }

    #[test]
    fn drag_rect_is_live_while_dragging() {
        let mut e = Editor::new(200, 100);
        e.on_down(Point::new(10, 10));
        e.on_move(Point::new(60, 40));
        assert_eq!(e.drag_rect(), Some(Rect::new(10, 10, 50, 30)));
        e.on_up(Point::new(60, 40));
        assert_eq!(e.drag_rect(), None);
    }

    #[test]
    fn text_tool_draw_then_type_then_enter_saves() {
        let mut e = Editor::new(200, 100);
        e.set_tool(Tool::Text);
        drag(&mut e, (10, 10), (110, 50));
        assert!(e.is_typing(), "drawing the box starts text entry");
        assert!(e.items().is_empty(), "not saved until Enter");

        type_str(&mut e, "Hello");
        assert_eq!(e.typing().unwrap().text, "Hello");

        assert!(e.on_enter());
        assert!(!e.is_typing());
        assert_eq!(
            e.items(),
            &[Item::Text {
                rect: Rect::new(10, 10, 100, 40),
                text: "Hello".into()
            }]
        );
    }

    #[test]
    fn backspace_edits_and_control_chars_are_ignored() {
        let mut e = Editor::new(200, 100);
        e.set_tool(Tool::Text);
        drag(&mut e, (10, 10), (110, 50));
        type_str(&mut e, "abc");
        e.on_char('\u{8}'); // raw control char must not be inserted
        e.on_char('\r');
        e.on_backspace();
        assert_eq!(e.typing().unwrap().text, "ab");
    }

    #[test]
    fn enter_on_an_empty_text_box_discards_it() {
        let mut e = Editor::new(200, 100);
        e.set_tool(Tool::Text);
        drag(&mut e, (10, 10), (110, 50));
        type_str(&mut e, "   ");
        assert!(e.on_enter());
        assert!(e.items().is_empty());
    }

    #[test]
    fn enter_without_a_text_box_does_nothing() {
        let mut e = Editor::new(200, 100);
        assert!(!e.on_enter());
    }

    #[test]
    fn clicking_elsewhere_commits_the_text_being_typed() {
        let mut e = Editor::new(200, 100);
        e.set_tool(Tool::Text);
        drag(&mut e, (10, 10), (110, 50));
        type_str(&mut e, "note");
        e.on_down(Point::new(150, 80));
        assert!(!e.is_typing());
        assert_eq!(e.items().len(), 1);
    }

    #[test]
    fn switching_tool_commits_text() {
        let mut e = Editor::new(200, 100);
        e.set_tool(Tool::Text);
        drag(&mut e, (10, 10), (110, 50));
        type_str(&mut e, "x");
        e.set_tool(Tool::Rect);
        assert_eq!(e.items().len(), 1);
        assert_eq!(e.tool(), Tool::Rect);
    }

    #[test]
    fn escape_cancels_in_progress_work_then_reports_nothing() {
        let mut e = Editor::new(200, 100);
        e.on_down(Point::new(10, 10));
        e.on_move(Point::new(60, 40));
        assert_eq!(e.on_escape(), EscapeResult::Cancelled);
        assert!(e.items().is_empty());

        e.set_tool(Tool::Text);
        drag(&mut e, (10, 10), (110, 50));
        type_str(&mut e, "draft");
        assert_eq!(e.on_escape(), EscapeResult::Cancelled);
        assert!(e.items().is_empty(), "Esc discards the unsaved text");

        assert_eq!(e.on_escape(), EscapeResult::Nothing);
    }

    #[test]
    fn undo_cancels_in_progress_else_pops_last_item() {
        let mut e = Editor::new(200, 100);
        drag(&mut e, (10, 10), (50, 50));
        drag(&mut e, (60, 10), (100, 50));
        assert_eq!(e.items().len(), 2);
        e.undo();
        assert_eq!(e.items().len(), 1);

        e.set_tool(Tool::Text);
        drag(&mut e, (10, 60), (90, 90));
        e.undo(); // cancels the text box, keeps the committed rect
        assert!(!e.is_typing());
        assert_eq!(e.items().len(), 1);
        e.undo();
        assert!(e.items().is_empty());
        e.undo(); // nothing left: no panic
    }

    #[test]
    fn typing_without_a_text_box_is_ignored() {
        let mut e = Editor::new(200, 100);
        type_str(&mut e, "stray");
        assert!(e.items().is_empty() && !e.is_typing());
    }

    #[test]
    fn font_size_scales_with_box_but_is_bounded() {
        assert_eq!(font_px(Rect::new(0, 0, 100, 10)), 14);
        assert_eq!(font_px(Rect::new(0, 0, 100, 40)), 20);
        assert_eq!(font_px(Rect::new(0, 0, 100, 400)), 32);
    }

    #[test]
    fn view_fit_never_upscales_and_centres() {
        // Small image in a big area: stays 1:1, centred.
        let v = View::fit(Rect::new(0, 40, 1000, 600), 200, 100);
        assert_eq!(v.scale, 1.0);
        assert_eq!((v.ox, v.oy), (400, 40 + 250));
        // Big image: scaled down to fit the limiting axis.
        let v = View::fit(Rect::new(0, 0, 500, 500), 1000, 500);
        assert!((v.scale - 0.5).abs() < 1e-9);
        assert_eq!(v.oy, 125);
    }

    #[test]
    fn view_round_trips_points() {
        let v = View::fit(Rect::new(10, 50, 400, 300), 1600, 1200);
        let p = Point::new(800, 600);
        let back = v.to_image(v.to_view(p).x, v.to_view(p).y);
        assert!((back.x - p.x).abs() <= 2 && (back.y - p.y).abs() <= 2);
    }
}
