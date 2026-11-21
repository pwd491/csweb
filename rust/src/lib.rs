//! lib.rs: C ABI around Servo for embedding into CS16Client

mod console;
mod ffi;
mod input;
mod types;
mod worker;

pub use ffi::{
    ServoView, cs_web_abi_version, cs_web_acquire, cs_web_create, cs_web_destroy, cs_web_key,
    cs_web_load_url, cs_web_mouse_button, cs_web_mouse_move, cs_web_resize,
    cs_web_set_debug_level, cs_web_wheel,
};
pub use types::{LogCallback, ServoViewConfig, ServoViewFrame};