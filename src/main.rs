mod gpu;
mod rects;
mod typeset;

use book_writing_app::cli::Args;
use book_writing_app::editor::Editor;
use book_writing_app::input;
use book_writing_app::scroll::Scroll;
use book_writing_app::vim::{Key, Mode};
use gpu::{Gpu, Layout, WindowGpu};
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

/// How much of the text to keep in view around the caret, and how far one wheel notch scrolls.
const PADDING: f32 = 54.0;
const WHEEL_NOTCH: f32 = 81.0;

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<WindowGpu>,
    editor: Editor,
    modifiers: ModifiersState,
    /// Where the mouse is, in pixels from the window's top left.
    mouse: (f32, f32),
    scroll: Scroll,
    /// The layout of the last frame, for scrolling by the wheel.
    layout: Layout,
    /// The display version and cursor of the last frame, to scroll to the caret when they change.
    seen: Option<(u64, usize)>,
    last_frame: Instant,
}

impl App {
    /// Draws a frame: scrolls to the caret if the editor changed, eases the scroll, and asks for
    /// another frame while it is still moving.
    fn redraw(&mut self) {
        let (Some(gpu), Some(window)) = (&mut self.gpu, &self.window) else {
            return;
        };
        let (width, height) = gpu.size();
        let view = self.editor.view();
        self.layout = gpu.gpu.layout(&view, width);
        let seen = (view.version, view.cursor);
        if self.seen != Some(seen) {
            self.seen = Some(seen);
            if let Some((top, bottom)) = self.layout.caret {
                let content = self.layout.content_height;
                self.scroll
                    .reveal(top, bottom, PADDING, content, height as f32);
            }
        }
        self.scroll.clamp(self.layout.content_height, height as f32);
        let now = Instant::now();
        // A long pause, like the window being hidden, shouldn't make the scroll leap.
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        let moving = self.scroll.step(dt);
        gpu.draw(&view, self.scroll.offset());
        if moving {
            window.request_redraw();
        }
    }

    /// Feeds keys to the editor and redraws. Quits when the editor says to.
    fn keys(&mut self, event_loop: &ActiveEventLoop, keys: impl IntoIterator<Item = Key>) {
        for key in keys {
            if self.editor.key(key) {
                event_loop.exit();
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = Window::default_attributes().with_title("Book Writing App");
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("failed to create window"),
        );
        self.gpu = Some(WindowGpu::new(window.clone()));
        window.set_ime_allowed(true);
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(gpu) = &self.gpu {
                    let block = self.editor.vim.mode != Mode::Insert;
                    let (display, _) = self.editor.display();
                    if let Some(offset) = gpu.gpu.hit(display, self.mouse.0, self.mouse.1, block) {
                        self.editor.click(offset);
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let keys = input::keys(&event.logical_key, event.text.as_deref(), self.modifiers);
                self.keys(event_loop, keys);
            }
            // Text composed by an input method arrives whole.
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.keys(event_loop, text.chars().map(Key::Char));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let pixels = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => lines * WHEEL_NOTCH,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32,
                };
                if let (Some(gpu), Some(window)) = (&self.gpu, &self.window) {
                    let (_, height) = gpu.size();
                    let content = self.layout.content_height;
                    self.scroll.by(-pixels, content, height as f32);
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }
}

fn main() {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            // Asking for help isn't a mistake.
            std::process::exit(i32::from(!message.starts_with("usage:")) * 2);
        }
    };

    let mut editor = match &args.file {
        Some(path) => Editor::open(path).unwrap_or_else(|error| {
            eprintln!("can't open {}: {error}", path.display());
            std::process::exit(1);
        }),
        None => Editor::default(),
    };
    // Screenshots can show any mode, so get into it the way a person would.
    let mode_key = match args.mode {
        Mode::Normal => None,
        Mode::Insert => Some('i'),
        Mode::Visual => Some('v'),
        Mode::VisualLine => Some('V'),
    };
    if args.screenshot.is_some() {
        for key in mode_key
            .map(Key::Char)
            .into_iter()
            .chain(args.keys.iter().copied())
        {
            editor.key(key);
        }
    }

    if let Some(path) = &args.screenshot {
        let (width, height) = args.size;
        let view = editor.view();
        let mut gpu = Gpu::headless();
        // Scroll to the caret as the window would, but arrive at once.
        let layout = gpu.layout(&view, width);
        let mut scroll = Scroll::default();
        if let Some((top, bottom)) = layout.caret {
            scroll.reveal(top, bottom, PADDING, layout.content_height, height as f32);
        }
        scroll.finish();
        let pixels = gpu.screenshot(width, height, &view, scroll.offset());
        let file = std::fs::File::create(path).expect("failed to create the screenshot file");
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&pixels))
            .expect("failed to write the PNG");
        return;
    }

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop
        .run_app(&mut App {
            window: None,
            gpu: None,
            editor,
            modifiers: ModifiersState::empty(),
            mouse: (0.0, 0.0),
            scroll: Scroll::default(),
            layout: Layout::default(),
            seen: None,
            last_frame: Instant::now(),
        })
        .expect("event loop failed");
}
