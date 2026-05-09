import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef } from "react";
import { useTerminalTheme } from "@/features/terminal/hooks";
import type { ITheme } from "@xterm/xterm";

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
			invoke("set_native_terminal_visible", { visible: false }).catch(() => {});
		};
	}, [syncLayout]);

	// Sync terminal theme to native renderer
	useEffect(() => {
		sendThemeToNative(terminalTheme);
	}, [terminalTheme]);

	// Keyboard input forwarding
	useEffect(() => {
		const handler = (e: KeyboardEvent) => {
			if (
				e.key === "Meta"
				|| e.key === "Control"
				|| e.key === "Alt"
				|| e.key === "Shift"
			)
				return;
			if (e.metaKey || e.ctrlKey) {
				if (e.ctrlKey && "cdlz".includes(e.key.toLowerCase())) {
					e.preventDefault();
					const charCode = e.key.toLowerCase().charCodeAt(0) - 96;
					invoke("write_to_native_terminal", {
						data: String.fromCharCode(charCode),
					});
					return;
				}
				return;
			}

			e.preventDefault();

			let data: string;
			if (e.key === "Enter") data = "\r";
			else if (e.key === "Backspace") data = "\x7f";
			else if (e.key === "Tab") data = "\t";
			else if (e.key === "Escape") data = "\x1b";
			else if (e.key === "ArrowUp") data = "\x1b[A";
			else if (e.key === "ArrowDown") data = "\x1b[B";
			else if (e.key === "ArrowRight") data = "\x1b[C";
			else if (e.key === "ArrowLeft") data = "\x1b[D";
			else if (e.key.length === 1) data = e.key;
			else return;

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
