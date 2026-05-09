use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{self, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor};
use alacritty_terminal::sync::FairMutex;
use std::sync::Arc;
use std::thread::JoinHandle;

/// A span of text with a foreground color.
pub struct ColoredSpan {
    pub text: String,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Terminal grid content with cursor position.
pub struct GridContent {
    pub spans: Vec<ColoredSpan>,
    pub cursor_row: usize,
    pub cursor_col: usize,
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
    #[allow(dead_code)]
    io_thread: JoinHandle<(EventLoop<tty::Pty, JsonEventListener>, alacritty_terminal::event_loop::State)>,
    cols: u16,
    rows: u16,
}

impl TerminalBackend {
    pub fn new(cols: u16, rows: u16, cell_width: u16, cell_height: u16) -> Self {
        let term_size = TermSize::new(cols as usize, rows as usize);
        let term = Term::new(term::Config::default(), &term_size, JsonEventListener);
        let term = Arc::new(FairMutex::new(term));

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());

        let pty_config = tty::Options {
            shell: Some(tty::Shell::new(shell, vec![])),
            drain_on_exit: false,
            ..Default::default()
        };

        let window_size = WindowSize {
            num_cols: cols,
            num_lines: rows,
            cell_width,
            cell_height,
        };

        let pty = tty::new(&pty_config, window_size, 0).expect("failed to create PTY");

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
            io_thread,
            cols,
            rows,
        }
    }

    /// Resize PTY to new dimensions.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
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

    /// Lock the terminal and extract visible grid as colored spans with cursor.
    pub fn grid_content(&self, theme: &crate::theme::TerminalTheme) -> GridContent {
        let term = self.term.lock();
        let cols = term.grid().columns();
        let lines = term.grid().screen_lines();
        let content = term.renderable_content();
        let colors = content.colors;
        let cursor_row = content.cursor.point.line.0 as usize;
        let cursor_col = content.cursor.point.column.0;
        let (fg_r, fg_g, fg_b) = theme.foreground;

        struct CellData {
            c: char,
            r: u8,
            g: u8,
            b: u8,
        }

        let mut grid: Vec<Vec<CellData>> = (0..lines)
            .map(|_| {
                (0..cols)
                    .map(|_| CellData { c: ' ', r: fg_r, g: fg_g, b: fg_b })
                    .collect()
            })
            .collect();

        for cell in content.display_iter {
            let row = cell.point.line.0 as usize;
            let col = cell.point.column.0;
            if row < lines && col < cols {
                let c = if cell.c == '\0' { ' ' } else { cell.c };
                let (r, g, b) = ansi_to_rgb(cell.fg, colors, theme);
                grid[row][col] = CellData { c, r, g, b };
            }
        }

        let mut spans = Vec::new();
        for row in &grid {
            let mut current_text = String::new();
            let mut current_r = fg_r;
            let mut current_g = fg_g;
            let mut current_b = fg_b;
            let mut first = true;

            for cell in row {
                if first {
                    current_r = cell.r;
                    current_g = cell.g;
                    current_b = cell.b;
                    first = false;
                }

                if cell.r == current_r && cell.g == current_g && cell.b == current_b {
                    current_text.push(cell.c);
                } else {
                    if !current_text.is_empty() {
                        spans.push(ColoredSpan {
                            text: current_text.clone(),
                            r: current_r,
                            g: current_g,
                            b: current_b,
                        });
                    }
                    current_text.clear();
                    current_text.push(cell.c);
                    current_r = cell.r;
                    current_g = cell.g;
                    current_b = cell.b;
                }
            }
            if !current_text.is_empty() {
                spans.push(ColoredSpan {
                    text: current_text,
                    r: current_r,
                    g: current_g,
                    b: current_b,
                });
            }
            spans.push(ColoredSpan {
                text: "\n".to_string(),
                r: fg_r,
                g: fg_g,
                b: fg_b,
            });
        }
        GridContent {
            spans,
            cursor_row,
            cursor_col,
        }
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }
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

fn named_to_rgb(c: NamedColor, theme: &crate::theme::TerminalTheme) -> (u8, u8, u8) {
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
