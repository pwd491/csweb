//! console.rs: logging of the library through the engine console.
//!
//! The engine console (Con_Printf) is not thread safe, and Servo logs from many
//! threads. So nothing here calls the engine directly: records (from the `log`
//! crate, from our own code and from the panic hook) go into a queue, and the
//! engine thread delivers them to the callback passed in `ServoViewConfig::log`
//! from inside `cs_web_*` calls (`drain`).
//!
//! Filtering by level is done by the client (see csw_log.h): only Info is always
//! printed, the rest needs csweb_debug 1. `set_debug_level` only decides whether
//! the `log` crate records anything at all, so nothing is queued needlessly.

use std::collections::VecDeque;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};

use crate::types::{LogCallback, lock};

/// Keep in sync with `csw::LogLevel` in cl_dll/cs_web/csw_log.h.
#[repr(i32)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum LogLevel {
    Info = 0, // printed by the client itself, never sent from here
    Debug = 1,
    Warn = 2,
    Error = 3,
    Panic = 4,
}

struct Entry {
    level: LogLevel,
    text: String,
}

/// Oldest messages are dropped when the engine does not drain for a long time.
const QUEUE_CAP: usize = 1024;

static QUEUE: Mutex<VecDeque<Entry>> = Mutex::new(VecDeque::new());
static CALLBACK: Mutex<LogCallback> = Mutex::new(None);
static PENDING: AtomicBool = AtomicBool::new(false);
static INSTALL: Once = Once::new();
static LOGGER: Logger = Logger;

struct Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        let level = match record.level() {
            log::Level::Error => LogLevel::Error,
            log::Level::Warn => LogLevel::Warn,
            _ => LogLevel::Debug,
        };
        push(level, format!("{}: {}", record.target(), record.args()));
    }

    fn flush(&self) {}
}

/// Remember the engine callback and (once) install the logger and the panic hook.
pub(crate) fn install(callback: LogCallback) {
    if callback.is_some() {
        *lock(&CALLBACK) = callback;
    }

    INSTALL.call_once(|| {
        let _ = log::set_logger(&LOGGER);

        std::panic::set_hook(Box::new(|info| {
            let thread = std::thread::current();
            let name = thread.name().unwrap_or("<unnamed>");
            push(LogLevel::Panic, format!("thread '{name}': {info}"));
        }));
    });
}

/// csweb_debug: 0 = record nothing from `log`, 1 = record everything up to info level.
pub(crate) fn set_debug_level(level: i32) {
    log::set_max_level(if level <= 0 {
        log::LevelFilter::Off
    } else {
        log::LevelFilter::Info
    });
}

pub(crate) fn push(level: LogLevel, text: String) {
    let mut queue = lock(&QUEUE);
    if queue.len() >= QUEUE_CAP {
        queue.pop_front();
    }
    queue.push_back(Entry { level, text });
    PENDING.store(true, Ordering::Release);
}

/// Deliver queued messages to the engine. Must only be called from `cs_web_*`
/// entry points, i.e. on the engine thread.
pub(crate) fn drain() {
    if !PENDING.swap(false, Ordering::AcqRel) {
        return;
    }

    let Some(callback) = *lock(&CALLBACK) else {
        PENDING.store(true, Ordering::Release); // nobody to deliver to yet
        return;
    };

    let entries: Vec<Entry> = lock(&QUEUE).drain(..).collect();

    for entry in entries {
        if let Ok(text) = CString::new(entry.text.replace('\0', " ")) {
            // The callback is printf-like; "%s" keeps '%' in messages harmless.
            unsafe { callback(entry.level as i32, c"%s".as_ptr(), text.as_ptr()) };
        }
    }
}