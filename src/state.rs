use std::net::SocketAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// State shared between the HTTPS bridge (tokio), the ENet bridge (own
/// thread) and the desktop window.
#[derive(Default)]
pub struct Shared {
    /// Real game server from the last server_data.php response; consumed when
    /// the client connects to the ENet proxy.
    server_data_upstream: Mutex<Option<SocketAddr>>,
    stop: AtomicBool,
    /// The game refused our HTTPS certificate (CA not trusted).
    tls_failed: AtomicBool,
    /// Proxy commands from the window, run by the ENet bridge like chat commands.
    commands: Mutex<Vec<String>>,
    /// What the window shows, refreshed by the ENet bridge a few times a second.
    snapshot: Mutex<Snapshot>,
}

impl Shared {
    pub fn set_server_data_upstream(&self, addr: SocketAddr) {
        *self.server_data_upstream.lock().unwrap() = Some(addr);
    }

    pub fn take_server_data_upstream(&self) -> Option<SocketAddr> {
        self.server_data_upstream.lock().unwrap().take()
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    pub fn note_tls_failure(&self) {
        self.tls_failed.store(true, Ordering::Relaxed);
    }

    pub fn tls_failed(&self) -> bool {
        self.tls_failed.load(Ordering::Relaxed)
    }

    /// Queues a proxy command line without the prefix, e.g. `"tp speed 40"`.
    pub fn command(&self, line: impl Into<String>) {
        self.commands.lock().unwrap().push(line.into());
    }

    pub fn take_commands(&self) -> Vec<String> {
        std::mem::take(&mut *self.commands.lock().unwrap())
    }

    pub fn publish(&self, s: Snapshot) {
        *self.snapshot.lock().unwrap() = s;
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().unwrap().clone()
    }
}

/// A read-only picture of the proxy for the window.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub game_connected: bool,
    pub server_connected: bool,
    pub server: Option<String>,
    pub client_ping_ms: Option<u32>,
    pub server_ping_ms: Option<u32>,

    pub name: Option<String>,
    pub world: Option<String>,
    pub level: Option<u32>,
    pub gems: Option<u64>,
    pub locks: (u32, u32, u32),
    pub inventory_items: usize,
    pub players: Vec<(i32, String)>,
    pub saved_worlds: Vec<(String, String)>,

    pub fast_drop: bool,
    pub fast_trash: bool,
    pub teleport: bool,
    pub teleport_step_ms: u64,
    pub wheel_zoom: bool,
    pub hide_clothes: bool,
    pub me: bool,
    pub formalize: bool,
    pub auto_change: bool,
    pub autosave: bool,
    pub wrench: Option<String>,
    pub ping_alert_ms: Option<u32>,
    pub spam_running: bool,
    pub auto_pull: bool,
    pub auto_ban: bool,
    pub ban_words: bool,

    pub auto_collect: bool,
    pub collect_range: u32,
    /// Item name when only one item is collected.
    pub collect_only: Option<String>,
    pub minimal: bool,
    pub hide_level: bool,
    pub track_drops: bool,
    /// Vends known from all visited worlds.
    pub vends_known: usize,
    /// Newest first: (player, number, colour code).
    pub spin_history: Vec<(String, u8, char)>,

    pub roulette_enabled: bool,
    pub roulette_instant: bool,
    pub roulette_mode: String,
    pub spins: SpinSummary,
}

#[derive(Debug, Clone, Default)]
pub struct SpinSummary {
    pub total: u32,
    pub red: u32,
    pub black: u32,
    pub green: u32,
    /// Most recent first: (number, colour code).
    pub recent: Vec<(u8, char)>,
}
