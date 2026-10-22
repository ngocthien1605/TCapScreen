//! The region/window selection overlay: a single frozen-frame window spanning the whole
//! virtual desktop. The interaction logic is the pure [`state`] machine; [`win`] is the Win32
//! plumbing (frozen backdrop, dim, rubber-band, window highlight, dimension label).

pub mod state;
mod win;

pub use state::{Outcome, Style};
pub use win::select;
