//! Keeps the latest log lines in memory for the window's Log tab.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::Mutex;

const MAX_LINES: usize = 3000;

static LINES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

pub fn lines() -> Vec<String> {
    LINES.lock().map(|l| l.iter().cloned().collect()).unwrap_or_default()
}

pub fn clear() {
    if let Ok(mut l) = LINES.lock() {
        l.clear();
    }
}

/// One log event; tracing writes it in pieces, it is stored when dropped.
#[derive(Default)]
pub struct Writer(Vec<u8>);

impl Write for Writer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.0).trim_end().to_string();
        if text.is_empty() {
            return;
        }
        if let Ok(mut l) = LINES.lock() {
            l.push_back(text);
            while l.len() > MAX_LINES {
                l.pop_front();
            }
        }
    }
}

pub fn writer() -> Writer {
    Writer::default()
}
