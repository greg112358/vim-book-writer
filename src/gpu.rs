use std::sync::Arc;
use winit::dpi::PhysicalSize;
use winit::window::Window;

/// Format of the offscreen frame. sRGB, so a color drawn here matches one drawn to a window.
const SCREENSHOT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Draws a frame into any texture view: the window's, or an offscreen one for screenshots.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// A GPU and the window surface it draws to.
pub struct WindowGpu {
    pub gpu: Gpu,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

impl Gpu {
    fn request(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface>,
    ) -> (Self, wgpu::Adapter) {
        let adapter = ["a GPU", "a software renderer"]
            .iter()
            .enumerate()
            .find_map(|(i, _)| {
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    compatible_surface: surface,
                    force_fallback_adapter: i == 1,
                    ..Default::default()
                }))
                .ok()
            })
            .expect("no GPU or software renderer available");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("failed to create device");
        (Self { device, queue }, adapter)
    }

    /// A GPU with no window, for screenshots.
    pub fn headless() -> Self {
        Self::request(&wgpu::Instance::default(), None).0
    }

    /// Draws the frame into `view`.
    fn paint(&self, view: &wgpu::TextureView) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.96,
                            g: 0.94,
                            b: 0.88,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        self.queue.submit([encoder.finish()]);
    }

    /// Draws one frame offscreen and returns it as tightly packed RGBA bytes.
    pub fn screenshot(&self, width: u32, height: u32) -> Vec<u8> {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screenshot"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCREENSHOT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.paint(&texture.create_view(&Default::default()));

        // Rows in a buffer copy must start on 256-byte boundaries.
        let row = width * 4;
        let padded_row = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot readback"),
            size: u64::from(padded_row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |result| {
            result.expect("failed to read the frame back")
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("GPU stopped responding");
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .expect("failed to read the frame back");
        mapped
            .chunks(padded_row as usize)
            .flat_map(|line| &line[..row as usize])
            .copied()
            .collect()
    }
}

impl WindowGpu {
    pub fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .expect("failed to create surface");
        let (gpu, adapter) = Gpu::request(&instance, Some(&surface));
        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface not supported by adapter");
        surface.configure(&gpu.device, &config);
        Self {
            gpu,
            surface,
            config,
        }
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width.max(1);
        self.config.height = size.height.max(1);
        self.surface.configure(&self.gpu.device, &self.config);
    }

    pub fn draw(&mut self) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            _ => {
                self.surface.configure(&self.gpu.device, &self.config);
                return;
            }
        };
        self.gpu
            .paint(&frame.texture.create_view(&Default::default()));
        self.gpu.queue.present(frame);
    }
}
