//! The annotation editor opened from the post-capture flyout's **Annotate** button: draw red
//! rectangles and red text boxes on a screenshot, then save an annotated copy or copy it to the
//! clipboard. [`model`] is the pure, unit-tested editor logic; [`win`] is the Win32 window.

mod cursor;
pub mod model;
mod win;

pub use win::open;
