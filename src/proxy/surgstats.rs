//! `/surgstats`: how the surgeries went, kept in `surgery_stats.json`.
//!
//! Fed by [`super::surgery`]: every surgery window (skill fails), every tool
//! pressed, the end of a surgery (saved, or why it failed), the reward line
//! after a saved one ("You got 5 `2Hospital Window`` and a `3Caduceus``!"),
//! Malpractice and Legal Briefs.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::commands::say;
use super::dialog::{Dialog, DialogReturn};
use super::interceptor::Interceptor;
use super::session::strip_colors;
use super::tools::show;

const FILE: &str = "surgery_stats.json";
const ICON: u32 = 1296;
/// The reward line comes right after "YOU SAVED YOUR PATIENT!".
const REWARD_WINDOW: Duration = Duration::from_secs(3);

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SurgStats {
    pub saved: u32,
    pub failed: u32,
    /// Tools pressed.
    pub moves: u32,
    pub skill_fails: u32,
    pub malpractice: u32,
    pub briefs: u32,
    /// Illness -> [saved, failed].
    pub illnesses: BTreeMap<String, [u32; 2]>,
    /// Why surgeries failed.
    pub causes: BTreeMap<String, u32>,
    /// Item name -> count won.
    pub rewards: BTreeMap<String, u32>,
    /// This run only: [saved, failed].
    #[serde(skip)]
    pub session: [u32; 2],
    #[serde(skip)]
    reward_until: Option<Instant>,
    #[serde(skip)]
    malpractice_at: Option<Instant>,
}

impl SurgStats {
    fn load() -> Self {
        if cfg!(test) {
            return Self::default();
        }
        std::fs::read_to_string(FILE).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save(&self) {
        if cfg!(test) {
            return;
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(FILE, s);
        }
    }
}

/// The stats, loaded on first use.
pub fn stats(ic: &mut Interceptor) -> &mut SurgStats {
    ic.surg_stats.get_or_insert_with(SurgStats::load)
}

/// "patient has a serious head injury." -> "serious head injury".
pub fn illness_name(diagnosis: &str) -> String {
    let mut s = diagnosis.trim().trim_end_matches(['.', '!']).to_lowercase();
    for prefix in ["patient ", "is showing signs of the ", "is showing signs of ", "has a ", "has an ", "has ", "suffered ", "was "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_string();
        }
    }
    s
}

fn cause(plain: &str) -> &'static str {
    match plain {
        p if p.contains("bled out") => "bled out",
        p if p.contains("not resuscitated") => "heart not restarted",
        p if p.contains("succumbed") || p.contains("infection") => "infection / fever",
        p if p.contains("abandoned") => "abandoned",
        p if p.contains("couldn't save") => "gave up",
        p if p.contains("coma") => "coma",
        _ => "other",
    }
}

pub fn on_window(ic: &mut Interceptor, markup: &str) {
    if markup.contains("Skill Fail") {
        stats(ic).skill_fails += 1;
    }
}

pub fn on_move(ic: &mut Interceptor) {
    stats(ic).moves += 1;
}

pub fn on_saved(ic: &mut Interceptor, illness: Option<String>) {
    let s = stats(ic);
    s.saved += 1;
    s.session[0] += 1;
    if let Some(i) = illness {
        s.illnesses.entry(i).or_default()[0] += 1;
    }
    s.reward_until = Some(Instant::now() + REWARD_WINDOW);
    s.save();
}

pub fn on_failed(ic: &mut Interceptor, illness: Option<String>, text: &str) {
    let plain = strip_colors(text).to_lowercase();
    let s = stats(ic);
    s.failed += 1;
    s.session[1] += 1;
    if let Some(i) = illness {
        s.illnesses.entry(i).or_default()[1] += 1;
    }
    *s.causes.entry(cause(&plain).to_string()).or_default() += 1;
    s.save();
}

pub fn on_malpractice(ic: &mut Interceptor) {
    let s = stats(ic);
    // The penalty comes as two lines.
    if s.malpractice_at.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
        return;
    }
    s.malpractice_at = Some(Instant::now());
    s.malpractice += 1;
    s.save();
}

pub fn on_briefs(ic: &mut Interceptor) {
    let s = stats(ic);
    s.briefs += 1;
    s.save();
}

/// "You got 5 `2Hospital Window`` and a `3Caduceus``!" -> [(5, "Hospital Window"), (1, "Caduceus")].
pub fn parse_rewards(text: &str) -> Option<Vec<(u32, String)>> {
    let plain = strip_colors(text);
    let rest = plain.trim().strip_prefix("You got ")?.trim_end_matches(['!', '.']);
    let parts = rest.split(" and ").flat_map(|p| p.split(", "));
    let mut out = Vec::new();
    for part in parts {
        let part = part.trim();
        let (n, name) = part.split_once(' ')?;
        let n = match n {
            "a" | "an" => 1,
            n => n.parse().ok()?,
        };
        out.push((n, name.trim().to_string()));
    }
    (!out.is_empty()).then_some(out)
}

/// Console lines: the reward after a saved patient.
pub fn on_message(ic: &mut Interceptor, text: &str) {
    let Some(s) = ic.surg_stats.as_mut() else { return };
    if !s.reward_until.is_some_and(|t| Instant::now() < t) {
        return;
    }
    let Some(rewards) = parse_rewards(text) else { return };
    s.reward_until = None;
    for (n, name) in rewards {
        *s.rewards.entry(name).or_default() += n;
    }
    s.save();
}

fn window(ic: &mut Interceptor) {
    let s = stats(ic).clone();
    let total = s.saved + s.failed;
    let pct = |a: u32, b: u32| if b == 0 { "-".to_string() } else { format!("{}%", a * 100 / b) };
    let mut d = Dialog::new("surgstats")
        .title("Surgery Stats", ICON)
        .field("Saved", &format!("{} / {total} ({})", s.saved, pct(s.saved, total)))
        .field("This session", &format!("{} saved, {} failed", s.session[0], s.session[1]))
        .field("Skill fails", &format!("{} of {} moves ({})", s.skill_fails, s.moves, pct(s.skill_fails, s.moves)))
        .field("Moves per surgery", &if total == 0 { "-".to_string() } else { format!("{:.1}", s.moves as f32 / total as f32) })
        .field("Malpractice", &format!("{} (Legal Briefs used {})", s.malpractice, s.briefs));
    if !s.causes.is_empty() {
        d = d.spacer().text("`wFailed because``");
        let mut causes: Vec<_> = s.causes.iter().collect();
        causes.sort_by(|a, b| b.1.cmp(a.1));
        for (c, n) in causes {
            d = d.small(&format!("{c}: {n}"));
        }
    }
    if !s.illnesses.is_empty() {
        d = d.spacer().text("`wBy illness`` (saved / failed)");
        let mut ill: Vec<_> = s.illnesses.iter().collect();
        ill.sort_by(|a, b| (b.1[0] + b.1[1]).cmp(&(a.1[0] + a.1[1])));
        for (name, [ok, bad]) in ill.into_iter().take(20) {
            d = d.small(&format!("{name}: {ok} / {bad} ({})", pct(*ok, ok + bad)));
        }
    }
    if !s.rewards.is_empty() {
        d = d.spacer().text("`wRewards``");
        let mut rewards: Vec<_> = s.rewards.iter().collect();
        rewards.sort_by(|a, b| b.1.cmp(a.1));
        for (name, n) in rewards.into_iter().take(20) {
            let icon = ic.items.find(name).unwrap_or(ICON);
            d = d.item(icon, &format!("{name}: {n}"));
        }
    }
    d = d.spacer().button("reset", "`4Reset stats``");
    show(ic, d, "Close", "");
}

/// `/surgstats [reset]`.
pub fn surgstats(ic: &mut Interceptor, args: &[&str]) {
    if args.first().is_some_and(|a| a.eq_ignore_ascii_case("reset")) {
        return reset(ic);
    }
    window(ic);
}

fn reset(ic: &mut Interceptor) {
    let s = stats(ic);
    *s = SurgStats::default();
    s.save();
    say(ic, "Surgery stats reset");
}

pub fn on_dialog(ic: &mut Interceptor, ret: &DialogReturn) -> bool {
    if ret.name != "surgstats" {
        return false;
    }
    if ret.button == Some("reset") {
        reset(ic);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewards_and_names() {
        assert_eq!(
            parse_rewards("You got 5 `2Hospital Window`` and a `3Caduceus``!"),
            Some(vec![(5, "Hospital Window".into()), (1, "Caduceus".into())])
        );
        assert_eq!(parse_rewards("You got 1 `2Surgical Sponge``, 2 `2Hospital Bed`` and an `3Anatomy Chart``!").unwrap().len(), 3);
        assert_eq!(parse_rewards("Collected `w1 Dirt``"), None);
        assert_eq!(illness_name("patient has a serious head injury."), "serious head injury");
        assert_eq!(illness_name("patient is showing signs of the monkey flu."), "monkey flu");
        assert_eq!(illness_name("patient broke his leg."), "broke his leg");
        assert_eq!(cause("the patient was not resuscitated in time."), "heart not restarted");
    }
}
