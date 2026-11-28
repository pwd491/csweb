// worker.rs

use std::cell::Cell;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use dpi::PhysicalSize;
use libservo::{
    DeviceIntPoint, DeviceIntRect, InputEvent, KeyState, Modifiers, MouseButtonAction,
    MouseButtonEvent, MouseMoveEvent, RenderingContext, Servo, ServoBuilder,
    SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate, WheelDelta, WheelEvent,
    WheelMode,
};
use url::Url;

use crate::input::{key_event, modifiers_from_bits, mouse_button, point, resolve_key};
use crate::types::{Cmd, Shared, lock};

const POLL_INTERVAL: Duration = Duration::from_millis(4);
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);

struct Delegate {
    frame_ready: Cell<bool>,
}

impl WebViewDelegate for Delegate {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.frame_ready.set(true);
    }

    fn notify_load_status_changed(&self, _webview: WebView, status: libservo::LoadStatus) {
        log::info!("load status: {status:?}");
    }

    fn notify_url_changed(&self, _webview: WebView, url: Url) {
        log::info!("url: {url}");
    }

    fn notify_page_title_changed(&self, _webview: WebView, title: Option<String>) {
        log::info!("title: {title:?}");
    }
}

struct WorkerView {
    webview: WebView,
    delegate: Rc<Delegate>,
    shared: Arc<Mutex<Shared>>,
    last_paint: Instant,
    context: Rc<SoftwareRenderingContext>,
}

struct Worker {
    servo: Option<&'static Servo>,
    views: HashMap<u64, WorkerView>,
}

impl Worker {
    fn new() -> Self {
        Self {
            servo: None,
            views: HashMap::new(),
        }
    }

    fn servo(&mut self) -> &'static Servo {
        *self
            .servo
            .get_or_insert_with(|| Box::leak(Box::new(ServoBuilder::default().build())))
    }

    fn create(
        &mut self,
        id: u64,
        width: u32,
        height: u32,
        url: Url,
        shared: Arc<Mutex<Shared>>,
    ) -> Result<(), String> {
        let context = Rc::new(
            SoftwareRenderingContext::new(PhysicalSize::new(width, height))
                .map_err(|e| format!("Rendering context creation failed: {e:?}"))?,
        );
        context
            .make_current()
            .map_err(|e| format!("make_current failed: {e:?}"))?;

        let servo = self.servo();
        let delegate = Rc::new(Delegate {
            frame_ready: Cell::new(true),
        });
        let rendering_context: Rc<dyn RenderingContext> = context.clone();

        log::info!("create view {id}: {width}x{height} {url}");
        let webview = WebViewBuilder::new(servo, rendering_context)
            .url(url)
            .delegate(delegate.clone())
            .build();
        webview.show();
        webview.set_focused(true);
        servo.spin_event_loop();
        log::info!("view {id} built, first spin done");

        self.views.insert(
            id,
            WorkerView {
                webview,
                delegate,
                shared,
                last_paint: Instant::now() - MIN_FRAME_INTERVAL,
                context,
            },
        );
        Ok(())
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Create {
                id,
                width,
                height,
                url,
                shared,
                reply,
            } => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    self.create(id, width, height, url, shared)
                }))
                .unwrap_or_else(|_| Err("Panic while creating view".to_owned()));
                let _ = reply.send(result);
            }
            Cmd::Destroy { id, done } => {
                self.views.remove(&id); // drops webview first, context last
                let _ = done.send(());
            }
            Cmd::Resize { id, width, height } => {
                if let Some(v) = self.views.get(&id) {
                    let size = PhysicalSize::new(width, height);
                    let _ = v.context.make_current();
                    v.webview.resize(size);
                    v.delegate.frame_ready.set(true);
                    log::info!("view {id} resized to {width}x{height}");
                    log::info!("scale factor: {:?}",v.webview.hidpi_scale_factor());
                }
            }
            Cmd::Load { id, url } => {
                if let Some(v) = self.views.get(&id) {
                    v.webview.load(url);
                }
            }
            Cmd::MouseMove { id, x, y } => {
                self.input(id, InputEvent::MouseMove(MouseMoveEvent::new(point(x, y))));
            }
            Cmd::MouseButton {
                id,
                button,
                pressed,
                x,
                y,
            } => {
                let action = if pressed {
                    MouseButtonAction::Down
                } else {
                    MouseButtonAction::Up
                };
                let event = MouseButtonEvent::new(action, mouse_button(button), point(x, y));
                self.input(id, InputEvent::MouseButton(event));
            }
            Cmd::Wheel { id, dx, dy, x, y } => {
                let delta = WheelDelta {
                    x: dx as f64,
                    y: dy as f64,
                    z: 0.0,
                    mode: WheelMode::DeltaLine,
                };
                let event = WheelEvent::new(delta, point(x, y), Modifiers::empty());
                self.input(id, InputEvent::Wheel(event));
            }
            Cmd::Key {
                id,
                down,
                codepoint,
                special,
                mods,
            } => {
                let Some(key) = resolve_key(codepoint, special) else {
                    return;
                };
                let state = if down { KeyState::Down } else { KeyState::Up };
                self.input(id, key_event(state, key, modifiers_from_bits(mods)));
            }
        }
    }

    fn input(&mut self, id: u64, event: InputEvent) {
        if let Some(v) = self.views.get(&id) {
            v.webview.notify_input_event(event);
        }
    }

    /// Pump Servo and publish new frames.
    fn tick(&mut self) {
        let Some(servo) = self.servo else { return };

        // With several views the "current" context is arbitrary; any of them will do for the spin.
        if let Some(v) = self.views.values().next() {
            let _ = v.context.make_current();
        }
        servo.spin_event_loop();

        let now = Instant::now();
        for v in self.views.values_mut() {
            if !v.delegate.frame_ready.get() {
                continue;
            }
            if now.duration_since(v.last_paint) < MIN_FRAME_INTERVAL {
                continue; // stay "ready", paint on a later tick
            }
            v.delegate.frame_ready.set(false);
            v.last_paint = now;
            paint_view(v);
        }
    }
}

fn paint_view(v: &mut WorkerView) {
    if let Err(e) = v.context.make_current() {
        log::warn!("paint: make_current failed: {e:?}");
        return;
    }
    v.webview.paint();

    let size = v.context.size();
    let rect = DeviceIntRect::new(
        DeviceIntPoint::new(0, 0),
        DeviceIntPoint::new(size.width as i32, size.height as i32),
    );
    let Some(image) = v.context.read_to_image(rect) else {
        log::warn!("read_to_image returned None ({}x{})", size.width, size.height);
        return;
    };

    let mut shared = lock(&v.shared);
    shared.pixels = image.into_raw();
    shared.width = size.width;
    shared.height = size.height;
    shared.frame_id += 1;
}

fn worker_main(rx: Receiver<Cmd>) {
    let mut worker = Worker::new();

    loop {
        let first = match rx.recv_timeout(POLL_INTERVAL) {
            Ok(cmd) => Some(cmd),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        let mut pending = first;
        while let Some(cmd) = pending.take().or_else(|| rx.try_recv().ok()) {
            if catch_unwind(AssertUnwindSafe(|| worker.handle(cmd))).is_err() {
                log::error!("panic while handling a command");
            }
        }

        if catch_unwind(AssertUnwindSafe(|| worker.tick())).is_err() {
            log::error!("panic while pumping Servo");
        }
    }

    // Servo must never be dropped (it cannot be created twice in one process).
    std::mem::forget(worker);
}

static WORKER: OnceLock<Option<Sender<Cmd>>> = OnceLock::new();

pub(crate) fn worker() -> Option<&'static Sender<Cmd>> {
    WORKER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel();
            thread::Builder::new()
                .name("cswebservo".to_owned())
                .stack_size(16 * 1024 * 1024)
                .spawn(move || worker_main(rx))
                .ok()
                .map(|_| tx)
        })
        .as_ref()
}