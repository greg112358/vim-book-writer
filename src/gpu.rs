use crate::rects::Rects;
use crate::typeset::{self, FONT_SIZE, LINE_HEIGHT};
use book_writing_app::display::Display;
use book_writing_app::editor::View;
use cosmic_text::fontdb;
use glyphon::{
    Attrs, Buffer, Cache, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport,
};
use std::sync::Arc;
use winit::dpi::PhysicalSize;
use winit::window::Window;

/// The width of the text column, and the least margin on either side of it.
const COLUMN_WIDTH: f32 = 720.0;
const MIN_MARGIN: f32 = 24.0;
const TOP_MARGIN: f32 = 32.0;

/// Format of the offscreen frame. sRGB, so a color drawn here matches one drawn to a window.
const SCREENSHOT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Draws a frame into any texture view: the window's, or an offscreen one for screenshots.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    fonts: FontSystem,
    swash: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text: TextRenderer,
    rects: Rects,
    buffer: Buffer,
    /// The display version and column width of the text `buffer` holds.
    shown: Option<(u64, u32)>,
    /// Where the text's top left corner was drawn, for turning clicks into text positions.
    origin: (f32, f32),
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
    ) -> (wgpu::Device, wgpu::Queue, wgpu::Adapter) {
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
        (device, queue, adapter)
    }

    /// Sets up text drawing for frames in `format`.
    fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let mut fonts = FontSystem::new();
        // The generic families mean whatever the OS names, which is often not installed, so point
        // each at the first of a few good fonts that is.
        let installed = |db: &fontdb::Database, names: &[&str]| {
            names
                .iter()
                .find(|name| {
                    db.faces()
                        .any(|f| f.families.iter().any(|(family, _)| family == **name))
                })
                .map(|name| name.to_string())
        };
        let (serif, sans, mono) = {
            let db = fonts.db();
            (
                installed(
                    db,
                    &[
                        "Source Serif Pro",
                        "Source Serif 4",
                        "Georgia",
                        "Times New Roman",
                        "Liberation Serif",
                        "Noto Serif",
                        "DejaVu Serif",
                    ],
                ),
                installed(
                    db,
                    &[
                        "Inter",
                        "Roboto",
                        "Noto Sans",
                        "Helvetica Neue",
                        "Arial",
                        "Liberation Sans",
                        "DejaVu Sans",
                    ],
                ),
                installed(
                    db,
                    &[
                        "Source Code Pro",
                        "JetBrains Mono",
                        "Menlo",
                        "Consolas",
                        "Liberation Mono",
                        "DejaVu Sans Mono",
                    ],
                ),
            )
        };
        let db = fonts.db_mut();
        if let Some(family) = serif {
            db.set_serif_family(family);
        }
        if let Some(family) = sans {
            db.set_sans_serif_family(family);
        }
        if let Some(family) = mono {
            db.set_monospace_family(family);
        }
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, format);
        let text = TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let buffer = Buffer::new(&mut fonts, Metrics::new(FONT_SIZE, FONT_SIZE * LINE_HEIGHT));
        let rects = Rects::new(&device, format);
        Self {
            device,
            queue,
            fonts,
            swash: SwashCache::new(),
            viewport,
            atlas,
            text,
            rects,
            buffer,
            shown: None,
            origin: (0.0, 0.0),
        }
    }

    /// The displayed offset at the pixel (`x`, `y`) of the last frame: the char under it if
    /// `block`, else the nearest boundary between chars.
    pub fn hit(&self, display: &Display, x: f32, y: f32, block: bool) -> Option<usize> {
        let (line, index) =
            typeset::hit(&self.buffer, x - self.origin.0, y - self.origin.1, block)?;
        Some(display.offset(line, index))
    }

    /// A GPU with no window, for screenshots.
    pub fn headless() -> Self {
        let (device, queue, _) = Self::request(&wgpu::Instance::default(), None);
        Self::new(device, queue, SCREENSHOT_FORMAT)
    }

    /// Draws the frame into `view`, which is `width` by `height` pixels.
    fn paint(&mut self, target: &wgpu::TextureView, width: u32, height: u32, view: &View<'_>) {
        let margin = ((width as f32 - COLUMN_WIDTH) / 2.0).max(MIN_MARGIN);
        let column = (width as f32 - 2.0 * margin).max(1.0);
        let key = (view.version, column as u32);
        if self.shown != Some(key) {
            self.buffer.set_size(Some(column), Some(height as f32));
            let display = view.display;
            self.buffer.set_rich_text(
                display.spans.iter().map(|span| {
                    (
                        &display.text[span.range.clone()],
                        typeset::attrs(&span.format, span.faint),
                    )
                }),
                &Attrs::new().family(Family::Serif),
                Shaping::Advanced,
                None,
            );
            self.shown = Some(key);
        }
        self.buffer.shape_until_scroll(&mut self.fonts, false);
        self.viewport
            .update(&self.queue, Resolution { width, height });
        self.text
            .prepare(
                &self.device,
                &self.queue,
                &mut self.fonts,
                &mut self.atlas,
                &self.viewport,
                [TextArea {
                    buffer: &self.buffer,
                    left: margin,
                    top: TOP_MARGIN,
                    scale: 1.0,
                    bounds: TextBounds::default(),
                    default_color: typeset::TEXT,
                    custom_glyphs: &[],
                }],
                &mut self.swash,
            )
            .expect("failed to prepare text");

        self.origin = (margin, TOP_MARGIN);
        let mut under = Vec::new();
        let mut over = typeset::decorations(&self.buffer, margin, TOP_MARGIN);
        typeset::caret_and_selection(&self.buffer, view, self.origin, &mut under, &mut over);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
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
            self.rects.draw(
                &self.device,
                &self.queue,
                &mut pass,
                (width, height),
                &under,
            );
            self.text
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("failed to draw text");
            self.rects
                .draw(&self.device, &self.queue, &mut pass, (width, height), &over);
        }
        self.queue.submit([encoder.finish()]);
        self.atlas.trim();
    }

    /// Draws one frame offscreen and returns it as tightly packed RGBA bytes.
    pub fn screenshot(&mut self, width: u32, height: u32, view: &View<'_>) -> Vec<u8> {
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
        self.paint(
            &texture.create_view(&Default::default()),
            width,
            height,
            view,
        );

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
        let (device, queue, adapter) = Gpu::request(&instance, Some(&surface));
        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface not supported by adapter");
        surface.configure(&device, &config);
        let gpu = Gpu::new(device, queue, config.format);
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

    pub fn draw(&mut self, view: &View<'_>) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            _ => {
                self.surface.configure(&self.gpu.device, &self.config);
                return;
            }
        };
        self.gpu.paint(
            &frame.texture.create_view(&Default::default()),
            self.config.width,
            self.config.height,
            view,
        );
        self.gpu.queue.present(frame);
    }
}
