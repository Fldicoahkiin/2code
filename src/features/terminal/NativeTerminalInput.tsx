import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
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
 * Renders a focusable div that captures input and reports its
 * position/size to the Rust backend for NSView positioning.
 */
export default function NativeTerminalInput() {
	const containerRef = useRef<HTMLDivElement>(null);
	const [focused, setFocused] = useState(true);
	const terminalTheme = useTerminalTheme();
	const fontFamily = useTerminalSettingsStore((s) => s.fontFamily);
	const fontSize = useTerminalSettingsStore((s) => s.fontSize);

	// Report container bounds to Rust so NSView can be positioned correctly.
	// AppKit's contentView uses Y-up coordinates, so we flip from the WebView's
	// Y-down origin. `scale` is forwarded so the wgpu surface is configured at
	// physical pixel resolution (crisp on Retina).
	const syncLayout = useCallback(() => {
		const el = containerRef.current;
		if (!el) return;
		const rect = el.getBoundingClientRect();
		const windowHeight = window.innerHeight;
		invoke("resize_native_terminal", {
			x: rect.left,
			y: windowHeight - rect.bottom,
			width: rect.width,
			height: rect.height,
			scale: window.devicePixelRatio || 1,
		}).catch(() => {});
	}, []);

	// Observe container resize and visibility
	useEffect(() => {
		const el = containerRef.current;
		if (!el) return;

		const observer = new ResizeObserver(() => syncLayout());
		observer.observe(el);
		window.addEventListener("resize", syncLayout);

		invoke("set_native_terminal_visible", { visible: true }).catch(() => {});
		requestAnimationFrame(syncLayout);

		// Auto-focus on mount
		el.focus();

		return () => {
			observer.disconnect();
			window.removeEventListener("resize", syncLayout);
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

	// Keyboard input forwarding (only when focused)
	useEffect(() => {
		if (!focused) return;

		const handler = (e: KeyboardEvent) => {
			if (
				e.key === "Meta" ||
				e.key === "Control" ||
				e.key === "Alt" ||
				e.key === "Shift"
			)
				return;

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

			// Cmd+V → paste from clipboard (with bracketed paste support)
			if (e.metaKey && e.key === "v") {
				e.preventDefault();
				navigator.clipboard.readText().then((text) => {
					if (text) {
						invoke("paste_to_native_terminal", { text });
					}
				});
				return;
			}

			if (e.metaKey) return;

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
	}, [focused]);

	// Mouse wheel forwarding (only when focused)
	useEffect(() => {
		if (!focused) return;

		const handler = (e: WheelEvent) => {
			const lines = Math.round(e.deltaY / 30) || (e.deltaY > 0 ? 1 : -1);
			const seq = lines > 0 ? "\x1b[B" : "\x1b[A";
			const count = Math.abs(lines);
			const data = seq.repeat(Math.min(count, 10));
			invoke("write_to_native_terminal", { data });
		};

		window.addEventListener("wheel", handler, { passive: true });
		return () => window.removeEventListener("wheel", handler);
	}, [focused]);

	return (
		<div
			ref={containerRef}
			tabIndex={0}
			onFocus={() => setFocused(true)}
			onBlur={() => setFocused(false)}
			onMouseDown={(e) => {
				// Click to focus the terminal area
				e.currentTarget.focus();
			}}
			style={{
				position: "absolute",
				inset: 0,
				outline: "none",
				cursor: "text",
			}}
		/>
	);
}
