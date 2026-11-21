// types.rs

use std::ffi::{c_char, c_void};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};

use url::Url;

pub(crate) const ABI_VERSION: u32 = 1;

pub type LogCallback = Option<unsafe extern "C" fn(level: i32, msg: *const c_char, ...)>;

#[repr(C)]
pub struct ServoViewConfig {
    pub abi_version: u32,
    pub width: u32,
    pub height: u32,
    pub url: *const c_char,
    pub resources_dir: *const c_char,
    pub native_display: *mut c_void,
    pub share_context: *mut c_void,
    pub log: LogCallback,
}

#[repr(C)]
pub struct ServoViewFrame {
    pub gl_texture: u32,
    pub width: u32,
    pub height: u32,
    pub frame_id: u64,
    pub flipped_y: i32,
    pub pixels: *const u8,
    pub stride: u32,
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Latest frame produced by the worker.
#[derive(Default)]
pub(crate) struct Shared {
    pub(crate) pixels: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) frame_id: u64,
}

pub(crate) enum Cmd {
    Create {
        id: u64,
        width: u32,
        height: u32,
        url: Url,
        shared: Arc<Mutex<Shared>>,
        reply: Sender<Result<(), String>>,
    },
    Destroy {
        id: u64,
        done: Sender<()>,
    },
    Resize {
        id: u64,
        width: u32,
        height: u32,
    },
    Load {
        id: u64,
        url: Url,
    },
    MouseMove {
        id: u64,
        x: f32,
        y: f32,
    },
    MouseButton {
        id: u64,
        button: i32,
        pressed: bool,
        x: f32,
        y: f32,
    },
    Wheel {
        id: u64,
        dx: f32,
        dy: f32,
        x: f32,
        y: f32,
    },
    Key {
        id: u64,
        down: bool,
        codepoint: u32,
        special: i32,
        mods: u32,
    },
}