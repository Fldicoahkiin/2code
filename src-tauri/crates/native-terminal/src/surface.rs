use crate::terminal::TerminalBackend;
use crate::theme::TerminalTheme;
use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

/// Cell dimensions for monospace grid layout (pixels).
const CELL_WIDTH: u32 = 8;
const CELL_HEIGHT: u32 = 18;
/// Padding around the text area (pixels).
const PADDING: f32 = 8.0;

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

        let font_size = 14.0;
        let line_height = CELL_HEIGHT as f32;
        let mut text_buffer = Buffer::new(&mut font_system, Metrics::new(font_size, line_height));
        text_buffer.set_size(&mut font_system, Some(width as f32), Some(height as f32));

        // Spawn real PTY terminal
        let cols = (width / CELL_WIDTH).max(1) as u16;
        let rows = (height / CELL_HEIGHT).max(1) as u16;
        let backend = TerminalBackend::new(cols, rows);

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
        let cols = (width / CELL_WIDTH).max(1) as u16;
        let rows = (height / CELL_HEIGHT).max(1) as u16;
        if let Some(ref mut backend) = self.backend {
            backend.resize(cols, rows);
        }
    }

    /// Apply a new color theme.
    pub fn set_theme(&mut self, theme: TerminalTheme) {
        self.theme = theme;
    }

    /// Quick hash of terminal content for change detection.
    pub fn content_hash(&self) -> u64 {
        if let Some(ref backend) = self.backend {
            let content = backend.grid_content(&self.theme);
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            for span in &content.spans {
                span.text.hash(&mut hasher);
                span.r.hash(&mut hasher);
                span.g.hash(&mut hasher);
                span.b.hash(&mut hasher);
            }
            content.cursor_row.hash(&mut hasher);
            content.cursor_col.hash(&mut hasher);
            hasher.finish()
        } else {
            0
        }
    }

    /// Send input bytes to the PTY.
    pub fn write_to_pty(&self, data: &[u8]) {
        if let Some(ref backend) = self.backend {
            backend.write(data);
        }
    }

    /// Read terminal grid and update text buffer, then render.
    pub fn render_test_frame(&mut self) {
        if let Some(ref backend) = self.backend {
            let content = backend.grid_content(&self.theme);
            let cursor_row = content.cursor_row;
            let cursor_col = content.cursor_col;

            self.frame_count += 1;
            let cursor_visible = (self.frame_count / 30).is_multiple_of(2);

            // Build owned text + span ranges to avoid lifetime issues
            let mut full_text = String::with_capacity(4096);
            let mut span_ranges: Vec<(usize, usize, u8, u8, u8)> = Vec::new(); // (start, end, r, g, b)
            let mut line = 0usize;
            let mut col = 0usize;

            for span in &content.spans {
                if span.text == "\n" {
                    let start = full_text.len();
                    full_text.push('\n');
                    let (fr, fg, fb) = self.theme.foreground;
                    span_ranges.push((start, full_text.len(), fr, fg, fb));
                    line += 1;
                    col = 0;
                    continue;
                }

                let span_start_col = col;
                let span_end_col = col + span.text.chars().count();
                let is_cursor_line = line == cursor_row && cursor_visible;
                let cursor_in_span = is_cursor_line && cursor_col >= span_start_col && cursor_col < span_end_col;

                if cursor_in_span {
                    let offset = cursor_col - span_start_col;
                    let chars: Vec<char> = span.text.chars().collect();

                    // Before cursor
                    if offset > 0 {
                        let start = full_text.len();
                        for &c in &chars[..offset] { full_text.push(c); }
                        span_ranges.push((start, full_text.len(), span.r, span.g, span.b));
                    }

                    // Cursor char (inverted: use background color as text)
                    let start = full_text.len();
                    full_text.push(chars.get(offset).copied().unwrap_or(' '));
                    let (br, bg, bb) = self.theme.background;
                    span_ranges.push((start, full_text.len(), br, bg, bb));

                    // After cursor
                    if offset + 1 < chars.len() {
                        let start = full_text.len();
                        for &c in &chars[offset + 1..] { full_text.push(c); }
                        span_ranges.push((start, full_text.len(), span.r, span.g, span.b));
                    }
                } else {
                    let start = full_text.len();
                    full_text.push_str(&span.text);
                    span_ranges.push((start, full_text.len(), span.r, span.g, span.b));
                }

                col = span_end_col;
            }

            // Build rich text spans from owned string
            let rich: Vec<(&str, Attrs)> = span_ranges
                .iter()
                .map(|&(start, end, r, g, b)| {
                    (&full_text[start..end], Attrs::new().family(Family::Monospace).color(Color::rgb(r, g, b)))
                })
                .collect();

            self.text_buffer.set_rich_text(
                &mut self.font_system,
                rich,
                &Attrs::new().family(Family::Monospace).color(Color::rgb(
                    self.theme.foreground.0,
                    self.theme.foreground.1,
                    self.theme.foreground.2,
                )),
                Shaping::Advanced,
                None,
            );
            self.text_buffer.shape_until_scroll(&mut self.font_system, false);
        }

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                self.surface.configure(&self.device, &self.config);
                return;
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

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("native-terminal-pass"),
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

            self.text_renderer
                .render(&self.text_atlas, &self.viewport, &mut pass)
                .expect("failed to render text");
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        self.text_atlas.trim();
    }
}
