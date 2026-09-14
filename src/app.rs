use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::document::{self, Document};
use crate::platform;
use crate::render::{self, Renderer};
use crate::UserEvent;

const DEFAULT_SIZE: LogicalSize<f64> = LogicalSize::new(800.0, 600.0);
const MIN_SIZE: LogicalSize<f64> = LogicalSize::new(200.0, 200.0);

pub struct App {
    pending_path: Option<PathBuf>,
    document: Option<Document>,
    error: Option<String>,
    renderer: Renderer,
    // Drop surface before context/window (struct fields drop in declaration order).
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    context: Option<softbuffer::Context<Rc<Window>>>,
    window: Option<Rc<Window>>,
}

impl App {
    pub fn new(path: Option<PathBuf>) -> Self {
        let mut app = Self {
            pending_path: None,
            document: None,
            error: None,
            renderer: Renderer::new(),
            surface: None,
            context: None,
            window: None,
        };
        if let Some(path) = path {
            app.load_path(path);
        }
        app
    }

    fn load_path(&mut self, path: PathBuf) {
        match document::load(&path) {
            Ok(document) => {
                self.error = None;
                self.document = Some(document);
            }
            Err(err) => {
                eprintln!("savage: {err}");
                self.error = Some(err.to_string());
                self.document = None;
            }
        }
        self.sync_window();
    }

    fn sync_window(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        window.set_title(&document::title_for(
            self.document.as_ref().map(|d| d.path.as_path()),
            self.error.as_deref(),
        ));
        window.request_redraw();
    }

    fn ensure_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let size = window_size(self.document.as_ref(), event_loop);
        let attrs = Window::default_attributes()
            .with_title(document::title_for(
                self.document.as_ref().map(|d| d.path.as_path()),
                self.error.as_deref(),
            ))
            .with_inner_size(size)
            .with_min_inner_size(MIN_SIZE)
            .with_transparent(false)
            .with_visible(true);

        let window = Rc::new(
            event_loop
                .create_window(attrs)
                .expect("failed to create window"),
        );
        let context =
            softbuffer::Context::new(window.clone()).expect("failed to create surface context");
        let surface = softbuffer::Surface::new(&context, window.clone())
            .expect("failed to create software surface");

        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);
    }

    fn paint(&mut self) {
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return;
        };
        let size = window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return;
        };
        if surface.resize(width, height).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };

        if let Some(document) = &self.document {
            self.renderer
                .rasterize(&document.tree, width.get(), height.get(), &mut buffer);
        } else {
            render::fill_empty(&mut buffer);
        }

        let _ = buffer.present();
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        self.ensure_window(event_loop);
        platform::on_ready();
        if let Some(path) = self.pending_path.take() {
            self.load_path(path);
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Open(path) => {
                if self.window.is_some() {
                    self.load_path(path);
                } else {
                    self.pending_path = Some(path);
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().is_none_or(|w| w.id() != window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::DroppedFile(path) => self.load_path(path),
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat =>
            {
                match event.logical_key {
                    Key::Named(NamedKey::Escape | NamedKey::Space) => event_loop.exit(),
                    Key::Character(ch) if ch.eq_ignore_ascii_case("q") => event_loop.exit(),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn window_size(document: Option<&Document>, event_loop: &ActiveEventLoop) -> LogicalSize<f64> {
    let Some(document) = document else {
        return DEFAULT_SIZE;
    };

    let svg_w = f64::from(document.tree.size().width()).max(1.0);
    let svg_h = f64::from(document.tree.size().height()).max(1.0);
    let (max_w, max_h) = monitor_logical_max(event_loop);
    let scale = (max_w / svg_w).min(max_h / svg_h).min(1.0);

    LogicalSize::new(
        (svg_w * scale).max(MIN_SIZE.width),
        (svg_h * scale).max(MIN_SIZE.height),
    )
}

fn monitor_logical_max(event_loop: &ActiveEventLoop) -> (f64, f64) {
    let monitor = event_loop
        .primary_monitor()
        .or_else(|| event_loop.available_monitors().next());
    let Some(monitor) = monitor else {
        return (1280.0, 800.0);
    };
    let PhysicalSize { width, height } = monitor.size();
    let scale = monitor.scale_factor().max(1.0);
    (
        f64::from(width) / scale * 0.9,
        f64::from(height) / scale * 0.9,
    )
}
