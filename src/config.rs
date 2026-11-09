use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const CONFIG_PATH: &str = "config.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub proxy: ProxyConfig,
    pub upstream: UpstreamConfig,
    pub system: SystemConfig,
    pub log: LogConfig,
    pub command: CommandConfig,
    pub ui: UiConfig,
    pub report: ReportConfig,
}

/// Sharing with the web panel (`report.rs`). On by default; Settings turns it off.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReportConfig {
    pub enabled: bool,
    /// Panel address, e.g. `https://example.com/gtproxy`; empty = the project's panel.
    pub url: String,
    pub vends: bool,
    pub spins: bool,
    pub errors: bool,
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self { enabled: true, url: String::new(), vends: true, spins: true, errors: true }
    }
}

/// The project's panel; empty until the domain is set.
pub const DEFAULT_REPORT_URL: &str = "https://gtproxy.xyz";

/// Look of the desktop window; edited live in its Settings page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// "title" (game title screen), "connecting" (login screen) or "plain".
    pub backdrop: String,
    pub background: [u8; 3],
    pub sidebar: [u8; 3],
    pub panel: [u8; 3],
    pub rim: [u8; 3],
    pub inset: [u8; 3],
    pub button: [u8; 3],
    pub accent: [u8; 3],
    pub text: [u8; 3],
    pub muted: [u8; 3],
    /// Panel opacity, 0.3 - 1.0.
    pub opacity: f32,
    /// Multiplies every font size, 0.8 - 1.4.
    pub font_scale: f32,
    pub corner_radius: u8,
    /// Embedded mode: the game window lives inside the proxy window.
    pub embed_game: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        // Measured from the game's own screens.
        Self {
            backdrop: "title".into(),
            background: [16, 52, 68],
            sidebar: [11, 41, 54],
            panel: [51, 121, 154],
            rim: [133, 182, 202],
            inset: [22, 87, 108],
            button: [140, 196, 216],
            accent: [58, 170, 55],
            text: [255, 255, 255],
            muted: [252, 230, 186],
            opacity: 0.84,
            font_scale: 1.0,
            corner_radius: 8,
            embed_game: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxyConfig {
    /// Address the game client is pointed at (written into server_data / OnSendToServer).
    pub local_ip: String,
    /// HTTPS port for server_data.php. The game always uses 443.
    pub http_port: u16,
    /// UDP port of the ENet proxy server.
    pub enet_port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpstreamConfig {
    /// Hosts tried in order for server_data.php.
    pub server_data_hosts: Vec<String>,
    /// DNS server used to resolve the real Growtopia IPs (bypasses the hosts file).
    pub dns_server: String,
    /// Verify the real server's TLS certificate.
    pub verify_tls: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemConfig {
    /// Add our entries to the Windows hosts file on start, remove them on exit.
    pub manage_hosts: bool,
    /// Directory holding the generated CA and server certificate.
    pub cert_dir: String,
    /// In-game settings saved by /autosave and /options.
    pub settings_path: String,
    /// Growtopia.exe; empty = %LOCALAPPDATA%\Growtopia\Growtopia.exe.
    pub game_path: String,
    /// Start the game when the window's Start button is pressed.
    pub launch_game: bool,
    /// Open the terminal instead of the window on a plain start.
    pub terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    /// tracing filter, e.g. "info" or "gtproxy=debug".
    pub level: String,
    pub text: bool,
    pub game_packet: bool,
    pub variant: bool,
    /// Skip high-frequency noise (movement states, pings).
    pub skip_spam: bool,
    /// Mask tokens and device identifiers in logged login packets.
    pub redact_login: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CommandConfig {
    pub prefix: char,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            proxy: ProxyConfig::default(),
            upstream: UpstreamConfig::default(),
            system: SystemConfig::default(),
            log: LogConfig::default(),
            command: CommandConfig::default(),
            ui: UiConfig::default(),
            report: ReportConfig::default(),
        }
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            local_ip: "127.0.0.1".into(),
            http_port: 443,
            enet_port: 17091,
        }
    }
}

impl Default for UpstreamConfig {
    fn default() -> Self {
        Self {
            server_data_hosts: vec!["www.growtopia1.com".into(), "www.growtopia2.com".into()],
            dns_server: "1.1.1.1:53".into(),
            verify_tls: true,
        }
    }
}

impl Default for SystemConfig {
    fn default() -> Self {
        Self {
            manage_hosts: true,
            cert_dir: "certs".into(),
            settings_path: "settings.json".into(),
            game_path: String::new(),
            launch_game: true,
            terminal: false,
        }
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            text: true,
            game_packet: false,
            variant: true,
            skip_spam: true,
            redact_login: true,
        }
    }
}

impl Default for CommandConfig {
    fn default() -> Self {
        Self { prefix: '/' }
    }
}

impl Config {
    pub fn save(&self) -> Result<()> {
        std::fs::write(CONFIG_PATH, toml::to_string_pretty(self)?).with_context(|| format!("failed to write {CONFIG_PATH}"))
    }

    /// Loads `config.toml`, writing the defaults first if it does not exist.
    pub fn load_or_create() -> Result<Self> {
        let path = Path::new(CONFIG_PATH);
        if !path.exists() {
            let cfg = Config::default();
            std::fs::write(path, toml::to_string_pretty(&cfg)?)
                .with_context(|| format!("failed to write {CONFIG_PATH}"))?;
            return Ok(cfg);
        }
        let raw = std::fs::read_to_string(path).with_context(|| format!("failed to read {CONFIG_PATH}"))?;
        toml::from_str(&raw).with_context(|| format!("invalid {CONFIG_PATH}"))
    }
}
