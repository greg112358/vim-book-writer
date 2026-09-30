use vim_book_writer::ui::Rect;
use wgpu::util::DeviceExt;

const SHADER: &str = "
struct Screen { size: vec2<f32> };
@group(0) @binding(0) var<uniform> screen: Screen;

struct In { @location(0) pos: vec2<f32>, @location(1) color: vec4<f32> };
struct Out { @builtin(position) pos: vec4<f32>, @location(0) color: vec4<f32> };

@vertex fn vs(in: In) -> Out {
    var out: Out;
    out.pos = vec4<f32>(in.pos.x / screen.size.x * 2.0 - 1.0, 1.0 - in.pos.y / screen.size.y * 2.0, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment fn fs(in: Out) -> @location(0) vec4<f32> {
    return in.color;
}
";

/// Draws rectangles: text decorations, the selection, the caret. Glyphon draws only glyphs.
pub struct Rects {
    pipeline: wgpu::RenderPipeline,
    screen: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

fn srgb_to_linear(byte: u8) -> f32 {
    let c = f32::from(byte) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

impl Rects {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rects"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rects"),
            entries: &[layout_entry],
        });
        let screen = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rects screen size"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rects"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: screen.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rects"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rects"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 24,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            screen,
            bind_group,
        }
    }

    /// Draws `rects` in order into `pass`, for a frame `width` by `height` pixels.
    pub fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        (width, height): (u32, u32),
        rects: &[Rect],
    ) {
        if rects.is_empty() {
            return;
        }
        let mut screen = Vec::with_capacity(16);
        for value in [width as f32, height as f32, 0.0, 0.0] {
            screen.extend_from_slice(&value.to_ne_bytes());
        }
        queue.write_buffer(&self.screen, 0, &screen);

        // Two triangles per rectangle: a position and a color for each corner.
        let mut vertices: Vec<u8> = Vec::with_capacity(rects.len() * 6 * 24);
        for rect in rects {
            let color = [
                srgb_to_linear(rect.color[0]),
                srgb_to_linear(rect.color[1]),
                srgb_to_linear(rect.color[2]),
                f32::from(rect.color[3]) / 255.0,
            ];
            let (x0, y0, x1, y1) = (rect.x, rect.y, rect.x + rect.w, rect.y + rect.h);
            for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y0), (x1, y1), (x0, y1)] {
                for value in [x, y, color[0], color[1], color[2], color[3]] {
                    vertices.extend_from_slice(&value.to_ne_bytes());
                }
            }
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rects"),
            contents: &vertices,
            usage: wgpu::BufferUsages::VERTEX,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(0..(rects.len() * 6) as u32, 0..1);
    }
}
