mod gpu;
mod rects;
mod typeset;

use gpu::{Frame, Gpu, Layout, WindowGpu};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use vim_book_writer::app::{App, Effect, ExportKind, Screen};
use vim_book_writer::cli::Args;
use vim_book_writer::config::{self, Config};
use vim_book_writer::editor::Editor;
use vim_book_writer::input;
use vim_book_writer::recent::Recent;
use vim_book_writer::scroll::Scroll;
use vim_book_writer::toolbar::{self, Click};
use vim_book_writer::ui::Bounds;
use vim_book_writer::vim::{Key, Mode, ViewMove};
use vim_book_writer::watch::FileWatcher;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

/// How much of the text to keep in view around the caret, and how far one wheel notch scrolls.
const PADDING: f32 = 54.0;
const WHEEL_NOTCH: f32 = 81.0;

/// Something that happened outside the window, for the event loop to hear about.
enum UserEvent {
    /// The open file may have changed on disk.
    FileChanged,
}

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
    /// Where to put the cursor's line in the view, once the next frame has laid it out.
    align: Option<ViewMove>,
    /// The count of files opened as of the last frame, to start at the top of a new one.
    opened: u64,
    /// The system clipboard, kept for as long as the app runs because on some systems the text
    /// on it lives only while its owner does. `None` where there is no clipboard.
    clipboard: Option<arboard::Clipboard>,
    proxy: EventLoopProxy<UserEvent>,
    /// The file being watched for changes from outside, and the watch.
    watching: Option<(PathBuf, FileWatcher)>,
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
        let theme = self.app.theme();
        let reading = &self.app.reading;
        if self.app.screen != Screen::Editing {
            gpu.draw(&Frame {
                view: None,
                editor: Bounds::default(),
                scroll: 0.0,
                chrome: &chrome,
                theme,
                reading,
            });
            return;
        }
        if self.opened != self.app.opened {
            self.opened = self.app.opened;
            self.scroll = Scroll::default();
            self.seen = None;
        }
        let regions = self.app.regions(size);
        let view = self.app.editor.view();
        self.layout = gpu.gpu.layout(&view, regions.editor, theme, reading);
        let seen = (view.version, view.cursor);
        let viewport = regions.editor.h;
        if let Some(by) = self.align.take() {
            if let Some((top, bottom)) = self.layout.caret {
                let position = match by {
                    ViewMove::Top => top - PADDING,
                    ViewMove::Bottom => bottom + PADDING - viewport,
                    _ => (top + bottom - viewport) / 2.0,
                };
                self.scroll
                    .to(position, self.layout.content_height, viewport);
            }
            // The view was put where it should be, so don't move it to the cursor as well.
            self.seen = Some(seen);
        } else if self.seen != Some(seen) {
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
            theme,
            reading,
        });
        // Up and down follow the rows this frame wrapped the text into.
        self.app.editor.set_rows(&gpu.gpu.rows);
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
                Effect::SetClipboard(text) => {
                    if let Some(clipboard) = &mut self.clipboard {
                        let _ = clipboard.set_text(text);
                    }
                }
                Effect::ReadClipboard(purpose) => {
                    let text = self
                        .clipboard
                        .as_mut()
                        .and_then(|clipboard| clipboard.get_text().ok())
                        .unwrap_or_default();
                    self.app.clipboard_text(purpose, text);
                }
                Effect::ScrollRows(rows) => {
                    let viewport = self.app.regions(self.size()).editor.h;
                    self.scroll.by(
                        rows as f32 * self.app.reading.row(),
                        self.layout.content_height,
                        viewport,
                    );
                }
                // The cursor may have moved, so wait for the next layout to know where it is.
                Effect::Align(by) => self.align = Some(by),
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

impl ApplicationHandler<UserEvent> for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = Window::default_attributes()
            .with_title("Vim Book Writer")
            .with_window_icon(window_icon());
        // The app id is how Wayland finds the icon and X11 the window class, through the
        // `.desktop` file that `desktop::install` wrote.
        #[cfg(target_os = "linux")]
        let attributes = {
            use winit::platform::{
                wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11,
            };
            let id = vim_book_writer::desktop::APP_ID;
            let attributes = WindowAttributesExtWayland::with_name(attributes, id, id);
            WindowAttributesExtX11::with_name(attributes, id, id)
        };
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
            WindowEvent::Focused(true) => {
                self.app.focus_gained();
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::FileChanged => {
                if self.app.file_changed_on_disk() {
                    self.request_redraw();
                }
            }
        }
    }

    /// After a batch of events, and when auto-save's wait is over: save if it is time, and sleep
    /// until the next time it could be.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Watch whichever file is open now.
        let open = self
            .app
            .editor
            .path()
            .and_then(|path| std::fs::canonicalize(path).ok());
        if self.watching.as_ref().map(|(path, _)| path) != open.as_ref() {
            self.watching = open.and_then(|path| {
                let proxy = self.proxy.clone();
                let watcher = FileWatcher::new(&path, move || {
                    let _ = proxy.send_event(UserEvent::FileChanged);
                })
                .ok()?;
                Some((path, watcher))
            });
        }
        let modified = self.app.editor.buf.is_modified();
        if self.app.tick(Instant::now()) || modified != self.app.editor.buf.is_modified() {
            self.request_redraw();
        }
        event_loop.set_control_flow(match self.app.next_wake() {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }
}

/// The logo, as the icon for the window's title bar and task switcher.
fn window_icon() -> Option<winit::window::Icon> {
    let bytes = include_bytes!("../packaging/icons/256x256.png");
    let mut reader = png::Decoder::new(std::io::Cursor::new(&bytes[..]))
        .read_info()
        .ok()?;
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut pixels).ok()?;
    pixels.truncate(frame.buffer_size());
    winit::window::Icon::from_rgba(pixels, frame.width, frame.height).ok()
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

    config::migrate();
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
    config.dark |= args.dark;
    if let Some(font) = &args.font {
        config.font.clone_from(font);
    }
    config.size = args.pt.unwrap_or(config.size);
    app.set_config(config, config_path.filter(|_| !screenshot));
    app.tree_visible = args.tree;
    app.refresh_git();
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
            app.open_menu(menu, anchor, size);
        }
        if let Some(label) = &args.hover {
            let toolbar = toolbar::toolbar(regions.toolbar);
            app.hover = toolbar.buttons.iter().position(|b| &b.label == label);
            app.tip_shown = true;
        }
        let chrome = app.chrome(size);
        let theme = app.theme();
        let reading = &app.reading;
        let mut gpu = Gpu::headless();
        let pixels = if app.screen == Screen::Editing {
            let view = app.editor.view();
            // Scroll to the caret as the window would, but arrive at once.
            let layout = gpu.layout(&view, regions.editor, theme, reading);
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
                    theme,
                    reading,
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
                    theme,
                    reading,
                },
            )
        };
        write_png(path, args.size, &pixels);
        return;
    }

    // Give the window its icon and a menu entry, where the desktop wants them from a file.
    #[cfg(target_os = "linux")]
    let _ = vim_book_writer::desktop::install_for_user();

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("failed to create event loop");
    let proxy = event_loop.create_proxy();
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
            align: None,
            opened: 0,
            clipboard: arboard::Clipboard::new().ok(),
            proxy,
            watching: None,
        })
        .expect("event loop failed");
}
