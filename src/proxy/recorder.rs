//! Full recordings of dialogs and messages, for studying a game feature
//! before writing a helper for it (the packet log cuts long dialogs at ~750
//! characters).
//!
//! - Opening the Legendary Wizard starts a recording to `wizard_log.txt` by
//!   itself; it stops when you leave the world.
//! - `/record [on|off]` records anything to `record_log.txt`.
//!
//! Recorded: every dialog the server opens (full markup), console lines, talk
//! bubbles and overlays, and what you send back (`dialog_return`, chat and
//! commands, other `action|` lines). Login packets are never recorded.

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use super::commands::say;
use super::interceptor::Interceptor;

const WIZARD_FILE: &str = "wizard_log.txt";
const MANUAL_FILE: &str = "record_log.txt";
/// A recording file is started over once it gets this big.
const MAX_FILE: u64 = 8 * 1024 * 1024;
/// Text that marks the Legendary Wizard's window.
const WIZARD_MARK: &str = "The Legendary Wizard";

#[derive(Debug, Default)]
pub struct Recorder {
    /// The file being written, `None` when not recording.
    file: Option<&'static str>,
    /// Started by the Wizard (stops when leaving the world).
    auto: bool,
}

fn now() -> String {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    // HH:MM:SS UTC is enough to follow a session.
    format!("{:02}:{:02}:{:02}", t / 3600 % 24, t / 60 % 60, t % 60)
}

fn write(file: &str, text: &str) {
    if cfg!(test) {
        return;
    }
    if std::fs::metadata(file).is_ok_and(|m| m.len() > MAX_FILE) {
        let _ = std::fs::remove_file(file);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
        let _ = f.write_all(text.as_bytes());
    }
}

fn start(ic: &mut Interceptor, file: &'static str, auto: bool) {
    ic.recorder.file = Some(file);
    ic.recorder.auto = auto;
    let world = ic.session.world.clone().unwrap_or_else(|| "-".into());
    write(file, &format!("\n===== recording started {} in {world} =====\n", now()));
}

fn stop(ic: &mut Interceptor) {
    if let Some(file) = ic.recorder.file.take() {
        write(file, &format!("===== recording stopped {} =====\n", now()));
    }
    ic.recorder.auto = false;
}

/// A function call from the server: dialogs, console lines, bubbles, overlays.
pub fn server_call(ic: &mut Interceptor, function: &str, text: &str) {
    if function == "OnDialogRequest" && text.contains(WIZARD_MARK) && ic.recorder.file.is_none() {
        start(ic, WIZARD_FILE, true);
        say(ic, format!("Recording the Legendary Wizard to `w{WIZARD_FILE}`` until you leave the world"));
    }
    let Some(file) = ic.recorder.file else { return };
    write(file, &format!("--- {} server {function}\n{}\n", now(), text.trim_end()));
}

/// Text the client sends. Only `action|` lines: login packets never start so.
pub fn client_text(ic: &mut Interceptor, text: &str) {
    let Some(file) = ic.recorder.file else { return };
    if !text.starts_with("action|") || text.contains("tankIDPass") {
        return;
    }
    write(file, &format!("--- {} client\n{}\n", now(), text.trim_end_matches(['\0', '\n'])));
}

/// The world was left: a Wizard recording ends here.
pub fn left_world(ic: &mut Interceptor) {
    if ic.recorder.auto {
        stop(ic);
    }
}

/// `/record [on|off]`.
pub fn record(ic: &mut Interceptor, args: &[&str]) {
    let on = match args.first().map(|a| a.to_ascii_lowercase()).as_deref() {
        Some("on") => true,
        Some("off") => false,
        _ => ic.recorder.file.is_none(),
    };
    if on {
        if ic.recorder.file.is_some() {
            stop(ic);
        }
        start(ic, MANUAL_FILE, false);
        say(ic, format!("Recording dialogs and messages to `w{MANUAL_FILE}``. `w/record`` again to stop."));
    } else if let Some(file) = ic.recorder.file {
        stop(ic);
        say(ic, format!("Recording stopped, saved in `w{file}``"));
    } else {
        say(ic, "Nothing is being recorded");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::sync::Arc;

    fn ic() -> Interceptor {
        let mut cfg = Config::default();
        cfg.log.text = false;
        cfg.log.variant = false;
        let mut ic = Interceptor::new(Arc::new(cfg));
        ic.session.enter_world("WIZ".into());
        ic
    }

    #[test]
    fn the_wizard_starts_a_recording_that_ends_with_the_world() {
        let mut ic = ic();
        server_call(&mut ic, "OnConsoleMessage", "hello");
        assert!(ic.recorder.file.is_none());
        server_call(&mut ic, "OnDialogRequest", "add_label_with_icon|big|`9The Legendary Wizard``|left|1790|\nend_dialog|x|||");
        assert_eq!(ic.recorder.file, Some(WIZARD_FILE));
        left_world(&mut ic);
        assert!(ic.recorder.file.is_none());
    }

    #[test]
    fn manual_recording_toggles_and_survives_world_changes() {
        let mut ic = ic();
        record(&mut ic, &[]);
        assert_eq!(ic.recorder.file, Some(MANUAL_FILE));
        left_world(&mut ic);
        assert_eq!(ic.recorder.file, Some(MANUAL_FILE));
        record(&mut ic, &[]);
        assert!(ic.recorder.file.is_none());
    }
}
