mod gpu;
mod rects;
mod typeset;

use book_writing_app::cli::Args;
use book_writing_app::editor::Editor;
use book_writing_app::input;
use book_writing_app::vim::{Key, Mode};
use gpu::{Gpu, WindowGpu};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<WindowGpu>,
    editor: Editor,
    modifiers: ModifiersState,
    /// Where the mouse is, in pixels from the window's top left.
    mouse: (f32, f32),
}

impl App {
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
            WindowEvent::RedrawRequested => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.draw(&self.editor.view());
                }
            }
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
        let pixels = Gpu::headless().screenshot(width, height, &view);
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
        })
        .expect("event loop failed");
}
