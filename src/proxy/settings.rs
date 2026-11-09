//! Persisted proxy settings (`settings.json`), `/autosave` and `/options`.
//!
//! Only choices the player made are stored; session data (world, inventory,
//! running spam/timer) starts fresh every time.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::commands::{say, warn};
use super::dialog::{DEFAULT_ICON, Dialog, DialogReturn};
use super::features::{NameList, SavedWorld, WrenchAction};
use super::interceptor::Interceptor;
use super::roulette::SpinMode;
use super::tools::show;

/// How often `/autosave` looks for changes.
const AUTOSAVE_CHECK: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub autosave: bool,
    pub fast_drop: bool,
    pub fast_trash: bool,
    pub roulette_enabled: bool,
    pub roulette_instant: bool,
    pub roulette_mode: String,
    pub save_world: Option<String>,
    pub saved_worlds: Vec<SavedWorld>,
    pub ui_color: Option<[u8; 4]>,
    pub country: Option<String>,
    pub wrench: Option<String>,
    pub auto_change: bool,
    pub spam_text1: String,
    pub spam_text2: String,
    pub spam_interval_secs: u64,
    pub spam_rainbow: bool,
    pub hide_clothes: bool,
    pub hidden_names: BTreeSet<String>,
    pub auto_pull: NameList,
    pub auto_ban: NameList,
    pub ban_words: NameList,
    pub me: bool,
    pub formalize: bool,
    pub ping_alert_ms: Option<u32>,
    pub teleport: bool,
    pub teleport_step_ms: u64,
    #[serde(rename = "unlimited_zoom")]
    pub wheel_zoom: bool,
    pub minimal: bool,
    pub hide_level: bool,
    pub auto_collect: bool,
    pub collect_range: u32,
    pub collect_only: Option<u32>,
    pub collect_skip_gems: bool,
    pub collect_interval_ms: u64,
    pub collect_message: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let ic = Interceptor::new(Default::default());
        snapshot(&ic)
    }
}

pub fn snapshot(ic: &Interceptor) -> Settings {
    let f = &ic.features;
    Settings {
        autosave: f.autosave,
        fast_drop: ic.fast_drop,
        fast_trash: ic.fast_trash,
        roulette_enabled: ic.roulette.enabled,
        roulette_instant: ic.roulette.instant,
        roulette_mode: ic.roulette.mode.name().to_string(),
        save_world: f.save_world.clone(),
        saved_worlds: f.saved_worlds.clone(),
        ui_color: f.ui_color,
        country: f.country.clone(),
        wrench: f.wrench.map(|w| w.command().to_string()),
        auto_change: f.auto_change,
        spam_text1: f.spam.text1.clone(),
        spam_text2: f.spam.text2.clone(),
        spam_interval_secs: f.spam.interval.as_secs(),
        spam_rainbow: f.spam.rainbow,
        hide_clothes: f.hide_clothes,
        hidden_names: f.hidden_names.clone(),
        auto_pull: f.auto_pull.clone(),
        auto_ban: f.auto_ban.clone(),
        ban_words: f.ban_words.clone(),
        me: f.me,
        formalize: f.formalize,
        ping_alert_ms: f.ping_alert_ms,
        teleport: ic.teleport.enabled,
        teleport_step_ms: ic.teleport.step.as_millis() as u64,
        wheel_zoom: super::zoom::enabled(),
        minimal: f.minimal,
        hide_level: f.hide_level,
        auto_collect: ic.collect.auto,
        collect_range: ic.collect.range,
        collect_only: ic.collect.only,
        collect_skip_gems: ic.collect.skip_gems,
        collect_interval_ms: ic.collect.interval.as_millis() as u64,
        collect_message: ic.collect.show_message,
    }
}

pub fn apply(ic: &mut Interceptor, s: Settings) {
    ic.fast_drop = s.fast_drop;
    ic.fast_trash = s.fast_trash;
    ic.roulette.enabled = s.roulette_enabled;
    ic.roulette.instant = s.roulette_instant;
    ic.roulette.mode = SpinMode::parse(&s.roulette_mode).unwrap_or(SpinMode::Csn);
    let f = &mut ic.features;
    f.autosave = s.autosave;
    f.save_world = s.save_world;
    f.saved_worlds = s.saved_worlds;
    f.ui_color = s.ui_color;
    f.country = s.country;
    f.wrench = s.wrench.as_deref().and_then(WrenchAction::parse);
    f.auto_change = s.auto_change;
    f.spam.text1 = s.spam_text1;
    f.spam.text2 = s.spam_text2;
    f.spam.interval = Duration::from_secs(s.spam_interval_secs).max(super::features::SPAM_MIN_INTERVAL);
    f.spam.rainbow = s.spam_rainbow;
    f.hide_clothes = s.hide_clothes;
    f.hidden_names = s.hidden_names;
    f.auto_pull = s.auto_pull;
    f.auto_ban = s.auto_ban;
    f.ban_words = s.ban_words;
    f.me = s.me;
    f.formalize = s.formalize;
    f.ping_alert_ms = s.ping_alert_ms;
    f.minimal = s.minimal;
    f.hide_level = s.hide_level;
    let c = &mut ic.collect;
    c.auto = s.auto_collect;
    c.range = s.collect_range.clamp(1, super::collect::MAX_RANGE);
    c.only = s.collect_only;
    c.skip_gems = s.collect_skip_gems;
    c.interval = Duration::from_millis(s.collect_interval_ms.max(100));
    c.show_message = s.collect_message;
    ic.teleport.enabled = s.teleport;
    ic.teleport.step = Duration::from_millis(s.teleport_step_ms.clamp(10, 500));
    if s.wheel_zoom != super::zoom::enabled() {
        if let Err(e) = super::zoom::set_enabled(s.wheel_zoom) {
            tracing::warn!("{e}");
        }
    }
}

/// Where settings live and what was last written.
#[derive(Default)]
pub struct Store {
    pub path: Option<PathBuf>,
    last_saved: Option<Settings>,
    last_check: Option<Instant>,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path: Some(path), ..Default::default() }
    }
}

/// Loads settings into `ic` at startup; a missing file just keeps the defaults.
pub fn load(ic: &mut Interceptor) {
    let Some(path) = ic.store.path.clone() else { return };
    match read(&path) {
        Ok(Some(s)) => {
            apply(ic, s);
            tracing::info!("settings loaded from {}", path.display());
        }
        Ok(None) => {}
        Err(e) => tracing::warn!("cannot read {}: {e:#}; using defaults", path.display()),
    }
    ic.store.last_saved = Some(snapshot(ic));
}

fn read(path: &Path) -> anyhow::Result<Option<Settings>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_str(&std::fs::read_to_string(path)?)?))
}

pub fn save(ic: &mut Interceptor) -> anyhow::Result<()> {
    let path = ic.store.path.clone().ok_or_else(|| anyhow::anyhow!("no settings file configured"))?;
    let s = snapshot(ic);
    std::fs::write(&path, serde_json::to_string_pretty(&s)?)?;
    tracing::info!("settings saved to {}", path.display());
    ic.store.last_saved = Some(s);
    Ok(())
}

/// Called from `Interceptor::tick`: saves when something changed and autosave is on.
pub fn autosave_tick(ic: &mut Interceptor, now: Instant) {
    if !ic.features.autosave || ic.store.path.is_none() {
        return;
    }
    if ic.store.last_check.is_some_and(|t| now < t + AUTOSAVE_CHECK) {
        return;
    }
    ic.store.last_check = Some(now);
    if ic.store.last_saved.as_ref() != Some(&snapshot(ic)) {
        if let Err(e) = save(ic) {
            tracing::error!("autosave failed: {e:#}");
        }
    }
}

// ── Commands ─────────────────────────────────────────────────────────────────

pub fn autosave(ic: &mut Interceptor, _: &[&str]) {
    ic.features.autosave = !ic.features.autosave;
    if ic.features.autosave {
        match save(ic) {
            Ok(()) => say(ic, "autosave `2on`` - settings are kept between runs"),
            Err(e) => warn(ic, format!("cannot save settings: {e}")),
        }
    } else {
        say(ic, "autosave `4off`` (use the Save button in /options to save by hand)");
    }
}

fn onoff(b: bool) -> &'static str {
    if b { "`2on``" } else { "`4off``" }
}

pub fn options(ic: &mut Interceptor, _: &[&str]) {
    let f = &ic.features;
    let ping = f.ping_alert_ms.map_or("off".to_string(), |ms| format!("above {ms}ms"));
    let d = Dialog::new("options")
        .title("Options", DEFAULT_ICON)
        .small("Every switch here is also a chat command. Settings survive restarts when autosave is on.")
        .spacer()
        .heading("General")
        .field("Autosave", onoff(f.autosave))
        .field("Ping alert", &ping)
        .button("autosave", "Toggle autosave")
        .button("save", "`2Save now``")
        .spacer()
        .heading("Items")
        .field("Fast drop", onoff(ic.fast_drop))
        .field("Fast trash", onoff(ic.fast_trash))
        .button("fast_drop", "Toggle fast drop")
        .button("fast_trash", "Toggle fast trash")
        .spacer()
        .heading("Chat")
        .field("/me mode", onoff(f.me))
        .field("Formalize", onoff(f.formalize))
        .field("Minimal messages", onoff(f.minimal))
        .field("Hide \"You need to be Level\"", onoff(f.hide_level))
        .button("me", "Toggle /me mode")
        .button("formalize", "Toggle formalize")
        .button("minimal", "Toggle minimal")
        .button("hide_level", "Toggle hide level text")
        .spacer()
        .heading("World")
        .field("Hide clothes", onoff(f.hide_clothes))
        .field("Hidden names", &f.hidden_names.len().to_string())
        .field("Auto pull", &format!("{} ({})", onoff(f.auto_pull.enabled), f.auto_pull.entries.len()))
        .field("Auto ban", &format!("{} ({})", onoff(f.auto_ban.enabled), f.auto_ban.entries.len()))
        .field("Ban words", &format!("{} ({})", onoff(f.ban_words.enabled), f.ban_words.entries.len()))
        .button("hide_clothes", "Toggle hide clothes")
        .button("open_autopull", "Auto pull list")
        .button("open_autoban", "Auto ban list")
        .button("open_banwords", "Ban words")
        .button("open_hidename", "Hide names");
    show(ic, d, "Close", "");
}

/// Buttons of the options window; `false` when it is another dialog.
pub fn on_dialog(ic: &mut Interceptor, ret: &DialogReturn) -> bool {
    if ret.name != "options" {
        return false;
    }
    match ret.button {
        Some("autosave") => {
            autosave(ic, &[]);
        }
        Some("save") => {
            match save(ic) {
                Ok(()) => say(ic, "settings saved"),
                Err(e) => warn(ic, format!("cannot save settings: {e}")),
            }
        }
        Some("fast_drop") => ic.fast_drop = !ic.fast_drop,
        Some("fast_trash") => ic.fast_trash = !ic.fast_trash,
        Some("me") => ic.features.me = !ic.features.me,
        Some("formalize") => ic.features.formalize = !ic.features.formalize,
        Some("minimal") => ic.features.minimal = !ic.features.minimal,
        Some("hide_level") => ic.features.hide_level = !ic.features.hide_level,
        Some("hide_clothes") => ic.features.hide_clothes = !ic.features.hide_clothes,
        Some("open_autopull") => {
            super::social::list_dialog(ic, super::social::ListKind::AutoPull);
            return true;
        }
        Some("open_autoban") => {
            super::social::list_dialog(ic, super::social::ListKind::AutoBan);
            return true;
        }
        Some("open_banwords") => {
            super::social::list_dialog(ic, super::social::ListKind::BanWords);
            return true;
        }
        Some("open_hidename") => {
            super::social::hidename(ic, &[]);
            return true;
        }
        _ => return true,
    }
    options(ic, &[]);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_through_json() {
        let mut ic = Interceptor::new(Default::default());
        ic.fast_drop = true;
        ic.roulette.mode = SpinMode::Reme;
        ic.features.auto_pull.entries.insert("bob".into());
        ic.features.saved_worlds.push(SavedWorld { name: "START".into(), door: Some("ABC".into()), description: "x".into() });
        let json = serde_json::to_string(&snapshot(&ic)).unwrap();

        let mut fresh = Interceptor::new(Default::default());
        apply(&mut fresh, serde_json::from_str(&json).unwrap());
        assert!(fresh.fast_drop);
        assert_eq!(fresh.roulette.mode, SpinMode::Reme);
        assert!(fresh.features.auto_pull.entries.contains("bob"));
        assert_eq!(fresh.features.saved_worlds[0].target(), "START|ABC");
    }

    #[test]
    fn old_files_with_missing_fields_still_load() {
        let s: Settings = serde_json::from_str(r#"{"fast_drop": true}"#).unwrap();
        assert!(s.fast_drop);
        assert!(s.roulette_enabled);
    }
}
