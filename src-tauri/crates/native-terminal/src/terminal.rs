use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{self, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor};
use std::sync::Arc;
use std::thread::JoinHandle;

/// A span of text with foreground/background colors and style attributes.
pub struct ColoredSpan {
	pub text: String,
	/// Foreground RGB.
	pub r: u8,
	pub g: u8,
	pub b: u8,
	/// Background RGB, None means default (transparent).
	pub bg: Option<(u8, u8, u8)>,
	pub bold: bool,
	pub italic: bool,
	pub underline: bool,
	pub strikeout: bool,
	/// Number of grid columns this span occupies. May exceed text.chars().count()
	/// when the span contains wide CJK chars (spacer cells contribute to width
	/// but not to text).
	pub cols: usize,
}

/// Visual shape of the terminal cursor.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorShape {
	Block,
	Underline,
	Beam,
	Hidden,
}

/// Terminal grid content with cursor position.
pub struct GridContent {
	pub spans: Vec<ColoredSpan>,
	pub cursor_row: usize,
	pub cursor_col: usize,
	pub cursor_shape: CursorShape,
}

#[derive(Clone)]
struct JsonEventListener;

impl EventListener for JsonEventListener {
	fn send_event(&self, _event: Event) {}
}

/// Wraps alacritty_terminal's Term + PTY event loop.
pub struct TerminalBackend {
	term: Arc<FairMutex<Term<JsonEventListener>>>,
	notifier: Notifier,
	io_thread: Option<
		JoinHandle<(
			EventLoop<tty::Pty, JsonEventListener>,
			alacritty_terminal::event_loop::State,
		)>,
	>,
	cols: u16,
	rows: u16,
}

impl TerminalBackend {
	pub fn new(
		cols: u16,
		rows: u16,
		cell_width: u16,
		cell_height: u16,
	) -> Self {
		let term_size = TermSize::new(cols as usize, rows as usize);
		let term =
			Term::new(term::Config::default(), &term_size, JsonEventListener);
		let term = Arc::new(FairMutex::new(term));

		let shell =
			std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());

		// Tell programs running inside the PTY what the terminal supports.
		// TERM=xterm-256color is the broadest-compatible value; COLORTERM
		// signals 24-bit color (truecolor) support to apps like vim/nvim.
		let mut env = std::collections::HashMap::new();
		env.insert("TERM".into(), "xterm-256color".into());
		env.insert("COLORTERM".into(), "truecolor".into());
		if std::env::var_os("LANG").is_none() {
			env.insert("LANG".into(), "en_US.UTF-8".into());
		}

		let pty_config = tty::Options {
			shell: Some(tty::Shell::new(shell, vec![])),
			drain_on_exit: false,
			env,
			..Default::default()
		};

		let window_size = WindowSize {
			num_cols: cols,
			num_lines: rows,
			cell_width,
			cell_height,
		};

		let pty = tty::new(&pty_config, window_size, 0)
			.expect("failed to create PTY");

		let event_loop = EventLoop::new(
			term.clone(),
			JsonEventListener,
			pty,
			pty_config.drain_on_exit,
			false,
		)
		.expect("failed to create event loop");

		let notifier = Notifier(event_loop.channel());
		let io_thread = event_loop.spawn();

		log::info!("native-terminal: PTY spawned ({cols}x{rows})");

		Self {
			term,
			notifier,
			io_thread: Some(io_thread),
			cols,
			rows,
		}
	}

	/// Resize PTY to new dimensions.
	pub fn resize(
		&mut self,
		cols: u16,
		rows: u16,
		cell_width: u16,
		cell_height: u16,
	) {
		if cols == self.cols && rows == self.rows {
			return;
		}
		self.cols = cols;
		self.rows = rows;
		let window_size = WindowSize {
			num_cols: cols,
			num_lines: rows,
			cell_width,
			cell_height,
		};
		let _ = self.notifier.0.send(Msg::Resize(window_size));
		// Also resize the terminal grid
		let mut term = self.term.lock();
		term.resize(TermSize::new(cols as usize, rows as usize));
		log::info!("native-terminal: PTY resized to {cols}x{rows}");
	}

	/// Send raw input bytes to the PTY.
	pub fn write(&self, data: &[u8]) {
		let _ = self.notifier.0.send(Msg::Input(data.to_vec().into()));
	}

	/// Signal the io_thread to shut down cleanly.
	fn shutdown(&self) {
		let _ = self.notifier.0.send(Msg::Shutdown);
	}

	/// Check if bracketed paste mode is enabled by the running program.
	pub fn bracketed_paste_enabled(&self) -> bool {
		use alacritty_terminal::term::TermMode;
		let term = self.term.lock();
		term.mode().contains(TermMode::BRACKETED_PASTE)
	}

	/// Lock the terminal and extract visible grid as colored spans with cursor.
	pub fn grid_content(
		&self,
		theme: &crate::theme::TerminalTheme,
	) -> GridContent {
		let term = self.term.lock();
		let cols = term.grid().columns();
		let lines = term.grid().screen_lines();
		let content = term.renderable_content();
		let colors = content.colors;
		let cursor_row = content.cursor.point.line.0 as usize;
		let cursor_col = content.cursor.point.column.0;
		let cursor_shape = match content.cursor.shape {
			alacritty_terminal::vte::ansi::CursorShape::Block => {
				CursorShape::Block
			}
			alacritty_terminal::vte::ansi::CursorShape::Underline => {
				CursorShape::Underline
			}
			alacritty_terminal::vte::ansi::CursorShape::Beam => {
				CursorShape::Beam
			}
			alacritty_terminal::vte::ansi::CursorShape::HollowBlock => {
				CursorShape::Block
			}
			alacritty_terminal::vte::ansi::CursorShape::Hidden => {
				CursorShape::Hidden
			}
		};
		let (fg_r, fg_g, fg_b) = theme.foreground;
		let bg_default = theme.background;

		use alacritty_terminal::term::cell::Flags;

		#[derive(Clone, Copy, PartialEq)]
		struct CellAttrs {
			fg: (u8, u8, u8),
			bg: Option<(u8, u8, u8)>,
			bold: bool,
			italic: bool,
			underline: bool,
			strikeout: bool,
		}

		struct CellData {
			c: char,
			attrs: CellAttrs,
			/// True for the right-half cell of a wide CJK char. Should not
			/// contribute to span text but still keeps its grid column.
			spacer: bool,
		}

		let default_attrs = CellAttrs {
			fg: (fg_r, fg_g, fg_b),
			bg: None,
			bold: false,
			italic: false,
			underline: false,
			strikeout: false,
		};

		let mut grid: Vec<Vec<CellData>> = (0..lines)
			.map(|_| {
				(0..cols)
					.map(|_| CellData {
						c: ' ',
						attrs: default_attrs,
						spacer: false,
					})
					.collect()
			})
			.collect();

		for cell in content.display_iter {
			let row = cell.point.line.0 as usize;
			let col = cell.point.column.0;
			if row < lines && col < cols {
				let flags = cell.flags;

				// The right-half cell of a wide CJK char — wide char already
				// covers this column visually. Mark as spacer so it does not
				// contribute text but still occupies the grid column.
				let is_spacer = flags.intersects(
					Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER,
				);

				// SGR 8 hidden/conceal collapses to a space, same as the
				// null-char placeholder. Either way, cell width and background
				// are preserved but no glyph is drawn.
				let c = if flags.contains(Flags::HIDDEN) || cell.c == '\0' {
					' '
				} else {
					cell.c
				};
				let mut fg = ansi_to_rgb(cell.fg, colors, theme);
				let mut bg_rgb = ansi_to_rgb(cell.bg, colors, theme);

				// SGR 7: inverse video — swap fg/bg. Must resolve both colors
				// to concrete values first so default colors participate in swap.
				if flags.contains(Flags::INVERSE) {
					std::mem::swap(&mut fg, &mut bg_rgb);
					// After swap, bg holds the original fg (must render),
					// and fg holds the original bg. Force bg to be Some.
				}

				let bg =
					if flags.contains(Flags::INVERSE) || bg_rgb != bg_default {
						Some(bg_rgb)
					} else {
						None
					};

				// SGR 2: dim — multiply fg by 2/3 (~0.667) for visible darkening
				// that still preserves enough contrast against most backgrounds.
				let fg = if flags.contains(Flags::DIM) {
					dim_color(fg)
				} else {
					fg
				};

				let attrs = CellAttrs {
					fg,
					bg,
					bold: flags.contains(Flags::BOLD),
					italic: flags.contains(Flags::ITALIC),
					underline: flags.intersects(Flags::ALL_UNDERLINES),
					strikeout: flags.contains(Flags::STRIKEOUT),
				};
				grid[row][col] = CellData {
					c,
					attrs,
					spacer: is_spacer,
				};
			}
		}

		let mut spans = Vec::new();
		for (row_idx, row) in grid.iter().enumerate() {
			// Find the last cell with visible content. Trailing cells that
			// are plain space + default attrs contribute nothing to rendering,
			// so we skip them to avoid pointless text shaping. The cursor
			// row must extend to at least cursor_col so the cursor renders.
			let last_meaningful = row.iter().rposition(|c| {
				!c.spacer && (c.c != ' ' || c.attrs != default_attrs)
			});
			let mut effective_len = last_meaningful.map_or(0, |i| i + 1);
			if row_idx == cursor_row {
				effective_len = effective_len.max(cursor_col + 1);
			}

			let mut current_text = String::new();
			let mut current_cols = 0usize;
			let mut current = default_attrs;
			let mut first = true;

			for cell in &row[..effective_len] {
				// Spacer cells (right-half of wide CJK char) take one column
				// but contribute no text. They extend the previous span's width.
				if cell.spacer {
					current_cols += 1;
					continue;
				}

				if first {
					current = cell.attrs;
					first = false;
				}

				if cell.attrs == current {
					current_text.push(cell.c);
					current_cols += 1;
				} else {
					if !current_text.is_empty() {
						spans.push(ColoredSpan {
							text: current_text.clone(),
							r: current.fg.0,
							g: current.fg.1,
							b: current.fg.2,
							bg: current.bg,
							bold: current.bold,
							italic: current.italic,
							underline: current.underline,
							strikeout: current.strikeout,
							cols: current_cols,
						});
					}
					current_text.clear();
					current_text.push(cell.c);
					current_cols = 1;
					current = cell.attrs;
				}
			}
			if !current_text.is_empty() {
				spans.push(ColoredSpan {
					text: current_text,
					r: current.fg.0,
					g: current.fg.1,
					b: current.fg.2,
					bg: current.bg,
					bold: current.bold,
					italic: current.italic,
					underline: current.underline,
					strikeout: current.strikeout,
					cols: current_cols,
				});
			}
			spans.push(ColoredSpan {
				text: "\n".to_string(),
				r: fg_r,
				g: fg_g,
				b: fg_b,
				bg: None,
				bold: false,
				italic: false,
				underline: false,
				strikeout: false,
				cols: 0,
			});
		}
		GridContent {
			spans,
			cursor_row,
			cursor_col,
			cursor_shape,
		}
	}

	pub fn cols(&self) -> u16 {
		self.cols
	}

	pub fn rows(&self) -> u16 {
		self.rows
	}
}

impl Drop for TerminalBackend {
	fn drop(&mut self) {
		self.shutdown();
		if let Some(handle) = self.io_thread.take() {
			// Best-effort wait so PTY fd closes before process exits.
			let _ = handle.join();
		}
		log::info!("native-terminal: PTY shut down");
	}
}

/// Darken an RGB color by ~33% — used for SGR 2 (DIM) attribute.
fn dim_color((r, g, b): (u8, u8, u8)) -> (u8, u8, u8) {
	(
		(r as u16 * 2 / 3) as u8,
		(g as u16 * 2 / 3) as u8,
		(b as u16 * 2 / 3) as u8,
	)
}

fn ansi_to_rgb(
	color: AnsiColor,
	colors: &alacritty_terminal::term::color::Colors,
	theme: &crate::theme::TerminalTheme,
) -> (u8, u8, u8) {
	match color {
		AnsiColor::Named(named) => named_to_rgb(named, theme),
		AnsiColor::Spec(rgb) => (rgb.r, rgb.g, rgb.b),
		AnsiColor::Indexed(idx) => {
			if let Some(c) = colors[idx as usize] {
				(c.r, c.g, c.b)
			} else if idx < 16 {
				theme.named_color(idx as usize)
			} else if idx < 232 {
				// 216 color cube (6x6x6)
				let idx = idx - 16;
				let r = (idx / 36) * 51;
				let g = ((idx % 36) / 6) * 51;
				let b = (idx % 6) * 51;
				(r, g, b)
			} else {
				// Grayscale ramp
				let v = 8 + (idx - 232) * 10;
				(v, v, v)
			}
		}
	}
}

fn named_to_rgb(
	c: NamedColor,
	theme: &crate::theme::TerminalTheme,
) -> (u8, u8, u8) {
	let idx = match c {
		NamedColor::Black => 0,
		NamedColor::Red => 1,
		NamedColor::Green => 2,
		NamedColor::Yellow => 3,
		NamedColor::Blue => 4,
		NamedColor::Magenta => 5,
		NamedColor::Cyan => 6,
		NamedColor::White => 7,
		NamedColor::BrightBlack => 8,
		NamedColor::BrightRed => 9,
		NamedColor::BrightGreen => 10,
		NamedColor::BrightYellow => 11,
		NamedColor::BrightBlue => 12,
		NamedColor::BrightMagenta => 13,
		NamedColor::BrightCyan => 14,
		NamedColor::BrightWhite => 15,
		NamedColor::Foreground => return theme.foreground,
		NamedColor::Background => return theme.background,
		NamedColor::Cursor => return theme.cursor,
		_ => return theme.foreground,
	};
	theme.named_color(idx)
}
