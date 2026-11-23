//! A minimal [`log`] backend that writes to a file and to the Windows debug output.

use std::{
    fs::File,
    io::Write,
    path::Path,
    sync::{Mutex, PoisonError},
    time::{SystemTime, UNIX_EPOCH},
};

use log::{LevelFilter, Log, Metadata, Record};
use windows::{Win32::System::Diagnostics::Debug::OutputDebugStringW, core::PCWSTR};

struct FileLogger(Mutex<File>);

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.target().starts_with(env!("CARGO_CRATE_NAME"))
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64();
        let message = format!("{:<5} {}", record.level(), record.args());

        let debug_line: Vec<u16> = format!("[drawn_arts] {message}\n").encode_utf16().chain(Some(0)).collect();
        unsafe { OutputDebugStringW(PCWSTR(debug_line.as_ptr())) };

        let mut file = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        // Logging must never take the game down, so write errors are ignored.
        let _ = writeln!(file, "[{secs:.3}] {message}");
    }

    fn flush(&self) {
        let _ = self.0.lock().unwrap_or_else(PoisonError::into_inner).flush();
    }
}

/// Starts logging at info level to `path`, replacing its previous contents, and to the debug
/// output. Does nothing if the file can't be created.
pub fn init(path: &Path) {
    let Ok(file) = File::create(path) else { return };
    let logger = Box::leak(Box::new(FileLogger(Mutex::new(file))));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }
}
