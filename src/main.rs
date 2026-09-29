mod gpu;

use book_writing_app::cli::Args;
use gpu::{Gpu, WindowGpu};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

#[derive(Default)]
struct App {
    window: Option<Arc<Window>>,
    gpu: Option<WindowGpu>,
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
            WindowEvent::RedrawRequested => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.draw();
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

    if let Some(path) = &args.screenshot {
        let (width, height) = args.size;
        let pixels = Gpu::headless().screenshot(width, height);
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
        .run_app(&mut App::default())
        .expect("event loop failed");
}
