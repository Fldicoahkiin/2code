/// Pixel-space rectangle for rendering.
pub struct CursorRect {
	pub x: u32,
	pub y: u32,
	pub width: u32,
	pub height: u32,
	pub color: [f32; 4],
}

/// Renders solid-color rectangles for cursor block and cell backgrounds.
/// Uses a fullscreen triangle + scissor rect technique.
/// Batches same-color rectangles into a single render pass.
pub struct RectRenderer {
	pipeline: wgpu::RenderPipeline,
	color_buf: wgpu::Buffer,
}

impl RectRenderer {
	pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
		let shader =
			device.create_shader_module(wgpu::ShaderModuleDescriptor {
				label: Some("rect-shader"),
				source: wgpu::ShaderSource::Wgsl(RECT_WGSL.into()),
			});

		let bind_group_layout =
			device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
				label: Some("rect-bgl"),
				entries: &[wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Buffer {
						ty: wgpu::BufferBindingType::Uniform,
						has_dynamic_offset: false,
						min_binding_size: None,
					},
					count: None,
				}],
			});

		let pipeline_layout =
			device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
				label: Some("rect-pl"),
				bind_group_layouts: &[Some(&bind_group_layout)],
				immediate_size: 0,
			});

		let pipeline =
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some("rect-pipeline"),
				layout: Some(&pipeline_layout),
				vertex: wgpu::VertexState {
					module: &shader,
					entry_point: Some("vs_main"),
					buffers: &[],
					compilation_options: Default::default(),
				},
				fragment: Some(wgpu::FragmentState {
					module: &shader,
					entry_point: Some("fs_main"),
					targets: &[Some(wgpu::ColorTargetState {
						format,
						blend: Some(wgpu::BlendState::ALPHA_BLENDING),
						write_mask: wgpu::ColorWrites::ALL,
					})],
					compilation_options: Default::default(),
				}),
				primitive: wgpu::PrimitiveState {
					topology: wgpu::PrimitiveTopology::TriangleList,
					..Default::default()
				},
				depth_stencil: None,
				multisample: wgpu::MultisampleState::default(),
				multiview_mask: None,
				cache: None,
			});

		let color_buf = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("rect-color"),
			size: 16, // vec4<f32>
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});

		Self {
			pipeline,
			color_buf,
		}
	}

	/// Draw multiple solid rectangles, batching same-color rects into one pass.
	pub fn render_batch(
		&self,
		encoder: &mut wgpu::CommandEncoder,
		view: &wgpu::TextureView,
		queue: &wgpu::Queue,
		device: &wgpu::Device,
		rects: &mut [CursorRect],
	) {
		// Filter out zero-size rects
		let valid_count =
			rects.iter().filter(|r| r.width > 0 && r.height > 0).count();
		if valid_count == 0 {
			return;
		}

		// Sort by color to group same-color rects together.
		// f32 comparison via bit pattern for grouping (not ordering correctness).
		rects.sort_unstable_by(|a, b| {
			let ka = bytemuck::cast::<[f32; 4], [u32; 4]>(a.color);
			let kb = bytemuck::cast::<[f32; 4], [u32; 4]>(b.color);
			ka.cmp(&kb)
		});

		let bind_group_layout = self.pipeline.get_bind_group_layout(0);
		let mut i = 0;

		while i < rects.len() {
			if rects[i].width == 0 || rects[i].height == 0 {
				i += 1;
				continue;
			}

			let color = rects[i].color;
			queue.write_buffer(
				&self.color_buf,
				0,
				bytemuck::cast_slice(&color),
			);

			let bind_group =
				device.create_bind_group(&wgpu::BindGroupDescriptor {
					label: None,
					layout: &bind_group_layout,
					entries: &[wgpu::BindGroupEntry {
						binding: 0,
						resource: self.color_buf.as_entire_binding(),
					}],
				});

			let mut pass =
				encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
					label: None,
					color_attachments: &[Some(
						wgpu::RenderPassColorAttachment {
							view,
							resolve_target: None,
							ops: wgpu::Operations {
								load: wgpu::LoadOp::Load,
								store: wgpu::StoreOp::Store,
							},
							depth_slice: None,
						},
					)],
					depth_stencil_attachment: None,
					timestamp_writes: None,
					occlusion_query_set: None,
					multiview_mask: None,
				});

			pass.set_pipeline(&self.pipeline);
			pass.set_bind_group(0, &bind_group, &[]);

			// Draw all rects with this color
			while i < rects.len() && rects[i].color == color {
				let r = &rects[i];
				if r.width > 0 && r.height > 0 {
					pass.set_scissor_rect(r.x, r.y, r.width, r.height);
					pass.draw(0..3, 0..1);
				}
				i += 1;
			}
		}
	}
}

const RECT_WGSL: &str = r#"
@group(0) @binding(0) var<uniform> rect_color: vec4<f32>;

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(idx) / 2) * 4.0 - 1.0;
    let y = f32(i32(idx) % 2) * 4.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return rect_color;
}
"#;
