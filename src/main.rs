mod gpu;
mod rects;
mod typeset;

use book_writing_app::app::{App, Effect, ExportKind, Screen};
use book_writing_app::cli::Args;
use book_writing_app::config::{self, Config};
use book_writing_app::editor::Editor;
use book_writing_app::input;
use book_writing_app::recent::Recent;
use book_writing_app::scroll::Scroll;
use book_writing_app::toolbar::{self, Click};
use book_writing_app::ui::Bounds;
use book_writing_app::vim::{Key, Mode};
use gpu::{Frame, Gpu, Layout, WindowGpu};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

/// How much of the text to keep in view around the caret, and how far one wheel notch scrolls.
const PADDING: f32 = 54.0;
const WHEEL_NOTCH: f32 = 81.0;

/// The window and everything that draws to it, around the app that decides what to draw.
struct Shell {
    window: Option<Arc<Window>>,
    gpu: Option<WindowGpu>,
    app: App,
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

impl Shell {
    fn size(&self) -> (f32, f32) {
        self.gpu.as_ref().map_or((0.0, 0.0), |gpu| {
            let (width, height) = gpu.size();
            (width as f32, height as f32)
        })
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Draws a frame: scrolls to the caret if the editor changed, eases the scroll, and asks for
    /// another frame while it is still moving.
    fn redraw(&mut self) {
        let size = self.size();
        let (Some(gpu), Some(window)) = (&mut self.gpu, &self.window) else {
            return;
        };
        let chrome = self.app.chrome(size);
        if self.app.screen != Screen::Editing {
            gpu.draw(&Frame {
                view: None,
                editor: Bounds::default(),
                scroll: 0.0,
                chrome: &chrome,
            });
            return;
        }
        let regions = self.app.regions(size);
        let view = self.app.editor.view();
        self.layout = gpu.gpu.layout(&view, regions.editor);
        let seen = (view.version, view.cursor);
        let viewport = regions.editor.h;
        if self.seen != Some(seen) {
            self.seen = Some(seen);
            if let Some((top, bottom)) = self.layout.caret {
                let content = self.layout.content_height;
                self.scroll.reveal(top, bottom, PADDING, content, viewport);
            }
        }
        self.scroll.clamp(self.layout.content_height, viewport);
        let now = Instant::now();
        // A long pause, like the window being hidden, shouldn't make the scroll leap.
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        let moving = self.scroll.step(dt);
        gpu.draw(&Frame {
            view: Some(&view),
            editor: regions.editor,
            scroll: self.scroll.offset(),
            chrome: &chrome,
        });
        if moving {
            window.request_redraw();
        }
    }

    /// Carries out what the app asked for: quitting, and the file dialogs that only the window
    /// can show.
    fn carry_out(&mut self, event_loop: &ActiveEventLoop, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Quit => event_loop.exit(),
                Effect::PickFile => {
                    let file = rfd::FileDialog::new()
                        .set_title("Open a djot file")
                        .add_filter("Djot and text", &["dj", "djot", "md", "txt"])
                        .pick_file();
                    if let Some(file) = file {
                        self.app.open_file(&file);
                        self.scroll = Scroll::default();
                        self.seen = None;
                    }
                }
                Effect::PickExport(kind) => {
                    let suggestion = self.app.export_suggestion(kind);
                    let (title, name, filter) = match kind {
                        ExportKind::Markdown => ("Export as Markdown", "Markdown", "md"),
                        ExportKind::Pdf => ("Export as PDF", "PDF", "pdf"),
                    };
                    let mut dialog = rfd::FileDialog::new()
                        .set_title(title)
                        .add_filter(name, &[filter]);
                    if let Some(folder) = suggestion.parent().filter(|p| !p.as_os_str().is_empty())
                    {
                        dialog = dialog.set_directory(folder);
                    }
                    if let Some(file) = suggestion.file_name() {
                        dialog = dialog.set_file_name(file.to_string_lossy());
                    }
                    if let Some(file) = dialog.save_file() {
                        self.app.export_to(kind, &file);
                    }
                }
                Effect::PickFolder => {
                    if let Some(folder) = rfd::FileDialog::new()
                        .set_title("Open a folder")
                        .pick_folder()
                    {
                        self.app.open_folder(&folder);
                    }
                }
            }
        }
        self.request_redraw();
    }
}

impl ApplicationHandler for Shell {
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
                self.request_redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x as f32, position.y as f32);
                if self.app.hover_at(self.size(), self.mouse.0, self.mouse.1) {
                    self.request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let (size, mouse) = (self.size(), self.mouse);
                let Some(gpu) = &self.gpu else { return };
                let effects = self.app.click(size, mouse, |display, block| {
                    gpu.gpu.hit(display, mouse.0, mouse.1, block)
                });
                self.carry_out(event_loop, effects);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let pixels = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => lines * WHEEL_NOTCH,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32,
                };
                let viewport = self.app.regions(self.size()).editor.h;
                self.scroll
                    .by(-pixels, self.layout.content_height, viewport);
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let keys = input::keys(&event.logical_key, event.text.as_deref(), self.modifiers);
                let size = self.size();
                let effects = keys
                    .into_iter()
                    .flat_map(|key| self.app.key(size, key))
                    .collect();
                self.carry_out(event_loop, effects);
            }
            // Text composed by an input method arrives whole.
            WindowEvent::Ime(Ime::Commit(text)) => {
                let size = self.size();
                let effects = text
                    .chars()
                    .flat_map(|c| self.app.key(size, Key::Char(c)))
                    .collect();
                self.carry_out(event_loop, effects);
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    /// After a batch of events, and when auto-save's wait is over: save if it is time, and sleep
    /// until the next time it could be.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let modified = self.app.editor.buf.is_modified();
        self.app.tick(Instant::now());
        if modified != self.app.editor.buf.is_modified() {
            self.request_redraw();
        }
        event_loop.set_control_flow(match self.app.next_wake() {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }
}

/// Writes RGBA pixels to a PNG file.
fn write_png(path: &std::path::Path, (width, height): (u32, u32), pixels: &[u8]) {
    let file = std::fs::File::create(path).expect("failed to create the screenshot file");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(pixels))
        .expect("failed to write the PNG");
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

    let editor = match &args.file {
        Some(path) => Editor::open(path).unwrap_or_else(|error| {
            eprintln!("can't open {}: {error}", path.display());
            std::process::exit(1);
        }),
        None => Editor::default(),
    };
    let screenshot = args.screenshot.is_some();
    // Screenshots list the files they are given; the window lists and saves the user's own.
    let (recent, recent_path) = if screenshot {
        let mut recent = Recent::default();
        for path in args.recent.iter().rev() {
            recent.add(path.clone());
        }
        (recent, None)
    } else {
        let path: Option<PathBuf> = config::dir().map(|dir| dir.join("recent.toml"));
        (
            path.as_deref().map_or_else(Recent::default, Recent::load),
            path,
        )
    };
    let start = args.file.is_none() || args.start;
    let screen = if start {
        Screen::Start {
            message: String::new(),
        }
    } else {
        Screen::Editing
    };
    let mut app = App::new(editor, screen, recent, recent_path);
    // Screenshots use the default settings, and the window the user's own.
    let config_path: Option<PathBuf> = config::dir().map(|dir| dir.join("config.toml"));
    let mut config = match (&config_path, screenshot) {
        (Some(path), false) => Config::load(path),
        _ => Config::default(),
    };
    config.auto_save |= args.autosave;
    app.set_config(config, config_path.filter(|_| !screenshot));
    app.tree_visible = args.tree;
    if let Some(folder) = &args.folder {
        app.open_folder(folder);
    }

    if let Some(path) = &args.export {
        let kind = if path.extension().is_some_and(|e| e == "pdf") {
            ExportKind::Pdf
        } else {
            ExportKind::Markdown
        };
        app.export_to(kind, path);
        eprintln!("{}", app.message);
        std::process::exit(i32::from(!app.message.starts_with("Exported")));
    }

    if let Some(path) = &args.screenshot {
        let (width, height) = args.size;
        let size = (width as f32, height as f32);
        // Screenshots can show any mode, so get into it the way a person would.
        let mode_key = match args.mode {
            Mode::Normal => None,
            Mode::Insert => Some('i'),
            Mode::Visual => Some('v'),
            Mode::VisualLine => Some('V'),
        };
        if app.screen == Screen::Editing {
            for key in mode_key
                .map(Key::Char)
                .into_iter()
                .chain(args.keys.iter().copied())
            {
                app.key(size, key);
            }
        }
        let regions = app.regions(size);
        if let Some(menu) = args.popup {
            let toolbar = toolbar::toolbar(regions.toolbar);
            let anchor = toolbar
                .buttons
                .iter()
                .find(|b| b.click == Click::Open(menu))
                .map_or_else(Default::default, |b| b.bounds);
            app.popup = Some(toolbar::popup(menu, anchor, size));
        }
        let chrome = app.chrome(size);
        let mut gpu = Gpu::headless();
        let pixels = if app.screen == Screen::Editing {
            let view = app.editor.view();
            // Scroll to the caret as the window would, but arrive at once.
            let layout = gpu.layout(&view, regions.editor);
            let mut scroll = Scroll::default();
            if let Some((top, bottom)) = layout.caret {
                let content = layout.content_height;
                scroll.reveal(top, bottom, PADDING, content, regions.editor.h);
            }
            scroll.finish();
            gpu.screenshot(
                width,
                height,
                &Frame {
                    view: Some(&view),
                    editor: regions.editor,
                    scroll: scroll.offset(),
                    chrome: &chrome,
                },
            )
        } else {
            gpu.screenshot(
                width,
                height,
                &Frame {
                    view: None,
                    editor: Bounds::default(),
                    scroll: 0.0,
                    chrome: &chrome,
                },
            )
        };
        write_png(path, args.size, &pixels);
        return;
    }

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop
        .run_app(&mut Shell {
            window: None,
            gpu: None,
            app,
            modifiers: ModifiersState::empty(),
            mouse: (0.0, 0.0),
            scroll: Scroll::default(),
            layout: Layout::default(),
            seen: None,
            last_frame: Instant::now(),
        })
        .expect("event loop failed");
}
