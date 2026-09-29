mod gpu;
mod rects;
mod typeset;

use book_writing_app::cli::Args;
use book_writing_app::editor::Editor;
use book_writing_app::input;
use book_writing_app::scroll::Scroll;
use book_writing_app::toolbar::{self, Click, Popup};
use book_writing_app::ui::{self, Interface, Regions, Status};
use book_writing_app::vim::{Key, Mode};
use gpu::{Frame, Gpu, Layout, WindowGpu};
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

/// What the window around the editor shows for the editor's current state.
fn chrome(editor: &mut Editor, regions: &Regions, interface: &Interface<'_>) -> ui::Chrome {
    let status = Status {
        mode: editor.vim.mode,
        file: editor.file_name(),
        modified: editor.buf.is_modified(),
        words: editor.word_count(),
        message: editor.vim.message.clone(),
    };
    ui::chrome(regions, &status, interface)
}

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<WindowGpu>,
    editor: Editor,
    modifiers: ModifiersState,
    /// Where the mouse is, in pixels from the window's top left.
    mouse: (f32, f32),
    /// Whether the file tree is showing.
    tree: bool,
    /// The toolbar menu that is open, the button the mouse is over, and the last color chosen.
    popup: Option<Popup>,
    hover: Option<usize>,
    color: [u8; 3],
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
        let regions = ui::regions(width as f32, height as f32, self.tree);
        let toolbar = toolbar::toolbar(regions.toolbar);
        let interface = Interface {
            toolbar: &toolbar,
            hover: self.hover,
            popup: self.popup.as_ref(),
            color: self.color,
        };
        let chrome = chrome(&mut self.editor, &regions, &interface);
        let view = self.editor.view();
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
            view: &view,
            editor: regions.editor,
            scroll: self.scroll.offset(),
            chrome: &chrome,
        });
        if moving {
            window.request_redraw();
        }
    }

    /// A left click at `mouse`: on an open menu's choices, the toolbar's buttons, or the text.
    fn click(&mut self, (x, y): (f32, f32)) {
        let Some(gpu) = &self.gpu else { return };
        let (width, height) = gpu.size();
        let regions = ui::regions(width as f32, height as f32, self.tree);
        // An open menu takes the click: a choice applies, and anywhere else just closes it.
        if let Some(popup) = self.popup.take() {
            if let Some(item) = popup.hit(x, y) {
                if let Some(color) = item.swatch {
                    self.color = color;
                }
                self.editor.apply(&item.action);
            }
            return;
        }
        let toolbar = toolbar::toolbar(regions.toolbar);
        if let Some(i) = toolbar.hit(x, y) {
            match toolbar.buttons[i].click.clone() {
                Click::Do(action) => self.editor.apply(&action),
                Click::Open(menu) => {
                    let anchor = toolbar.buttons[i].bounds;
                    self.popup = Some(toolbar::popup(menu, anchor, (width as f32, height as f32)));
                }
            }
        } else if regions.editor.contains(x, y) {
            let block = self.editor.vim.mode != Mode::Insert;
            let (display, _) = self.editor.display();
            if let Some(offset) = gpu.gpu.hit(display, x, y, block) {
                self.editor.click(offset);
            }
        }
    }

    /// Feeds keys to the editor and redraws. Quits when the editor says to.
    fn keys(&mut self, event_loop: &ActiveEventLoop, keys: impl IntoIterator<Item = Key>) {
        for key in keys {
            // Esc closes an open menu instead of reaching the editor.
            if key == Key::Esc && self.popup.take().is_some() {
                continue;
            }
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
                if let Some(gpu) = &self.gpu {
                    let (width, height) = gpu.size();
                    let regions = ui::regions(width as f32, height as f32, self.tree);
                    let hover = toolbar::toolbar(regions.toolbar).hit(self.mouse.0, self.mouse.1);
                    if hover != self.hover {
                        self.hover = hover;
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                self.click(self.mouse);
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
                    let (width, height) = gpu.size();
                    let viewport = ui::regions(width as f32, height as f32, self.tree).editor.h;
                    self.scroll
                        .by(-pixels, self.layout.content_height, viewport);
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
        let regions = ui::regions(width as f32, height as f32, args.tree);
        let toolbar = toolbar::toolbar(regions.toolbar);
        let popup = args.popup.map(|menu| {
            let anchor = toolbar
                .buttons
                .iter()
                .find(|b| b.click == Click::Open(menu))
                .map_or_else(Default::default, |b| b.bounds);
            toolbar::popup(menu, anchor, (width as f32, height as f32))
        });
        let interface = Interface {
            toolbar: &toolbar,
            hover: None,
            popup: popup.as_ref(),
            color: [0xcc, 0, 0],
        };
        let chrome = chrome(&mut editor, &regions, &interface);
        let view = editor.view();
        let mut gpu = Gpu::headless();
        // Scroll to the caret as the window would, but arrive at once.
        let layout = gpu.layout(&view, regions.editor);
        let mut scroll = Scroll::default();
        if let Some((top, bottom)) = layout.caret {
            let content = layout.content_height;
            scroll.reveal(top, bottom, PADDING, content, regions.editor.h);
        }
        scroll.finish();
        let pixels = gpu.screenshot(
            width,
            height,
            &Frame {
                view: &view,
                editor: regions.editor,
                scroll: scroll.offset(),
                chrome: &chrome,
            },
        );
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
            tree: args.tree,
            popup: None,
            hover: None,
            color: [0xcc, 0, 0],
            scroll: Scroll::default(),
            layout: Layout::default(),
            seen: None,
            last_frame: Instant::now(),
        })
        .expect("event loop failed");
}
