import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef } from "react";
import { useTerminalSettingsStore } from "@/features/settings/stores/terminalSettingsStore";
import { useTerminalTheme } from "@/features/terminal/hooks";
import type { ITheme } from "@xterm/xterm";

/** Map special keys to terminal escape sequences. */
const KEY_MAP: Record<string, string> = {
	Enter: "\r",
	Backspace: "\x7f",
	Tab: "\t",
	Escape: "\x1b",
	ArrowUp: "\x1b[A",
	ArrowDown: "\x1b[B",
	ArrowRight: "\x1b[C",
	ArrowLeft: "\x1b[D",
	Home: "\x1b[H",
	End: "\x1b[F",
	Delete: "\x1b[3~",
	PageUp: "\x1b[5~",
	PageDown: "\x1b[6~",
	Insert: "\x1b[2~",
	F1: "\x1bOP",
	F2: "\x1bOQ",
	F3: "\x1bOR",
	F4: "\x1bOS",
	F5: "\x1b[15~",
	F6: "\x1b[17~",
	F7: "\x1b[18~",
	F8: "\x1b[19~",
	F9: "\x1b[20~",
	F10: "\x1b[21~",
	F11: "\x1b[23~",
	F12: "\x1b[24~",
};

function sendThemeToNative(theme: ITheme) {
	const bg = theme.background ?? "#161616";
	const fg = theme.foreground ?? "#BFD4E1";
	const cur = theme.cursor ?? "#f0f3bd";
	const ansiColors = [
		theme.black ?? "#353535",
		theme.red ?? "#d97397",
		theme.green ?? "#CEE397",
		theme.yellow ?? "#E9CA5C",
		theme.blue ?? "#63B0C6",
		theme.magenta ?? "#E9AEBA",
		theme.cyan ?? "#70C1B3",
		theme.white ?? "#BFD4E1",
		theme.brightBlack ?? "#729098",
		theme.brightRed ?? "#ffadad",
		theme.brightGreen ?? "#caffbf",
		theme.brightYellow ?? "#f0f3bd",
		theme.brightBlue ?? "#9bf6ff",
		theme.brightMagenta ?? "#ffc6ff",
		theme.brightCyan ?? "#a8dadc",
		theme.brightWhite ?? "#ffffff",
	];
	invoke("set_native_terminal_theme", {
		background: bg,
		foreground: fg,
		cursor: cur,
		ansiColors,
	}).catch(() => {});
}

/**
 * PoC: Keyboard listener + layout sync for native wgpu terminal.
 * Renders a transparent div that captures input and reports its
 * position/size to the Rust backend for NSView positioning.
 */
export default function NativeTerminalInput() {
	const containerRef = useRef<HTMLDivElement>(null);
	const terminalTheme = useTerminalTheme();
	const fontFamily = useTerminalSettingsStore((s) => s.fontFamily);
	const fontSize = useTerminalSettingsStore((s) => s.fontSize);

	// Report container bounds to Rust so NSView can be positioned correctly
	const syncLayout = useCallback(() => {
		const el = containerRef.current;
		if (!el) return;
		const rect = el.getBoundingClientRect();
		// macOS NSView coordinates: origin at bottom-left of window.
		// WebView rect is top-left origin. Convert:
		const windowHeight = window.innerHeight;
		const x = rect.left;
		const y = windowHeight - rect.bottom; // flip Y for NSView
		invoke("resize_native_terminal", {
			x,
			y,
			width: rect.width,
			height: rect.height,
		}).catch(() => {});
	}, []);

	// Observe container resize and visibility
	useEffect(() => {
		const el = containerRef.current;
		if (!el) return;

		const observer = new ResizeObserver(() => syncLayout());
		observer.observe(el);
		window.addEventListener("resize", syncLayout);

		// Show native view when mounted
		invoke("set_native_terminal_visible", { visible: true }).catch(() => {});
		requestAnimationFrame(syncLayout);

		return () => {
			observer.disconnect();
			window.removeEventListener("resize", syncLayout);
			// Hide native view when unmounted or tab switches away
			invoke("set_native_terminal_visible", { visible: false }).catch(
				() => {},
			);
		};
	}, [syncLayout]);

	// Sync terminal theme to native renderer
	useEffect(() => {
		sendThemeToNative(terminalTheme);
	}, [terminalTheme]);

	// Sync font settings to native renderer
	useEffect(() => {
		invoke("set_native_terminal_font", {
			family: fontFamily,
			size: fontSize,
		}).catch(() => {});
	}, [fontFamily, fontSize]);

	// Keyboard input forwarding
	useEffect(() => {
		const handler = (e: KeyboardEvent) => {
			if (
				e.key === "Meta" ||
				e.key === "Control" ||
				e.key === "Alt" ||
				e.key === "Shift"
			)
				return;

			// Ctrl+letter → send as control character (e.g. Ctrl+C = 0x03)
			if (e.ctrlKey && !e.metaKey) {
				if (e.key.length === 1 && /[a-z]/i.test(e.key)) {
					e.preventDefault();
					const charCode = e.key.toLowerCase().charCodeAt(0) - 96;
					invoke("write_to_native_terminal", {
						data: String.fromCharCode(charCode),
					});
				}
				return;
			}

			// Cmd+key — let the system handle (copy, paste, etc.)
			if (e.metaKey) return;

			// Alt+key → send ESC + key (terminal meta mode)
			if (e.altKey && e.key.length === 1) {
				e.preventDefault();
				invoke("write_to_native_terminal", {
					data: "\x1b" + e.key,
				});
				return;
			}

			e.preventDefault();

			const data = KEY_MAP[e.key] ?? (e.key.length === 1 ? e.key : null);
			if (!data) return;

			invoke("write_to_native_terminal", { data });
		};

		window.addEventListener("keydown", handler);
		return () => window.removeEventListener("keydown", handler);
	}, []);

	// This div marks the area where the native terminal should appear.
	// It's transparent — the actual rendering is done by wgpu underneath.
	return (
		<div
			ref={containerRef}
			style={{
				position: "absolute",
				inset: 0,
				pointerEvents: "none",
			}}
		/>
	);
}
