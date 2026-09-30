use crate::rects::Rects;
use crate::typeset::{self, LINE_HEIGHT, Reading};
use cosmic_text::fontdb;
use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, Style,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight,
};
use std::collections::HashMap;
use std::sync::Arc;
use vim_book_writer::display::Display;
use vim_book_writer::editor::View;
use vim_book_writer::ui::{Align, Bounds, Chrome, Label, Theme};
use winit::dpi::PhysicalSize;
use winit::window::Window;

/// The width of the text column, and the least margin on either side of it.
const COLUMN_WIDTH: f32 = 720.0;
const MIN_MARGIN: f32 = 24.0;
const TOP_MARGIN: f32 = 32.0;

/// Where things are in the laid-out text, in pixels from the top of the scrollable area, which
/// starts with the top margin.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Layout {
    /// The height of the text with the margins above and below it.
    pub content_height: f32,
    /// The top and bottom of the line the caret is on, if it is laid out.
    pub caret: Option<(f32, f32)>,
}

/// What to draw in one frame.
pub struct Frame<'a, 'v> {
    /// The text to show in the editor region, if there is any; the start screen has none.
    pub view: Option<&'a View<'v>>,
    /// Where the editor goes; the text column is centered in it and clipped to it.
    pub editor: Bounds,
    /// How far the text is scrolled up, in pixels.
    pub scroll: f32,
    /// The toolbar, status bar, and other parts of the window around the editor.
    pub chrome: &'a Chrome,
    pub theme: &'static Theme,
    /// The font and size the text is read in.
    pub reading: &'a Reading,
}

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
    /// Draws the text that goes over the rest, like an open menu.
    text_over: TextRenderer,
    rects: Rects,
    buffer: Buffer,
    /// Shaped interface text, by text, size, and weight.
    labels: HashMap<LabelKey, Buffer>,
    /// The display version, column width, theme and reading font of the text `buffer` holds.
    shown: Option<(u64, u32, bool, Reading)>,
    /// Where each wrapped row of the text in `buffer` starts, as displayed offsets.
    pub rows: Vec<usize>,
    /// The superscripts and subscripts of `buffer`, each drawn again where it belongs.
    scripts: Vec<Script>,
    /// Where the text's top left corner was drawn, for turning clicks into text positions.
    origin: (f32, f32),
}

/// A superscript or subscript shaped on its own: where it goes from the text's top left corner.
struct Script {
    buffer: Buffer,
    x: f32,
    y: f32,
}

/// A GPU and the window surface it draws to.
pub struct WindowGpu {
    pub gpu: Gpu,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

type LabelKey = (String, u32, u8);

fn label_key(label: &Label) -> LabelKey {
    (
        label.text.clone(),
        label.size.to_bits(),
        u8::from(label.bold) | u8::from(label.italic) << 1,
    )
}

/// Shapes each label that isn't cached yet.
fn shape_labels(cache: &mut HashMap<LabelKey, Buffer>, fonts: &mut FontSystem, labels: &[Label]) {
    for label in labels {
        cache.entry(label_key(label)).or_insert_with(|| {
            let mut buffer = Buffer::new(fonts, Metrics::new(label.size, label.size * 1.3));
            let mut attrs = Attrs::new().family(Family::SansSerif);
            if label.bold {
                attrs = attrs.weight(Weight::BOLD);
            }
            if label.italic {
                attrs = attrs.style(Style::Italic);
            }
            buffer.set_text(&label.text, &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(fonts, false);
            buffer
        });
    }
}

/// Where each shaped label goes on screen, from its anchor and alignment.
fn label_areas<'a>(cache: &'a HashMap<LabelKey, Buffer>, labels: &[Label]) -> Vec<TextArea<'a>> {
    labels
        .iter()
        .map(|label| {
            let buffer = &cache[&label_key(label)];
            let width = buffer
                .layout_runs()
                .map(|run| run.line_w)
                .fold(0.0, f32::max);
            let left = match label.align {
                Align::Left => label.x,
                Align::Right => label.x - width,
                Align::Center => label.x - width / 2.0,
            };
            let [r, g, b, a] = label.color;
            TextArea {
                buffer,
                left,
                top: label.y,
                scale: 1.0,
                bounds: TextBounds::default(),
                default_color: Color::rgba(r, g, b, a),
                custom_glyphs: &[],
            }
        })
        .collect()
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
        let buffer = Buffer::new(&mut fonts, Metrics::new(18.0, 18.0 * LINE_HEIGHT));
        let text_over =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let rects = Rects::new(&device, format);
        Self {
            device,
            queue,
            fonts,
            swash: SwashCache::new(),
            viewport,
            atlas,
            text,
            text_over,
            rects,
            buffer,
            labels: HashMap::new(),
            shown: None,
            rows: Vec::new(),
            scripts: Vec::new(),
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

    /// Lays out the view's text in a column centered in an editor `width` wide. Only shapes again
    /// if the text or the width changed. Returns the margin to the left of the column.
    fn sync(&mut self, view: &View<'_>, width: f32, theme: &Theme, reading: &Reading) -> f32 {
        let margin = ((width - COLUMN_WIDTH) / 2.0).max(MIN_MARGIN);
        let column = (width - 2.0 * margin).max(1.0);
        let key = (view.version, column as u32, theme.dark, reading.clone());
        let reshaped = self.shown.as_ref() != Some(&key);
        if reshaped {
            // No height, so every line is laid out and the caret can be found wherever it is.
            self.buffer.set_size(Some(column), None);
            let display = view.display;
            self.buffer.set_rich_text(
                display.spans.iter().map(|span| {
                    (
                        &display.text[span.range.clone()],
                        typeset::attrs(&span.format, span.faint, theme, reading),
                    )
                }),
                &Attrs::new().family(Family::Serif),
                Shaping::Advanced,
                None,
            );
            self.shown = Some(key);
        }
        self.buffer.shape_until_scroll(&mut self.fonts, false);
        if reshaped {
            self.scripts.clear();
            for span in &view.display.spans {
                let format = &span.format;
                if span.faint || !(format.superscript || format.subscript) {
                    continue;
                }
                let (attrs, raise) = typeset::script_attrs(format, theme, reading);
                let size = typeset::base_size(format, reading) * 0.65;
                let mut at = span.range.start;
                for piece in view.display.text[span.range.clone()].split_inclusive('\n') {
                    let start = at;
                    at += piece.len();
                    let text = piece.trim_end_matches('\n');
                    let (line, index) = view.display.line_and_index(start);
                    // The glyph the piece starts at, and the baseline of the row it is on.
                    let found = self
                        .buffer
                        .layout_runs()
                        .filter(|run| run.line_i == line)
                        .find_map(|run| {
                            run.glyphs
                                .iter()
                                .find(|g| g.start <= index && index < g.end)
                                .map(|g| (g.x, run.line_y))
                        });
                    let (Some((x, baseline)), false) = (found, text.is_empty()) else {
                        continue;
                    };
                    let mut buffer =
                        Buffer::new(&mut self.fonts, Metrics::new(size, size * LINE_HEIGHT));
                    buffer.set_size(None, None);
                    buffer.set_rich_text(
                        [(text, attrs.clone())],
                        &Attrs::new().family(Family::Serif),
                        Shaping::Advanced,
                        None,
                    );
                    buffer.shape_until_scroll(&mut self.fonts, false);
                    let own = buffer.layout_runs().next().map_or(0.0, |run| run.line_y);
                    self.scripts.push(Script {
                        buffer,
                        x,
                        y: baseline - own - raise,
                    });
                }
            }
            self.rows.clear();
            self.rows.extend(self.buffer.layout_runs().map(|run| {
                let index = run.glyphs.first().map_or(0, |g| g.start);
                view.display.offset(run.line_i, index)
            }));
        }
        margin
    }

    /// Lays out the text for an editor `editor` big and says how tall it is and where the caret
    /// is, so the caller can decide how far to scroll.
    pub fn layout(
        &mut self,
        view: &View<'_>,
        editor: Bounds,
        theme: &Theme,
        reading: &Reading,
    ) -> Layout {
        let margin = self.sync(view, editor.w, theme, reading);
        let caret = typeset::caret_and_selection(
            &self.buffer,
            view,
            (margin, 0.0),
            &mut Vec::new(),
            &mut Vec::new(),
            theme,
            reading,
        );
        Layout {
            content_height: typeset::text_height(&self.buffer) + 2.0 * TOP_MARGIN,
            caret: caret.map(|(top, bottom)| (top + TOP_MARGIN, bottom + TOP_MARGIN)),
        }
    }

    /// Draws the frame into `target`, which is `width` by `height` pixels.
    fn paint(
        &mut self,
        target: &wgpu::TextureView,
        (width, height): (u32, u32),
        frame: &Frame<'_, '_>,
    ) {
        let editor = frame.editor;
        let (mut left, mut top) = (0.0, 0.0);
        if let Some(view) = frame.view {
            let margin = self.sync(view, editor.w, frame.theme, frame.reading);
            (left, top) = (editor.x + margin, editor.y + TOP_MARGIN - frame.scroll);
            self.origin = (left, top);
        }

        // Shape each label once, and work out where it goes from its width.
        if self.labels.len() > 512 {
            self.labels.clear();
        }
        shape_labels(&mut self.labels, &mut self.fonts, &frame.chrome.labels);
        shape_labels(&mut self.labels, &mut self.fonts, &frame.chrome.over_labels);
        let mut areas = Vec::new();
        if frame.view.is_some() {
            areas.push(TextArea {
                buffer: &self.buffer,
                left,
                top,
                scale: 1.0,
                bounds: TextBounds {
                    left: editor.x as i32,
                    top: editor.y as i32,
                    right: (editor.x + editor.w) as i32,
                    bottom: (editor.y + editor.h) as i32,
                },
                default_color: typeset::text_color(frame.theme),
                custom_glyphs: &[],
            });
        }
        if frame.view.is_some() {
            areas.extend(self.scripts.iter().map(|script| TextArea {
                buffer: &script.buffer,
                left: left + script.x,
                top: top + script.y,
                scale: 1.0,
                bounds: TextBounds {
                    left: editor.x as i32,
                    top: editor.y as i32,
                    right: (editor.x + editor.w) as i32,
                    bottom: (editor.y + editor.h) as i32,
                },
                default_color: typeset::text_color(frame.theme),
                custom_glyphs: &[],
            }));
        }
        areas.extend(label_areas(&self.labels, &frame.chrome.labels));
        let over_areas = label_areas(&self.labels, &frame.chrome.over_labels);
        self.viewport
            .update(&self.queue, Resolution { width, height });
        self.text
            .prepare(
                &self.device,
                &self.queue,
                &mut self.fonts,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash,
            )
            .expect("failed to prepare text");
        self.text_over
            .prepare(
                &self.device,
                &self.queue,
                &mut self.fonts,
                &mut self.atlas,
                &self.viewport,
                over_areas,
                &mut self.swash,
            )
            .expect("failed to prepare text");

        let mut under = Vec::new();
        let mut over = Vec::new();
        if let Some(view) = frame.view {
            over = typeset::decorations(&self.buffer, left, top, frame.theme);
            over.extend(typeset::gutter(
                &self.buffer,
                view,
                (left, top),
                frame.theme,
            ));
            typeset::caret_and_selection(
                &self.buffer,
                view,
                self.origin,
                &mut under,
                &mut over,
                frame.theme,
                frame.reading,
            );
        }
        // A thin thumb at the editor's right edge shows how far through the text the view is.
        if frame.view.is_some() {
            let content = typeset::text_height(&self.buffer) + 2.0 * TOP_MARGIN;
            if content > editor.h {
                let thumb = (editor.h * editor.h / content).max(24.0);
                let range = (content - editor.h).max(1.0);
                let [r, g, b, _] = frame.theme.muted;
                over.push(vim_book_writer::ui::Rect {
                    x: editor.x + editor.w - 8.0,
                    y: editor.y + (frame.scroll / range).clamp(0.0, 1.0) * (editor.h - thumb),
                    w: 4.0,
                    h: thumb,
                    color: [r, g, b, 0x70],
                });
            }
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: frame.theme.page[0],
                            g: frame.theme.page[1],
                            b: frame.theme.page[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let size = (width, height);
            self.rects.draw(
                &self.device,
                &self.queue,
                &mut pass,
                size,
                &frame.chrome.rects,
            );
            // The selection and caret stay inside the editor, however far the text is scrolled.
            pass.set_scissor_rect(
                editor.x as u32,
                editor.y as u32,
                editor.w as u32,
                editor.h as u32,
            );
            self.rects
                .draw(&self.device, &self.queue, &mut pass, size, &under);
            pass.set_scissor_rect(0, 0, width, height);
            self.text
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("failed to draw text");
            pass.set_scissor_rect(
                editor.x as u32,
                editor.y as u32,
                editor.w as u32,
                editor.h as u32,
            );
            self.rects
                .draw(&self.device, &self.queue, &mut pass, size, &over);
            pass.set_scissor_rect(0, 0, width, height);
            self.rects.draw(
                &self.device,
                &self.queue,
                &mut pass,
                size,
                &frame.chrome.over_rects,
            );
            self.text_over
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("failed to draw text");
        }
        self.queue.submit([encoder.finish()]);
        self.atlas.trim();
    }

    /// Draws one frame offscreen and returns it as tightly packed RGBA bytes.
    pub fn screenshot(&mut self, width: u32, height: u32, frame: &Frame<'_, '_>) -> Vec<u8> {
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
            (width, height),
            frame,
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

    /// The size of the window's surface in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn draw(&mut self, frame: &Frame<'_, '_>) {
        let surface_frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            _ => {
                self.surface.configure(&self.gpu.device, &self.config);
                return;
            }
        };
        self.gpu.paint(
            &surface_frame.texture.create_view(&Default::default()),
            (self.config.width, self.config.height),
            frame,
        );
        self.gpu.queue.present(surface_frame);
    }
}
