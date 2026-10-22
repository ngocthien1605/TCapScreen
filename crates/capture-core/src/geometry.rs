//! Integer-pixel geometry and DPI conversions shared by capture and overlay code.
//!
//! All capture coordinates are **physical pixels** in the virtual-desktop space, where the
//! primary monitor's top-left is the origin and other monitors may have negative offsets.
//! Rectangles carry a non-negative width/height invariant (enforced by the constructors).

/// A point in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned rectangle in physical pixels. `width`/`height` are always `>= 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    /// Create a rect, clamping negative extents to zero.
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width: width.max(0),
            height: height.max(0),
        }
    }

    /// Build a normalized rect from two arbitrary corner points (handles any drag direction).
    pub fn from_points(a: Point, b: Point) -> Self {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        let width = (a.x - b.x).abs();
        let height = (a.y - b.y).abs();
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn left(&self) -> i32 {
        self.x
    }
    pub const fn top(&self) -> i32 {
        self.y
    }
    pub const fn right(&self) -> i32 {
        self.x + self.width
    }
    pub const fn bottom(&self) -> i32 {
        self.y + self.height
    }

    /// Area in pixels (as `i64` to avoid overflow on large virtual desktops).
    pub fn area(&self) -> i64 {
        self.width as i64 * self.height as i64
    }

    /// True if the rect has zero area.
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// True if the point lies inside the rect (left/top inclusive, right/bottom exclusive).
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.left() && p.x < self.right() && p.y >= self.top() && p.y < self.bottom()
    }

    /// Intersection of two rects, or `None` if they do not overlap.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.left().max(other.left());
        let y = self.top().max(other.top());
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right > x && bottom > y {
            Some(Rect::new(x, y, right - x, bottom - y))
        } else {
            None
        }
    }

    /// Smallest rect containing both inputs (bounding box / union).
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.left().min(other.left());
        let y = self.top().min(other.top());
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Rect::new(x, y, right - x, bottom - y)
    }

    /// Clamp this rect to lie within `bounds`; returns `None` if there is no overlap.
    pub fn clamp_to(&self, bounds: &Rect) -> Option<Rect> {
        self.intersect(bounds)
    }

    /// Translate the rect so its coordinates are relative to `origin` (e.g. convert a
    /// virtual-desktop rect to monitor-local pixels).
    pub fn to_local(&self, origin: Point) -> Rect {
        Rect::new(
            self.x - origin.x,
            self.y - origin.y,
            self.width,
            self.height,
        )
    }
}

/// The bounding box (union) of a set of rects. Empty input yields an empty rect.
pub fn bounding_box(rects: &[Rect]) -> Rect {
    rects
        .iter()
        .copied()
        .reduce(|acc, r| acc.union(&r))
        .unwrap_or(Rect::new(0, 0, 0, 0))
}

/// Convert a logical (DPI-independent) length to physical pixels for a given scale factor.
/// e.g. at 150% scale, 100 logical px -> 150 physical px.
pub fn logical_to_physical(logical: f64, scale_factor: f64) -> i32 {
    (logical * scale_factor).round() as i32
}

/// Convert a physical-pixel length to logical units for a given scale factor.
pub fn physical_to_logical(physical: i32, scale_factor: f64) -> f64 {
    if scale_factor == 0.0 {
        return physical as f64;
    }
    physical as f64 / scale_factor
}

/// Convert a Windows DPI value (96 = 100%) to a scale factor (1.0 = 100%).
pub fn dpi_to_scale(dpi: u32) -> f64 {
    dpi as f64 / 96.0
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn from_points_handles_any_drag_direction() {
        let a = Point::new(100, 80);
        let b = Point::new(40, 200);
        let r = Rect::from_points(a, b);
        assert_eq!(r, Rect::new(40, 80, 60, 120));
        // Reverse order yields the same rect.
        assert_eq!(Rect::from_points(b, a), r);
    }

    #[test]
    fn zero_area_region_is_empty() {
        let r = Rect::from_points(Point::new(10, 10), Point::new(10, 10));
        assert!(r.is_empty());
        assert_eq!(r.area(), 0);
    }

    #[test]
    fn intersect_overlapping() {
        let a = Rect::new(0, 0, 100, 100);
        let b = Rect::new(50, 50, 100, 100);
        assert_eq!(a.intersect(&b), Some(Rect::new(50, 50, 50, 50)));
    }

    #[test]
    fn intersect_disjoint_is_none() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(100, 100, 10, 10);
        assert_eq!(a.intersect(&b), None);
    }

    #[test]
    fn edge_touching_is_not_overlap() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(10, 0, 10, 10); // shares the x=10 edge only
        assert_eq!(a.intersect(&b), None);
    }

    #[test]
    fn union_across_negative_offsets() {
        // A left monitor at negative x, primary at origin.
        let left = Rect::new(-1920, 0, 1920, 1080);
        let primary = Rect::new(0, 0, 2560, 1440);
        let bb = left.union(&primary);
        assert_eq!(bb, Rect::new(-1920, 0, 4480, 1440));
    }

    #[test]
    fn bounding_box_of_three_monitors() {
        let rects = [
            Rect::new(0, 0, 2560, 1440),
            Rect::new(-1920, 0, 1920, 1080),
            Rect::new(2560, -200, 1920, 1080),
        ];
        let bb = bounding_box(&rects);
        assert_eq!(bb.left(), -1920);
        assert_eq!(bb.top(), -200);
        assert_eq!(bb.right(), 4480);
        assert_eq!(bb.bottom(), 1440);
    }

    #[test]
    fn bounding_box_empty_input() {
        assert_eq!(bounding_box(&[]), Rect::new(0, 0, 0, 0));
    }

    #[test]
    fn contains_is_half_open() {
        let r = Rect::new(0, 0, 10, 10);
        assert!(r.contains(Point::new(0, 0)));
        assert!(r.contains(Point::new(9, 9)));
        assert!(!r.contains(Point::new(10, 10))); // right/bottom exclusive
        assert!(!r.contains(Point::new(-1, 5)));
    }

    #[test]
    fn clamp_region_to_monitor() {
        // Region spans x 2500..2700 (clamped to 2560 -> width 60) and y -50..150
        // (clamped to 0 -> height 150).
        let region = Rect::new(2500, -50, 200, 200);
        let monitor = Rect::new(0, 0, 2560, 1440);
        assert_eq!(region.clamp_to(&monitor), Some(Rect::new(2500, 0, 60, 150)));
    }

    #[test]
    fn to_local_translates_to_monitor_space() {
        let region = Rect::new(-1820, 100, 200, 150); // on the left monitor
        let monitor_origin = Point::new(-1920, 0);
        assert_eq!(
            region.to_local(monitor_origin),
            Rect::new(100, 100, 200, 150)
        );
    }

    #[test]
    fn dpi_and_scale_conversions() {
        assert_eq!(dpi_to_scale(96), 1.0);
        assert_eq!(dpi_to_scale(144), 1.5);
        assert_eq!(dpi_to_scale(192), 2.0);
        assert_eq!(logical_to_physical(100.0, 1.5), 150);
        assert_eq!(logical_to_physical(100.0, 2.0), 200);
        assert!((physical_to_logical(150, 1.5) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn physical_to_logical_zero_scale_is_safe() {
        assert_eq!(physical_to_logical(100, 0.0), 100.0);
    }
}
