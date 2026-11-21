// ffi.rs

use std::ffi::{CStr, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use url::Url;

use crate::console::{self, LogLevel};
use crate::types::{ABI_VERSION, Cmd, ServoViewConfig, ServoViewFrame, Shared, lock};
use crate::worker::worker;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct ServoView {
    id: u64,
    tx: Sender<Cmd>,
    shared: Arc<Mutex<Shared>>,
    /// Buffer handed out to the caller; valid until the next acquire.
    front: Vec<u8>,
    last_frame_id: u64,
}

fn read_c_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr).to_str().ok().map(str::to_owned) }
}

fn send(view: *mut ServoView, make: impl FnOnce(u64) -> Cmd) {
    if view.is_null() {
        return;
    }
    let view = unsafe { &*view };
    let _ = view.tx.send(make(view.id));
}

fn create_view(cfg: &ServoViewConfig) -> Result<Box<ServoView>, String> {
    if cfg.abi_version != ABI_VERSION {
        return Err(format!("Unsupported ABI version: {}", cfg.abi_version));
    }
    if cfg.width == 0 || cfg.height == 0 {
        return Err("Invalid viewport size".to_owned());
    }
    let url_string = read_c_string(cfg.url).unwrap_or_else(|| "about:blank".to_owned());
    let url = Url::parse(&url_string).map_err(|e| format!("Invalid URL: {e}"))?;

    let tx = worker()
        .ok_or_else(|| "Failed to start the Servo thread".to_owned())?
        .clone();
    let shared = Arc::new(Mutex::new(Shared::default()));
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);

    let (reply_tx, reply_rx) = mpsc::channel();
    tx.send(Cmd::Create {
        id,
        width: cfg.width,
        height: cfg.height,
        url,
        shared: shared.clone(),
        reply: reply_tx,
    })
    .map_err(|_| "The Servo thread is not running".to_owned())?;

    // Blocks until Servo and the rendering context are up (first call is slow).
    reply_rx
        .recv()
        .map_err(|_| "The Servo thread died while creating the view".to_owned())??;

    Ok(Box::new(ServoView {
        id,
        tx,
        shared,
        front: Vec::new(),
        last_frame_id: 0,
    }))
}

#[unsafe(no_mangle)]
pub extern "C" fn cs_web_abi_version() -> u32 {
    ABI_VERSION
}

/// Whether the library records its log at all, see console.rs (0 = no, otherwise yes).
#[unsafe(no_mangle)]
pub extern "C" fn cs_web_set_debug_level(level: i32) {
    console::set_debug_level(level);
}

#[unsafe(no_mangle)]
pub extern "C" fn cs_web_create(cfg: *const ServoViewConfig) -> *mut ServoView {
    if cfg.is_null() {
        return std::ptr::null_mut();
    }
    let cfg = unsafe { &*cfg };
    console::install(cfg.log);

    let view = match catch_unwind(AssertUnwindSafe(|| create_view(cfg))) {
        Ok(Ok(view)) => Box::into_raw(view),
        Ok(Err(error)) => {
            console::push(LogLevel::Error, error);
            std::ptr::null_mut()
        }
        Err(_) => {
            console::push(LogLevel::Error, "panic while creating view".to_owned());
            std::ptr::null_mut()
        }
    };
    console::drain();
    view
}

#[unsafe(no_mangle)]
pub extern "C" fn cs_web_destroy(view: *mut ServoView) {
    if view.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let view = unsafe { Box::from_raw(view) };
        let (done_tx, done_rx) = mpsc::channel();
        if view
            .tx
            .send(Cmd::Destroy { id: view.id, done: done_tx })
            .is_ok()
        {
            let _ = done_rx.recv_timeout(Duration::from_secs(5));
        }
    }));
    console::drain();
}

#[unsafe(no_mangle)]
pub extern "C" fn cs_web_resize(view: *mut ServoView, width: u32, height: u32) {
    if width == 0 || height == 0 {
        return;
    }
    send(view, |id| Cmd::Resize { id, width, height });
}

#[unsafe(no_mangle)]
pub extern "C" fn cs_web_load_url(view: *mut ServoView, url: *const c_char) {
    let Some(text) = read_c_string(url) else { return };
    match Url::parse(&text) {
        Ok(url) => send(view, |id| Cmd::Load { id, url }),
        Err(error) => console::push(LogLevel::Error, format!("invalid URL {text:?}: {error}")),
    }
    console::drain();
}

#[unsafe(no_mangle)]
pub extern "C" fn cs_web_acquire(view: *mut ServoView, out: *mut ServoViewFrame) -> i32 {
    // Called every frame on the engine thread: the place to deliver queued log messages.
    console::drain();

    if view.is_null() || out.is_null() {
        return 0;
    }
    let result = catch_unwind(AssertUnwindSafe(|| -> i32 {
        let view = unsafe { &mut *view };

        let mut shared = lock(&view.shared);
        if shared.frame_id == view.last_frame_id || shared.pixels.is_empty() {
            return 0;
        }
        // Take the new frame without copying; the worker gets our old buffer back and overwrites it.
        std::mem::swap(&mut view.front, &mut shared.pixels);
        view.last_frame_id = shared.frame_id;
        let (width, height) = (shared.width, shared.height);
        drop(shared);

        unsafe {
            *out = ServoViewFrame {
                gl_texture: 0,
                width,
                height,
                frame_id: view.last_frame_id,
                flipped_y: 0,
                pixels: view.front.as_ptr(),
                stride: width * 4,
            };
        }
        1
    }));
    result.unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn cs_web_mouse_move(view: *mut ServoView, x: f32, y: f32) {
    send(view, |id| Cmd::MouseMove { id, x, y });
}

#[unsafe(no_mangle)]
pub extern "C" fn cs_web_mouse_button(
    view: *mut ServoView,
    button: i32,
    pressed: i32,
    x: f32,
    y: f32,
) {
    send(view, |id| Cmd::MouseButton {
        id,
        button,
        pressed: pressed != 0,
        x,
        y,
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn cs_web_wheel(view: *mut ServoView, dx: f32, dy: f32, x: f32, y: f32) {
    send(view, |id| Cmd::Wheel { id, dx, dy, x, y });
}

#[unsafe(no_mangle)]
pub extern "C" fn cs_web_key(
    view: *mut ServoView,
    down: i32,
    codepoint: u32,
    special: i32,
    mods: u32,
) {
    send(view, |id| Cmd::Key {
        id,
        down: down != 0,
        codepoint,
        special,
        mods,
    });
}