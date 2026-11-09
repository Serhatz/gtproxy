//! Runtime switches and state for the optional proxy features.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrenchAction {
    Pull,
    Kick,
    Ban,
}

impl WrenchAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "pull" => Some(Self::Pull),
            "kick" => Some(Self::Kick),
            "ban" => Some(Self::Ban),
            _ => None,
        }
    }

    /// The game's own chat command.
    pub fn command(self) -> &'static str {
        match self {
            Self::Pull => "pull",
            Self::Kick => "kick",
            Self::Ban => "ban",
        }
    }
}

pub const SPAM_MIN_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct Spam {
    pub running: bool,
    pub text1: String,
    pub text2: String,
    pub interval: Duration,
    pub rainbow: bool,
    pub next_at: Instant,
    /// Which text goes next; also walks the rainbow colours.
    pub turn: usize,
}

impl Default for Spam {
    fn default() -> Self {
        Self {
            running: false,
            text1: String::new(),
            text2: String::new(),
            interval: Duration::from_secs(5),
            rainbow: false,
            next_at: Instant::now(),
            turn: 0,
        }
    }
}

const RAINBOW: &[&str] = &["`4", "`6", "`9", "`2", "`e", "`1", "`5", "`p", "`8", "`c"];

impl Spam {
    /// The next message, if one is due.
    pub fn due(&mut self, now: Instant) -> Option<String> {
        if !self.running || now < self.next_at || self.text1.is_empty() {
            return None;
        }
        self.next_at = now + self.interval;
        let text = if self.turn % 2 == 1 && !self.text2.is_empty() { &self.text2 } else { &self.text1 };
        let colour = if self.rainbow { RAINBOW[self.turn % RAINBOW.len()] } else { "" };
        self.turn += 1;
        Some(format!("{colour}{text}"))
    }
}

/// Telephone -> Sales-Man flow of `/change` (100 DL -> 1 BGL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChangeStage {
    #[default]
    Idle,
    /// Dialled the Sales-Man, waiting for his offer.
    Dialled,
}

#[derive(Debug, Default, Clone)]
pub struct Features {
    pub wrench: Option<WrenchAction>,
    pub auto_change: bool,
    pub change_stage: ChangeStage,
    /// `/country` override, two letters.
    pub country: Option<String>,
    pub save_world: Option<String>,
    /// `/ui` dialog background, r,g,b,a.
    pub ui_color: Option<[u8; 4]>,
    pub spam: Spam,
    /// Last Growtopia Gazette dialog from the server, for `/news`.
    pub news: Option<String>,
    /// Last `OnCountryState` text, so `/country` can apply at once.
    pub last_country_state: Option<String>,

    /// `/autosave`: write settings.json whenever something changes.
    pub autosave: bool,
    pub saved_worlds: Vec<SavedWorld>,
    pub hide_clothes: bool,
    /// `/hidename`: lowercase names whose name tag is blanked.
    pub hidden_names: BTreeSet<String>,
    pub auto_pull: NameList,
    pub auto_ban: NameList,
    /// `/banwords`: lowercase words; whoever says one gets banned.
    pub ban_words: NameList,
    pub last_pulled: Option<String>,
    /// `/me`: every chat line is sent as `/me <line>`.
    pub me: bool,
    pub formalize: bool,
    /// `/timer`: start time, and when the overlay was last refreshed.
    pub timer: Option<Instant>,
    pub timer_shown: Option<Instant>,
    /// Warn when proxy <-> server ping is above this.
    pub ping_alert_ms: Option<u32>,
    pub last_ping_alert: Option<Instant>,
    /// `/minimal`: proxy chat messages are not shown (warnings still are).
    pub minimal: bool,
    /// `/hidelevel`: drop the game's "You need to be Level ..." bubbles.
    pub hide_level: bool,
    /// Last proxy command line, for `/re`.
    pub last_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SavedWorld {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub door: Option<String>,
    #[serde(default)]
    pub description: String,
}

impl SavedWorld {
    /// What goes into `join_request`: `WORLD` or `WORLD|DOOR`.
    pub fn target(&self) -> String {
        match &self.door {
            Some(d) => format!("{}|{d}", self.name),
            None => self.name.clone(),
        }
    }
}

/// An on/off list of names or words (stored lowercase).
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NameList {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub entries: BTreeSet<String>,
}

impl NameList {
    pub fn contains(&self, name: &str) -> bool {
        self.enabled && self.entries.contains(&name.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spam_alternates_and_waits() {
        let mut s = Spam { running: true, text1: "a".into(), text2: "b".into(), ..Default::default() };
        let t = Instant::now();
        assert_eq!(s.due(t).as_deref(), Some("a"));
        assert_eq!(s.due(t), None);
        assert_eq!(s.due(t + s.interval).as_deref(), Some("b"));
        assert_eq!(s.due(t + s.interval * 2).as_deref(), Some("a"));
    }
}
