//! Optional sharing with the web panel (`[report]` in config.toml, off by default).
//!
//! On by default (the user can turn it off in Settings). What is sent over HTTPS:
//! - vends: world name, tile, item, price, when seen (what any visitor sees),
//!   right after entering a world that has vending machines;
//! - spins: world, number, colour, time (no player names);
//! - errors: this proxy's own warning/error log lines, with anything that
//!   looks like a login value removed.
//!
//! Spins and errors go out in a batch about once a minute.
//!
//! Never sent: GrowID, password, tokens, MAC/RID, chat, player names. Each
//! install has a random ID (`report_id.txt`) so the server can rate-limit and
//! block a misbehaving install; it says nothing about the person.
//!
//! The proxy code only calls [`vends`] / [`spin`]; when sharing is off they
//! return at once. A failed send keeps the batch (up to a limit) for the next try.

use std::collections::VecDeque;
use std::sync::{Mutex, mpsc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::config::ReportConfig;

/// How often spins and errors go out.
const SEND_EVERY: Duration = Duration::from_secs(60);
/// After a world's vends arrive, wait this long for more (tile updates,
/// a quick world hop) and send them together.
const VEND_GATHER: Duration = Duration::from_secs(2);
/// Kept while the server cannot be reached; the oldest go first.
const MAX_WORLDS: usize = 200;
const MAX_SPINS: usize = 2000;
const MAX_ERRORS: usize = 200;
/// Longest error text sent.
const MAX_MESSAGE: usize = 400;
const ID_FILE: &str = "report_id.txt";

static VENDS_ON: AtomicBool = AtomicBool::new(false);
static SPINS_ON: AtomicBool = AtomicBool::new(false);
static ERRORS_ON: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
/// Wakes the sender when vends are waiting.
static NUDGE: Mutex<Option<mpsc::Sender<()>>> = Mutex::new(None);
static QUEUE: Mutex<Pending> = Mutex::new(Pending { worlds: VecDeque::new(), spins: VecDeque::new(), errors: VecDeque::new() });

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VendItem {
    pub x: i32,
    pub y: i32,
    pub item: u32,
    pub name: String,
    pub price: i32,
}

#[derive(Debug, Clone, Serialize)]
struct WorldVends {
    world: String,
    at: u64,
    items: Vec<VendItem>,
}

#[derive(Debug, Clone, Serialize)]
struct SpinRow {
    world: String,
    n: u8,
    c: char,
    at: u64,
}

#[derive(Debug, Clone, Serialize)]
struct ErrorRow {
    level: &'static str,
    msg: String,
    at: u64,
}

struct Pending {
    worlds: VecDeque<WorldVends>,
    spins: VecDeque<SpinRow>,
    errors: VecDeque<ErrorRow>,
}

#[derive(Serialize)]
struct Batch<'a> {
    v: u32,
    install: &'a str,
    version: &'static str,
    vends: Vec<WorldVends>,
    spins: Vec<SpinRow>,
    errors: Vec<ErrorRow>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn push_capped<T>(q: &mut VecDeque<T>, item: T, cap: usize) {
    if q.len() >= cap {
        q.pop_front();
    }
    q.push_back(item);
}

/// All vending machines of a world as just seen (replaces what the server has for it).
pub fn vends(world: &str, items: Vec<VendItem>) {
    if !VENDS_ON.load(Ordering::Relaxed) || world.is_empty() {
        return;
    }
    let Ok(mut q) = QUEUE.lock() else { return };
    // A newer look at the same world replaces the waiting one.
    q.worlds.retain(|w| w.world != world);
    push_capped(&mut q.worlds, WorldVends { world: world.to_string(), at: now(), items }, MAX_WORLDS);
    drop(q);
    if let Ok(n) = NUDGE.lock() {
        if let Some(tx) = n.as_ref() {
            let _ = tx.send(());
        }
    }
}

pub fn spin(world: &str, number: u8, colour: char) {
    if !SPINS_ON.load(Ordering::Relaxed) {
        return;
    }
    let Ok(mut q) = QUEUE.lock() else { return };
    push_capped(&mut q.spins, SpinRow { world: world.to_string(), n: number, c: colour, at: now() }, MAX_SPINS);
}

fn error(level: &'static str, msg: String) {
    if !ERRORS_ON.load(Ordering::Relaxed) {
        return;
    }
    let Ok(mut q) = QUEUE.lock() else { return };
    // The same line again and again (e.g. a reconnect loop) is sent once.
    if q.errors.iter().any(|e| e.msg == msg) {
        return;
    }
    push_capped(&mut q.errors, ErrorRow { level, msg, at: now() }, MAX_ERRORS);
}

/// Drops values that could identify the player from a log line: anything
/// after a login field name, IPv4 addresses of the player's side, long tokens.
pub fn scrub(text: &str) -> String {
    const SECRET_KEYS: &[&str] = &[
        "tankIDPass", "tankIDName", "token", "ltoken", "mac", "rid", "wk", "zf", "fz", "hash", "hash2", "meta", "klv",
        "gid", "sid", "vid", "aid", "uuid", "UUIDToken", "doorID", "user", "password", "growId", "name",
    ];
    let mut out = String::with_capacity(text.len());
    for word in text.split_inclusive(|c: char| c.is_whitespace() || c == ',' || c == '"') {
        let core = word.trim_end_matches(|c: char| c.is_whitespace() || c == ',' || c == '"');
        let tail = &word[core.len()..];
        let secret_field = core.split_once('|').is_some_and(|(k, _)| SECRET_KEYS.iter().any(|s| s.eq_ignore_ascii_case(k)))
            || core.split_once('=').is_some_and(|(k, _)| SECRET_KEYS.iter().any(|s| s.eq_ignore_ascii_case(k)));
        let long_token = core.len() >= 24 && core.chars().all(|c| c.is_ascii_alphanumeric() || "+/=_-".contains(c));
        if secret_field {
            let key = core.split(['|', '=']).next().unwrap_or_default();
            out.push_str(key);
            out.push_str("|***");
        } else if long_token {
            out.push_str("***");
        } else {
            out.push_str(core);
        }
        out.push_str(tail);
    }
    out.chars().take(MAX_MESSAGE).collect()
}

/// Turns sharing on as configured and starts the sender. Called once at start.
pub fn start(cfg: &ReportConfig) {
    // An empty address means the project's own panel.
    let url = if cfg.url.trim().is_empty() { crate::config::DEFAULT_REPORT_URL } else { cfg.url.trim() };
    let url = url.trim_end_matches('/').to_string();
    if !cfg.enabled || url.is_empty() || STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    if !url.starts_with("https://") {
        tracing::warn!("report.url must start with https://, sharing stays off");
        return;
    }
    VENDS_ON.store(cfg.vends, Ordering::Relaxed);
    SPINS_ON.store(cfg.spins, Ordering::Relaxed);
    ERRORS_ON.store(cfg.errors, Ordering::Relaxed);
    let install = install_id();
    tracing::info!("sharing with {url}: vends {}, spins {}, errors {}", cfg.vends, cfg.spins, cfg.errors);
    let (tx, rx) = mpsc::channel();
    if let Ok(mut n) = NUDGE.lock() {
        *n = Some(tx);
    }
    let spawned = std::thread::Builder::new().name("report".into()).spawn(move || sender(url, install, rx));
    if let Err(e) = spawned {
        tracing::warn!("report sender not started: {e}");
    }
}

/// The random ID of this install, made on first use.
fn install_id() -> String {
    if let Ok(id) = std::fs::read_to_string(ID_FILE) {
        let id = id.trim();
        if id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit()) {
            return id.to_string();
        }
    }
    use std::hash::{BuildHasher, Hasher};
    let mut id = String::new();
    for _ in 0..2 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        h.write_u32(std::process::id());
        id.push_str(&format!("{:016x}", h.finish()));
    }
    if let Err(e) = std::fs::write(ID_FILE, &id) {
        tracing::warn!("cannot save {ID_FILE}: {e}");
    }
    id
}

fn sender(url: String, install: String, nudge: mpsc::Receiver<()>) {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
    let client = match reqwest::Client::builder().timeout(Duration::from_secs(20)).user_agent(concat!("gtproxy/", env!("CARGO_PKG_VERSION"))).build() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("report client: {e}");
            return;
        }
    };
    let endpoint = format!("{url}/api/ingest.php");
    let mut next_full = Instant::now() + SEND_EVERY;
    loop {
        match nudge.recv_timeout(next_full.saturating_duration_since(Instant::now())) {
            // A world's vends: send them now, spins and errors keep their minute.
            Ok(()) => {
                std::thread::sleep(VEND_GATHER);
                while nudge.try_recv().is_ok() {}
                send(&rt, &client, &endpoint, &install, false);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                send(&rt, &client, &endpoint, &install, true);
                next_full = Instant::now() + SEND_EVERY;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Sends what is waiting: the vends, and with `all` the spins and errors too.
/// A failed batch goes back to the queue (except when the server refused it).
fn send(rt: &tokio::runtime::Runtime, client: &reqwest::Client, endpoint: &str, install: &str, all: bool) {
    let (worlds, spins, errors) = {
        let Ok(mut q) = QUEUE.lock() else { return };
        let worlds = std::mem::take(&mut q.worlds);
        if all {
            (worlds, std::mem::take(&mut q.spins), std::mem::take(&mut q.errors))
        } else {
            (worlds, VecDeque::new(), VecDeque::new())
        }
    };
    if worlds.is_empty() && spins.is_empty() && errors.is_empty() {
        return;
    }
    let batch = Batch {
        v: 1,
        install,
        version: env!("CARGO_PKG_VERSION"),
        vends: worlds.iter().cloned().collect(),
        spins: spins.iter().cloned().collect(),
        errors: errors.iter().cloned().collect(),
    };
    let Ok(body) = serde_json::to_vec(&batch) else { return };
    let sent = rt.block_on(async {
        let r = client.post(endpoint).header("Content-Type", "application/json").body(body).send().await?;
        Ok::<_, reqwest::Error>(r.status())
    });
    match sent {
        Ok(status) if status.is_success() => {
            tracing::debug!("report sent: {} worlds, {} spins, {} errors", worlds.len(), spins.len(), errors.len());
        }
        // Refused for good (bad data, blocked): do not retry the same batch.
        Ok(status) if status.is_client_error() && status.as_u16() != 429 => {
            tracing::debug!("report refused by the server: {status}");
        }
        other => {
            tracing::debug!("report not sent ({other:?}), will retry");
            let Ok(mut q) = QUEUE.lock() else { return };
            for w in worlds.into_iter().rev() {
                if !q.worlds.iter().any(|x| x.world == w.world) && q.worlds.len() < MAX_WORLDS {
                    q.worlds.push_front(w);
                }
            }
            for s in spins.into_iter().rev() {
                if q.spins.len() < MAX_SPINS {
                    q.spins.push_front(s);
                }
            }
            for e in errors.into_iter().rev() {
                if q.errors.len() < MAX_ERRORS {
                    q.errors.push_front(e);
                }
            }
        }
    }
}

// ── Error log lines ──────────────────────────────────────────────────────────

/// tracing layer that queues this program's warnings and errors.
pub struct ErrorLayer;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for ErrorLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        if !ERRORS_ON.load(Ordering::Relaxed) {
            return;
        }
        let meta = event.metadata();
        let level = match *meta.level() {
            tracing::Level::ERROR => "ERROR",
            tracing::Level::WARN => "WARN",
            _ => return,
        };
        // Only our own code; not the sender itself (no loops).
        if !meta.target().starts_with("gtproxy") || meta.target().starts_with("gtproxy::report") {
            return;
        }
        let mut text = String::new();
        event.record(&mut MessageVisitor(&mut text));
        error(level, scrub(&text));
    }
}

struct MessageVisitor<'a>(&'a mut String);

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0.push_str(&format!("{value:?}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_removes_login_values() {
        let s = scrub("bad login tankIDName|Serhat tankIDPass|hunter2 mac|aa:bb:cc rid|0123456789ABCDEF0123456789ABCDEF ok");
        assert!(!s.contains("Serhat") && !s.contains("hunter2") && !s.contains("aa:bb") && !s.contains("0123456789ABCDEF"), "{s}");
        assert!(s.starts_with("bad login tankIDName|***"));
        assert!(s.ends_with(" ok"));
        let long = scrub("token Zm9vYmFyYmF6cXV4cXV1eHF1dXhxdXV4 here");
        assert_eq!(long, "token *** here");
        assert_eq!(scrub("world map not readable (tile 3)"), "world map not readable (tile 3)");
    }

    #[test]
    fn off_means_nothing_is_queued() {
        vends("TEST", vec![]);
        spin("TEST", 3, '4');
        let q = QUEUE.lock().unwrap();
        assert!(q.worlds.is_empty() && q.spins.is_empty());
    }
}
