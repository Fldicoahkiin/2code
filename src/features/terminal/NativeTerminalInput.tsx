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
 * Renders a focusable div that captures input and reports its
 * position/size to the Rust backend for NSView positioning.
 */
export default function NativeTerminalInput() {
	const containerRef = useRef<HTMLDivElement>(null);
	const terminalTheme = useTerminalTheme();
	const fontFamily = useTerminalSettingsStore((s) => s.fontFamily);
	const fontSize = useTerminalSettingsStore((s) => s.fontSize);

	// Send the current container bounds to the native renderer.
	// AppKit's contentView uses Y-up coords, so we flip from WebView Y-down.
	// `scale` (devicePixelRatio) lets the wgpu surface render at physical
	// pixel resolution — crisp on Retina.
	const sendLayoutNow = useCallback(() => {
		const el = containerRef.current;
		if (!el) return Promise.resolve();
		const rect = el.getBoundingClientRect();
		return invoke("resize_native_terminal", {
			x: rect.left,
			y: window.innerHeight - rect.bottom,
			width: rect.width,
			height: rect.height,
			scale: window.devicePixelRatio || 1,
		}).catch(() => {});
	}, []);

	// Coalesce burst-y resize signals (window drag, sidebar animation, etc.)
	// into one IPC per animation frame.
	const rafHandleRef = useRef<number | null>(null);
	const scheduleSync = useCallback(() => {
		if (rafHandleRef.current !== null) return;
		rafHandleRef.current = requestAnimationFrame(() => {
			rafHandleRef.current = null;
			sendLayoutNow();
		});
	}, [sendLayoutNow]);

	// Observe container resize, window resize, and DPR changes
	useEffect(() => {
		const el = containerRef.current;
		if (!el) return;

		const observer = new ResizeObserver(scheduleSync);
		observer.observe(el);
		window.addEventListener("resize", scheduleSync);

		// Re-sync when dragging between displays with different scale factors.
		const dprMedia = window.matchMedia(
			`(resolution: ${window.devicePixelRatio}dppx)`,
		);
		dprMedia.addEventListener("change", scheduleSync);

		// Push first layout *before* unhiding so the user never sees the
		// stale 1×1 surface that was created at app startup.
		sendLayoutNow().then(() => {
			invoke("set_native_terminal_visible", { visible: true }).catch(
				() => {},
			);
		});

		el.focus();

		return () => {
			if (rafHandleRef.current !== null) {
				cancelAnimationFrame(rafHandleRef.current);
				rafHandleRef.current = null;
			}
			observer.disconnect();
			window.removeEventListener("resize", scheduleSync);
			dprMedia.removeEventListener("change", scheduleSync);
			invoke("set_native_terminal_visible", { visible: false }).catch(
				() => {},
			);
		};
	}, [scheduleSync, sendLayoutNow]);

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

	// Keyboard input forwarding via DOM element listener.
	// Window-level listeners would fire on every NativeTerminalInput instance
	// mounted by TerminalLayer (one per profile), multiplying keystrokes by
	// the number of loaded profiles. Element-level listeners only fire when
	// the div has focus, which DOM enforces — one event = one PTY write.
	useEffect(() => {
		const el = containerRef.current;
		if (!el) return;

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

		el.addEventListener("keydown", handler);
		return () => el.removeEventListener("keydown", handler);
	}, []);

	// Wheel listener also on the element — same rationale.
	useEffect(() => {
		const el = containerRef.current;
		if (!el) return;

		const handler = (e: WheelEvent) => {
			const lines = Math.round(e.deltaY / 30) || (e.deltaY > 0 ? 1 : -1);
			const seq = lines > 0 ? "\x1b[B" : "\x1b[A";
			const count = Math.abs(lines);
			const data = seq.repeat(Math.min(count, 10));
			invoke("write_to_native_terminal", { data });
		};

		el.addEventListener("wheel", handler, { passive: true });
		return () => el.removeEventListener("wheel", handler);
	}, []);

	return (
		<div
			ref={containerRef}
			tabIndex={0}
			onMouseDown={(e) => {
				// Click to focus so element-level key listeners receive events
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
