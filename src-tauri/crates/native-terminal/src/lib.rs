mod surface;
mod terminal;
mod theme;

#[cfg(target_os = "macos")]
mod platform;

pub use surface::TerminalSurface;
pub use terminal::TerminalBackend;
pub use theme::TerminalTheme;

#[cfg(target_os = "macos")]
pub use platform::macos::{NativeTerminalView, SendableNativeView};
