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
