use crate::cursor::{CursorRect, RectRenderer};
use crate::terminal::TerminalBackend;
use crate::theme::TerminalTheme;
use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Default font size in points.
const DEFAULT_FONT_SIZE: f32 = 14.0;
/// Padding around the text area (pixels).
const PADDING: f32 = 8.0;

/// Estimate monospace cell dimensions from font size.
/// Width ≈ font_size × 0.6 (standard monospace ratio), height ≈ font_size × 1.3.
fn cell_dimensions(font_size: f32) -> (u32, u32) {
    let w = (font_size * 0.6).ceil() as u32;
    let h = (font_size * 1.3).ceil() as u32;
    (w.max(1), h.max(1))
}

pub struct TerminalSurface {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    font_system: FontSystem,
    swash_cache: SwashCache,
    text_atlas: TextAtlas,
    text_renderer: TextRenderer,
    viewport: Viewport,
    text_buffer: Buffer,
    backend: Option<TerminalBackend>,
    frame_count: u64,
    theme: TerminalTheme,
    font_size: f32,
    font_family: String,
    cell_width: u32,
    cell_height: u32,
    rect_renderer: RectRenderer,
    /// Cursor position in grid coordinates, updated each frame.
    cursor_pos: Option<(usize, usize)>,
}

impl TerminalSurface {
    /// # Safety
    /// The raw handles must be valid and the view must outlive this surface.
    pub unsafe fn new(
        raw_window: RawWindowHandle,
        raw_display: RawDisplayHandle,
        width: u32,
        height: u32,
    ) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());

        let surface = unsafe {
            instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(raw_display),
                    raw_window_handle: raw_window,
                })
                .expect("failed to create wgpu surface")
        };

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("no suitable GPU adapter found");

        log::info!("native-terminal: GPU adapter: {}", adapter.get_info().name);

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("native-terminal"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                    .using_resolution(adapter.limits()),
                ..Default::default()
            },
        ))
        .expect("failed to create GPU device");

        let surface_caps = surface.get_capabilities(&adapter);
        let format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Text rendering setup
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let mut text_atlas = TextAtlas::new(&device, &queue, &cache, format);
        let text_renderer = TextRenderer::new(
            &mut text_atlas,
            &device,
            wgpu::MultisampleState::default(),
            None,
        );
        let viewport = Viewport::new(&device, &cache);

        let font_size = DEFAULT_FONT_SIZE;
        let (cell_width, cell_height) = cell_dimensions(font_size);
        let line_height = cell_height as f32;
        let mut text_buffer = Buffer::new(&mut font_system, Metrics::new(font_size, line_height));
        text_buffer.set_size(&mut font_system, Some(width as f32), Some(height as f32));

        // Spawn real PTY terminal
        let cols = (width / cell_width).max(1) as u16;
        let rows = (height / cell_height).max(1) as u16;
        let backend = TerminalBackend::new(cols, rows, cell_width as u16, cell_height as u16);
        let rect_renderer = RectRenderer::new(&device, format);

        Self {
            surface,
            device,
            queue,
            config,
            font_system,
            swash_cache,
            text_atlas,
            text_renderer,
            viewport,
            text_buffer,
            backend: Some(backend),
            frame_count: 0,
            theme: TerminalTheme::default(),
            font_size,
            font_family: "JetBrains Mono".to_string(),
            cell_width,
            cell_height,
            rect_renderer,
            cursor_pos: None,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.text_buffer.set_size(
            &mut self.font_system,
            Some(width as f32),
            Some(height as f32),
        );
        self.text_buffer.shape_until_scroll(&mut self.font_system, false);

        // Resize PTY grid to match new pixel dimensions
        let cols = (width / self.cell_width).max(1) as u16;
        let rows = (height / self.cell_height).max(1) as u16;
        if let Some(ref mut backend) = self.backend {
            backend.resize(cols, rows, self.cell_width as u16, self.cell_height as u16);
        }
    }

    /// Apply a new color theme.
    pub fn set_theme(&mut self, theme: TerminalTheme) {
        self.theme = theme;
    }

    /// Update font family and size. Rebuilds text metrics and resizes PTY grid.
    pub fn set_font(&mut self, family: String, size: f32) {
        let size = size.clamp(10.0, 20.0);
        self.font_family = family;
        self.font_size = size;
        let (cw, ch) = cell_dimensions(size);
        self.cell_width = cw;
        self.cell_height = ch;

        // Rebuild text buffer with new metrics
        self.text_buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(size, ch as f32),
        );
        self.text_buffer.set_size(
            &mut self.font_system,
            Some(self.config.width as f32),
            Some(self.config.height as f32),
        );

        // Resize PTY to match new cell dimensions
        let cols = (self.config.width / cw).max(1) as u16;
        let rows = (self.config.height / ch).max(1) as u16;
        if let Some(ref mut backend) = self.backend {
            backend.resize(cols, rows, self.cell_width as u16, self.cell_height as u16);
        }

        log::info!(
            "native-terminal: font set to {} {}pt (cell {}x{})",
            self.font_family, size, cw, ch
        );
    }

    /// Send input bytes to the PTY.
    pub fn write_to_pty(&self, data: &[u8]) {
        if let Some(ref backend) = self.backend {
            backend.write(data);
        }
    }

    /// Paste text into the PTY, wrapping with bracketed paste sequences
    /// if the running program has enabled bracketed paste mode.
    pub fn paste_to_pty(&self, text: &str) {
        if let Some(ref backend) = self.backend {
            if backend.bracketed_paste_enabled() {
                backend.write(b"\x1b[200~");
                backend.write(text.as_bytes());
                backend.write(b"\x1b[201~");
            } else {
                backend.write(text.as_bytes());
            }
        }
    }

    /// Read terminal grid, update text buffer, render, and return content hash.
    /// The hash can be used for idle detection without a separate grid read.
    pub fn render(&mut self) -> u64 {
        // Background color rectangles: (row, col_start, col_count, r, g, b)
        let mut bg_rects: Vec<(usize, usize, usize, u8, u8, u8)> = Vec::new();

        let content_hash = if let Some(ref backend) = self.backend {
            let content = backend.grid_content(&self.theme);

            // Compute hash from grid content
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for span in &content.spans {
                span.text.hash(&mut hasher);
                span.r.hash(&mut hasher);
                span.g.hash(&mut hasher);
                span.b.hash(&mut hasher);
                span.bg.hash(&mut hasher);
                span.bold.hash(&mut hasher);
                span.italic.hash(&mut hasher);
                span.underline.hash(&mut hasher);
            }
            content.cursor_row.hash(&mut hasher);
            content.cursor_col.hash(&mut hasher);
            let hash = hasher.finish();
            let cursor_row = content.cursor_row;
            let cursor_col = content.cursor_col;

            self.frame_count += 1;
            let cursor_visible = (self.frame_count / 30).is_multiple_of(2);
            self.cursor_pos = if cursor_visible {
                Some((cursor_row, cursor_col))
            } else {
                None
            };

            // Build owned text + span ranges to avoid lifetime issues
            struct SpanRange {
                start: usize,
                end: usize,
                r: u8,
                g: u8,
                b: u8,
                bold: bool,
                italic: bool,
            }

            let mut full_text = String::with_capacity(4096);
            let mut span_ranges: Vec<SpanRange> = Vec::new();
            let mut line = 0usize;
            let mut col = 0usize;

            for span in &content.spans {
                if span.text == "\n" {
                    let start = full_text.len();
                    full_text.push('\n');
                    let (fr, fg, fb) = self.theme.foreground;
                    span_ranges.push(SpanRange {
                        start, end: full_text.len(), r: fr, g: fg, b: fb,
                        bold: false, italic: false,
                    });
                    line += 1;
                    col = 0;
                    continue;
                }

                let span_start_col = col;
                let char_count = span.text.chars().count();
                let span_end_col = col + char_count;
                let is_cursor_line = line == cursor_row && cursor_visible;
                let cursor_in_span = is_cursor_line && cursor_col >= span_start_col && cursor_col < span_end_col;

                // Collect background color rectangle
                if let Some((br, bg_color, bb)) = span.bg {
                    bg_rects.push((line, span_start_col, char_count, br, bg_color, bb));
                }

                let push = |sr: &mut Vec<SpanRange>, start, end, r, g, b, bold, italic| {
                    sr.push(SpanRange { start, end, r, g, b, bold, italic });
                };

                if cursor_in_span {
                    let offset = cursor_col - span_start_col;
                    let chars: Vec<char> = span.text.chars().collect();

                    // Before cursor
                    if offset > 0 {
                        let start = full_text.len();
                        for &c in &chars[..offset] { full_text.push(c); }
                        push(&mut span_ranges, start, full_text.len(),
                             span.r, span.g, span.b, span.bold, span.italic);
                    }

                    // Cursor char (inverted: use background color as text)
                    let start = full_text.len();
                    full_text.push(chars.get(offset).copied().unwrap_or(' '));
                    let (br, bg, bb) = self.theme.background;
                    push(&mut span_ranges, start, full_text.len(), br, bg, bb, false, false);

                    // After cursor
                    if offset + 1 < chars.len() {
                        let start = full_text.len();
                        for &c in &chars[offset + 1..] { full_text.push(c); }
                        push(&mut span_ranges, start, full_text.len(),
                             span.r, span.g, span.b, span.bold, span.italic);
                    }
                } else {
                    let start = full_text.len();
                    full_text.push_str(&span.text);
                    push(&mut span_ranges, start, full_text.len(),
                         span.r, span.g, span.b, span.bold, span.italic);
                }

                col = span_end_col;
            }

            // Build rich text spans from owned string
            let rich: Vec<(&str, Attrs)> = span_ranges
                .iter()
                .map(|sr| {
                    let mut attrs = Attrs::new()
                        .family(Family::Name(&self.font_family))
                        .color(Color::rgb(sr.r, sr.g, sr.b));
                    if sr.bold {
                        attrs = attrs.weight(glyphon::Weight::BOLD);
                    }
                    if sr.italic {
                        attrs = attrs.style(glyphon::Style::Italic);
                    }
                    (&full_text[sr.start..sr.end], attrs)
                })
                .collect();

            self.text_buffer.set_rich_text(
                &mut self.font_system,
                rich,
                &Attrs::new().family(Family::Name(&self.font_family)).color(Color::rgb(
                    self.theme.foreground.0,
                    self.theme.foreground.1,
                    self.theme.foreground.2,
                )),
                Shaping::Advanced,
                None,
            );
            self.text_buffer.shape_until_scroll(&mut self.font_system, false);
            hash
        } else {
            0
        };

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                self.surface.configure(&self.device, &self.config);
                return content_hash;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );

        let text_area = TextArea {
            buffer: &self.text_buffer,
            left: PADDING,
            top: PADDING,
            scale: 1.0,
            bounds: TextBounds {
                left: 0,
                top: 0,
                right: self.config.width as i32,
                bottom: self.config.height as i32,
            },
            default_color: Color::rgb(
                self.theme.foreground.0,
                self.theme.foreground.1,
                self.theme.foreground.2,
            ),
            custom_glyphs: &[],
        };

        self.text_renderer
            .prepare(
                &self.device,
                &self.queue,
                &mut self.font_system,
                &mut self.text_atlas,
                &self.viewport,
                [text_area],
                &mut self.swash_cache,
            )
            .expect("failed to prepare text");

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("native-terminal-encoder"),
            });

        // Pass 1: Clear background
        {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.theme.background.0 as f64 / 255.0,
                            g: self.theme.background.1 as f64 / 255.0,
                            b: self.theme.background.2 as f64 / 255.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }

        // Pass 2: Draw cell backgrounds + cursor block (batched by color)
        {
            let mut all_rects: Vec<CursorRect> = bg_rects
                .iter()
                .map(|&(row, col_start, col_count, r, g, b)| CursorRect {
                    x: PADDING as u32 + col_start as u32 * self.cell_width,
                    y: PADDING as u32 + row as u32 * self.cell_height,
                    width: col_count as u32 * self.cell_width,
                    height: self.cell_height,
                    color: [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0],
                })
                .collect();

            if let Some((row, col)) = self.cursor_pos {
                let (cr, cg, cb) = self.theme.cursor;
                all_rects.push(CursorRect {
                    x: PADDING as u32 + col as u32 * self.cell_width,
                    y: PADDING as u32 + row as u32 * self.cell_height,
                    width: self.cell_width,
                    height: self.cell_height,
                    color: [cr as f32 / 255.0, cg as f32 / 255.0, cb as f32 / 255.0, 1.0],
                });
            }

            self.rect_renderer.render_batch(
                &mut encoder,
                &view,
                &self.queue,
                &self.device,
                &mut all_rects,
            );
        }

        // Pass 3: Render text on top
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            self.text_renderer
                .render(&self.text_atlas, &self.viewport, &mut pass)
                .expect("failed to render text");
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        self.text_atlas.trim();
        content_hash
    }
}
