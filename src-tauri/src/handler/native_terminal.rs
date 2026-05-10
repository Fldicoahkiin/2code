use std::sync::{Arc, Mutex};
use tauri::State;

/// Forward keyboard input from WebView to the native terminal PTY.
#[tauri::command]
pub fn write_to_native_terminal(
	surface: State<'_, Arc<Mutex<native_terminal::TerminalSurface>>>,
	data: String,
) -> Result<(), String> {
	let surface = surface.lock().map_err(|e| e.to_string())?;
	surface.write_to_pty(data.as_bytes());
	Ok(())
}

/// Paste text into the native terminal with bracketed paste support.
#[tauri::command]
pub fn paste_to_native_terminal(
	surface: State<'_, Arc<Mutex<native_terminal::TerminalSurface>>>,
	text: String,
) -> Result<(), String> {
	let surface = surface.lock().map_err(|e| e.to_string())?;
	surface.paste_to_pty(&text);
	Ok(())
}

/// Update font family and size for native terminal.
#[tauri::command]
pub fn set_native_terminal_font(
	surface: State<'_, Arc<Mutex<native_terminal::TerminalSurface>>>,
	family: String,
	size: f32,
) -> Result<(), String> {
	let mut s = surface.lock().map_err(|e| e.to_string())?;
	s.set_font(family, size);
	Ok(())
}

/// Apply terminal color theme from frontend settings.
#[tauri::command]
pub fn set_native_terminal_theme(
	surface: State<'_, Arc<Mutex<native_terminal::TerminalSurface>>>,
	background: String,
	foreground: String,
	cursor: String,
	ansi_colors: [String; 16],
) -> Result<(), String> {
	let theme =
		native_terminal::TerminalTheme::from_hex(&background, &foreground, &cursor, &ansi_colors);
	let mut s = surface.lock().map_err(|e| e.to_string())?;
	s.set_theme(theme);
	Ok(())
}

/// Resize and reposition the native terminal view + wgpu surface.
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn resize_native_terminal(
	surface: State<'_, Arc<Mutex<native_terminal::TerminalSurface>>>,
	view: State<'_, Arc<native_terminal::SendableNativeView>>,
	x: f64,
	y: f64,
	width: f64,
	height: f64,
) -> Result<(), String> {
	view.set_frame(x, y, width, height);
	let mut s = surface.lock().map_err(|e| e.to_string())?;
	s.resize(width as u32, height as u32);
	Ok(())
}

/// Show or hide the native terminal view.
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn set_native_terminal_visible(
	view: State<'_, Arc<native_terminal::SendableNativeView>>,
	visible: bool,
) -> Result<(), String> {
	view.set_hidden(!visible);
	Ok(())
}
