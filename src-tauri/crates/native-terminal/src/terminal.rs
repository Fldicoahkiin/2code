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
    pub fn new(cols: u16, rows: u16) -> Self {
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
            cell_width: 8,
            cell_height: 18,
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

    /// Send raw input bytes to the PTY.
    pub fn write(&self, data: &[u8]) {
        let _ = self.notifier.0.send(Msg::Input(data.to_vec().into()));
    }

    /// Lock the terminal and extract visible grid as colored spans with cursor.
    pub fn grid_content(&self) -> GridContent {
        let term = self.term.lock();
        let cols = term.grid().columns();
        let lines = term.grid().screen_lines();
        let content = term.renderable_content();
        let colors = content.colors;
        let cursor_row = content.cursor.point.line.0 as usize;
        let cursor_col = content.cursor.point.column.0;

        struct CellData {
            c: char,
            r: u8,
            g: u8,
            b: u8,
        }

        let mut grid: Vec<Vec<CellData>> = (0..lines)
            .map(|_| {
                (0..cols)
                    .map(|_| CellData { c: ' ', r: 220, g: 220, b: 220 })
                    .collect()
            })
            .collect();

        for cell in content.display_iter {
            let row = cell.point.line.0 as usize;
            let col = cell.point.column.0;
            if row < lines && col < cols {
                let c = if cell.c == '\0' { ' ' } else { cell.c };
                let (r, g, b) = ansi_to_rgb(cell.fg, colors);
                grid[row][col] = CellData { c, r, g, b };
            }
        }

        let mut spans = Vec::new();
        for row in &grid {
            let mut current_text = String::new();
            let mut current_r = 220u8;
            let mut current_g = 220u8;
            let mut current_b = 220u8;
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
                r: 220,
                g: 220,
                b: 220,
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

fn ansi_to_rgb(color: AnsiColor, colors: &alacritty_terminal::term::color::Colors) -> (u8, u8, u8) {
    match color {
        AnsiColor::Named(named) => named_to_rgb(named),
        AnsiColor::Spec(rgb) => (rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(idx) => {
            if let Some(c) = colors[idx as usize] {
                (c.r, c.g, c.b)
            } else if idx < 16 {
                // Standard 16 colors
                named_to_rgb(match idx {
                    0 => NamedColor::Black,
                    1 => NamedColor::Red,
                    2 => NamedColor::Green,
                    3 => NamedColor::Yellow,
                    4 => NamedColor::Blue,
                    5 => NamedColor::Magenta,
                    6 => NamedColor::Cyan,
                    7 => NamedColor::White,
                    8 => NamedColor::BrightBlack,
                    9 => NamedColor::BrightRed,
                    10 => NamedColor::BrightGreen,
                    11 => NamedColor::BrightYellow,
                    12 => NamedColor::BrightBlue,
                    13 => NamedColor::BrightMagenta,
                    14 => NamedColor::BrightCyan,
                    15 => NamedColor::BrightWhite,
                    _ => NamedColor::White,
                })
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

fn named_to_rgb(c: NamedColor) -> (u8, u8, u8) {
    match c {
        NamedColor::Black => (30, 30, 30),
        NamedColor::Red => (204, 60, 60),
        NamedColor::Green => (80, 200, 80),
        NamedColor::Yellow => (220, 200, 60),
        NamedColor::Blue => (60, 120, 220),
        NamedColor::Magenta => (180, 80, 200),
        NamedColor::Cyan => (80, 200, 200),
        NamedColor::White => (220, 220, 220),
        NamedColor::BrightBlack => (100, 100, 100),
        NamedColor::BrightRed => (255, 100, 100),
        NamedColor::BrightGreen => (100, 255, 100),
        NamedColor::BrightYellow => (255, 255, 100),
        NamedColor::BrightBlue => (100, 150, 255),
        NamedColor::BrightMagenta => (255, 100, 255),
        NamedColor::BrightCyan => (100, 255, 255),
        NamedColor::BrightWhite => (255, 255, 255),
        _ => (220, 220, 220),
    }
}
