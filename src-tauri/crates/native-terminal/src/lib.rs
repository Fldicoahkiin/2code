mod surface;
mod terminal;

#[cfg(target_os = "macos")]
mod platform;

pub use surface::TerminalSurface;
pub use terminal::TerminalBackend;

#[cfg(target_os = "macos")]
pub use platform::macos::{NativeTerminalView, SendableNativeView};
