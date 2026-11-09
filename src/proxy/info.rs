//! Small information and convenience commands:
//! `/admin`, `/locked`, `/vendcopy`, `/hidelevel`, `/minimal`, `/rndm`, `/re`.
//!
//! Everything here reads what the client already received; nothing new is
//! asked from the server.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::protocol::{Variant, VariantList};

use super::commands::{say, warn};
use super::dialog::Dialog;
use super::effect::Effect;
use super::interceptor::{Interceptor, Verdict};
use super::session::strip_colors;
use super::world::{IS_OPEN_TO_PUBLIC, Special};

const LOCK_ICON: u32 = 242;
const DOOR_ICON: u32 = 12;
/// Rows shown before "... and N more".
const MAX_ROWS: usize = 100;
/// Letters used for `/rndm` world names.
const RANDOM_LEN: std::ops::RangeInclusive<usize> = 4..=7;

fn onoff(b: bool) -> &'static str {
    if b { "`2on``" } else { "`4off``" }
}

// ── /admin ───────────────────────────────────────────────────────────────────

/// A userID as a name when we have seen that player, else the number.
fn who(ic: &Interceptor, user_id: u32) -> String {
    match ic.session.user_name(user_id) {
        Some(name) => format!("`w{name}`` `s(#{user_id})``"),
        None => format!("`s#{user_id}``"),
    }
}

/// `/admin`: lock owners and everyone with access.
pub fn admin(ic: &mut Interceptor, _: &[&str]) {
    let Some(map) = ic.world.as_ref() else {
        return warn(ic, "Enter a world first");
    };
    let name = map.name.clone();
    let locks: Vec<(i32, i32, u16, u32, Vec<u32>, bool)> = map
        .locks()
        .into_iter()
        .map(|(x, y, fg, a)| {
            let public = map.tile(x, y).is_some_and(|t| t.flags & IS_OPEN_TO_PUBLIC != 0);
            (x, y, fg, a.owner, a.users.clone(), public)
        })
        .collect();
    let mut dialog = Dialog::new("admin").title(&format!("Admins of {name}"), LOCK_ICON);
    if let Some(owner) = ic.session.world_owner.clone() {
        dialog = dialog.text(&format!("World owner: `w{owner}``"));
    }
    if locks.is_empty() {
        dialog = dialog.text("This world has no locks.");
    }
    dialog = dialog.small("Names show for players seen since the proxy started, others as their user ID.");
    for (x, y, fg, owner, users, public) in locks.iter().take(MAX_ROWS) {
        let public = if *public { "  `2public``" } else { "" };
        dialog = dialog.spacer().item(*fg as u32, &format!("`w{}`` `s({x}, {y})``{public}", ic.items.name(*fg as u32)));
        dialog = dialog.text(&format!("Owner: {}", who(ic, *owner)));
        if users.is_empty() {
            dialog = dialog.small("No one else has access.");
        } else {
            let list: Vec<String> = users.iter().map(|u| who(ic, *u)).collect();
            dialog = dialog.text(&format!("Access ({}): {}", users.len(), list.join(", ")));
        }
    }
    if locks.len() > MAX_ROWS {
        dialog = dialog.small(&format!("... and {} more locks", locks.len() - MAX_ROWS));
    }
    super::tools::show(ic, dialog, "Close", "");
}

/// `OnConsoleMessage` on world entry: "`[WORLD World Locked by NAME (...)]`".
pub fn note_world_owner(ic: &mut Interceptor, text: &str) {
    let plain = strip_colors(text);
    let Some((_, rest)) = plain.split_once("World Locked by ") else { return };
    let name = rest.split([' ', ']', '(']).next().unwrap_or_default().trim();
    if !name.is_empty() {
        ic.session.world_owner = Some(name.to_string());
    }
}

// ── /locked ──────────────────────────────────────────────────────────────────

/// `/locked`: every door with its label; private (not public) doors first.
pub fn locked(ic: &mut Interceptor, _: &[&str]) {
    let Some(map) = ic.world.as_ref() else {
        return warn(ic, "Enter a world first");
    };
    let user = ic.session.user_id.unwrap_or(0) as u32;
    let mut doors: Vec<(bool, i32, i32, u16, String)> = map
        .tiles
        .iter()
        .enumerate()
        .filter_map(|(i, t)| match &t.special {
            Special::Door { label } => {
                let (x, y) = map.xy(i);
                let open = map.has_access(x, y, user);
                Some((open, x, y, t.fg, label.clone()))
            }
            _ => None,
        })
        .collect();
    doors.sort_by_key(|d| (d.0, d.2, d.1));
    let closed = doors.iter().filter(|d| !d.0).count();
    let mut dialog = Dialog::new("locked")
        .title("Doors", DOOR_ICON)
        .small(&format!("{} doors, `4{closed}`` you can't open. The door ID itself is not sent to players.", doors.len()));
    if doors.is_empty() {
        dialog = dialog.text("No doors in this world.");
    }
    for (open, x, y, fg, label) in doors.iter().take(MAX_ROWS) {
        let state = if *open { "`2open``" } else { "`4locked``" };
        let label = if label.is_empty() { "`s(no label)``".to_string() } else { format!("`w{label}``") };
        dialog = dialog.item(*fg as u32, &format!("{label}  {state}  `s({x}, {y})``"));
    }
    if doors.len() > MAX_ROWS {
        dialog = dialog.small(&format!("... and {} more", doors.len() - MAX_ROWS));
    }
    super::tools::show(ic, dialog, "Close", "");
}

// ── /vendcopy ────────────────────────────────────────────────────────────────

/// The vend list of this world as plain text, one machine per line.
pub fn vend_text(ic: &Interceptor) -> Option<String> {
    let map = ic.world.as_ref()?;
    let mut lines = Vec::new();
    for (i, t) in map.tiles.iter().enumerate() {
        if let Special::Vend { item, price } = t.special {
            if item == 0 || price == 0 {
                continue;
            }
            let (x, y) = map.xy(i);
            let price = if price > 0 { format!("{price} WL each") } else { format!("{} per WL", -price) };
            lines.push(format!("{} - {price} ({x}, {y})", ic.items.name(item)));
        }
    }
    (!lines.is_empty()).then(|| format!("{} vends:\n{}", map.name, lines.join("\n")))
}

/// `/vendcopy`: puts this world's vend prices on the clipboard.
pub fn vendcopy(ic: &mut Interceptor, _: &[&str]) {
    if ic.world.is_none() {
        return warn(ic, "Enter a world first");
    }
    let Some(text) = vend_text(ic) else {
        return warn(ic, "No vending machines with items here");
    };
    let n = text.lines().count() - 1;
    copy_to_clipboard(text);
    say(ic, format!("Copied `w{n}`` vend prices to the clipboard"));
}

/// Windows `clip.exe`; runs in the background so the bridge never waits.
fn copy_to_clipboard(text: String) {
    if cfg!(test) {
        return;
    }
    std::thread::spawn(move || {
        use std::io::Write;
        use std::process::{Command, Stdio};
        // clip reads UTF-16 when the text starts with a BOM, so names keep their letters.
        let mut bytes = vec![0xFF, 0xFE];
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let child = Command::new("clip").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        match child {
            Ok(mut c) => {
                if let Some(mut stdin) = c.stdin.take() {
                    let _ = stdin.write_all(&bytes);
                }
                let _ = c.wait();
            }
            Err(e) => tracing::warn!("clipboard: {e}"),
        }
    });
}

// ── /hidelevel, /minimal ─────────────────────────────────────────────────────

pub fn hidelevel(ic: &mut Interceptor, _: &[&str]) {
    ic.features.hide_level = !ic.features.hide_level;
    say(ic, format!("Hide \"You need to be Level\" text {}", onoff(ic.features.hide_level)));
}

/// `/minimal`: the switch message itself is always shown.
pub fn minimal(ic: &mut Interceptor, _: &[&str]) {
    ic.features.minimal = !ic.features.minimal;
    let text = if ic.features.minimal {
        "Minimal mode `2on``: proxy messages are hidden, warnings still show"
    } else {
        "Minimal mode `4off``"
    };
    ic.emit(Effect::console(format!("`9[proxy]`` {text}")));
}

/// Talk bubbles and console lines like "You need to be Level 5 to break that".
pub fn hide_level_text(ic: &Interceptor, vl: &VariantList, text_index: usize) -> Option<Verdict> {
    if !ic.features.hide_level {
        return None;
    }
    let text = vl.get(text_index).and_then(Variant::as_str)?;
    strip_colors(text).contains("You need to be Level ").then_some(Verdict::Drop)
}

// ── /terminal ────────────────────────────────────────────────────────────────

/// `/terminal [on|off]`: whether the next start opens the terminal or the
/// window (`system.terminal` in config.toml). The running proxy stays as it is.
pub fn terminal(ic: &mut Interceptor, args: &[&str]) {
    let mut cfg = match crate::config::Config::load_or_create() {
        Ok(c) => c,
        Err(e) => return warn(ic, format!("Cannot read config.toml: {e}")),
    };
    let on = match args.first().map(|a| a.to_ascii_lowercase()).as_deref() {
        None => !cfg.system.terminal,
        Some("on") => true,
        Some("off") => false,
        Some(_) => return warn(ic, format!("Usage: {0}terminal, {0}terminal on, {0}terminal off", ic.cfg.command.prefix)),
    };
    cfg.system.terminal = on;
    if !cfg!(test) {
        if let Err(e) = cfg.save() {
            return warn(ic, format!("Cannot save config.toml: {e}"));
        }
    }
    let text = if on {
        "Terminal mode `2on``: the next start opens the terminal instead of the window"
    } else {
        "Terminal mode `4off``: the next start opens the window"
    };
    ic.emit(Effect::console(format!("`9[proxy]`` {text}")));
}

// ── /rndm, /re ───────────────────────────────────────────────────────────────

/// A random world name of 4-7 letters.
pub fn random_world(seed: u64) -> String {
    let mut x = seed | 1;
    let mut next = || {
        // xorshift64
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let span = (RANDOM_LEN.end() - RANDOM_LEN.start() + 1) as u64;
    let len = RANDOM_LEN.start() + (next() % span) as usize;
    (0..len).map(|_| (b'A' + (next() % 26) as u8) as char).collect()
}

/// `/rndm`: warp to a random world.
pub fn rndm(ic: &mut Interceptor, _: &[&str]) {
    let seed = SystemTime::now().duration_since(UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64);
    let world = random_world(seed);
    say(ic, format!("Warping to a random world: `w{world}``"));
    super::warp::go_to(ic, &world);
}

/// `/re`: runs the last proxy command again.
pub fn re(ic: &mut Interceptor, _: &[&str]) {
    match ic.features.last_command.clone() {
        Some(line) => {
            say(ic, format!("Running `w{}{line}`` again", ic.cfg.command.prefix));
            super::commands::run(ic, &line);
        }
        None => warn(ic, "No command to repeat yet"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::sync::Arc;

    #[test]
    fn random_names_are_letters() {
        for seed in 0..50 {
            let w = random_world(seed * 7919);
            assert!(RANDOM_LEN.contains(&w.len()), "{w}");
            assert!(w.chars().all(|c| c.is_ascii_uppercase()));
        }
        assert_ne!(random_world(1), random_world(2));
    }

    #[test]
    fn owner_from_entry_message() {
        let mut ic = Interceptor::new(Arc::new(Config::default()));
        note_world_owner(&mut ic, "`5[```wSERHATMAIN`` `$World Locked`` by Serhatzzz (`2ACCESS GRANTED``)`5]``");
        assert_eq!(ic.session.world_owner.as_deref(), Some("Serhatzzz"));
        note_world_owner(&mut ic, "`5[```wBUY`` `$World Locked`` by Bob`5]``");
        assert_eq!(ic.session.world_owner.as_deref(), Some("Bob"));
    }

    #[test]
    fn level_text_is_hidden_only_when_on() {
        let mut ic = Interceptor::new(Arc::new(Config::default()));
        let vl = VariantList::new(vec![
            Variant::String("OnTalkBubble".into()),
            Variant::Unsigned(2),
            Variant::String("`wYou need to be Level 7 to break that.``".into()),
        ]);
        assert!(hide_level_text(&ic, &vl, 2).is_none());
        ic.features.hide_level = true;
        assert_eq!(hide_level_text(&ic, &vl, 2), Some(Verdict::Drop));
    }
}
