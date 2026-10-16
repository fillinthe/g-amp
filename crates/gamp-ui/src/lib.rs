//! G-Amp's windows, drawn pixel for pixel from classic Winamp skins.
//! Pure drawing and hit-testing: no window system here, so every window
//! can be rendered and checked in tests.

pub mod eq_window;
pub mod font;
pub mod frame;
pub mod main_window;
pub mod menu;
pub mod playlist_window;
pub mod vis;

pub use frame::Frame;
