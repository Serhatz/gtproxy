//! Other players: auto pull/ban lists, ban words, hidden names, hidden
//! clothes and `/lp`.

use std::time::Duration;

use crate::protocol::{GameUpdatePacket, Message, TextPacket, Variant, VariantList};

use super::commands::{say, warn};
use super::dialog::{DEFAULT_ICON, Dialog, DialogReturn};
use super::effect::Effect;
use super::features::NameList;
use super::interceptor::{Interceptor, Verdict};
use super::tools::show;

/// Give the joining player's client a moment before pulling/banning.
const JOIN_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    AutoPull,
    AutoBan,
    BanWords,
}

impl ListKind {
    fn key(self) -> &'static str {
        match self {
            Self::AutoPull => "autopull",
            Self::AutoBan => "autoban",
            Self::BanWords => "banwords",
        }
    }

    fn from_key(k: &str) -> Option<Self> {
        [Self::AutoPull, Self::AutoBan, Self::BanWords].into_iter().find(|l| l.key() == k)
    }

    fn title(self) -> &'static str {
        match self {
            Self::AutoPull => "Auto pull",
            Self::AutoBan => "Auto ban",
            Self::BanWords => "Ban words",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::AutoPull => "People on this list are pulled as soon as they enter your world.",
            Self::AutoBan => "People on this list are banned as soon as they enter your world.",
            Self::BanWords => "Whoever says one of these words in your world is banned.",
        }
    }

    fn list(self, ic: &mut Interceptor) -> &mut NameList {
        match self {
            Self::AutoPull => &mut ic.features.auto_pull,
            Self::AutoBan => &mut ic.features.auto_ban,
            Self::BanWords => &mut ic.features.ban_words,
        }
    }
}

// ── Lists ────────────────────────────────────────────────────────────────────

pub fn autopull_page(ic: &mut Interceptor, _: &[&str]) {
    list_dialog(ic, ListKind::AutoPull);
}

pub fn autoban_page(ic: &mut Interceptor, _: &[&str]) {
    list_dialog(ic, ListKind::AutoBan);
}

pub fn banwords_page(ic: &mut Interceptor, _: &[&str]) {
    list_dialog(ic, ListKind::BanWords);
}

pub fn add_pull(ic: &mut Interceptor, args: &[&str]) {
    add_to(ic, ListKind::AutoPull, args);
}

pub fn add_ban(ic: &mut Interceptor, args: &[&str]) {
    add_to(ic, ListKind::AutoBan, args);
}

fn add_to(ic: &mut Interceptor, kind: ListKind, args: &[&str]) {
    let Some(name) = args.first() else {
        warn(ic, format!("usage: {}add{} <name>", ic.cfg.command.prefix, &kind.key()[4..]));
        return;
    };
    let list = kind.list(ic);
    list.entries.insert(name.to_lowercase());
    list.enabled = true;
    say(ic, format!("`w{name}`` added to {} (now on)", kind.title()));
}

pub fn list_dialog(ic: &mut Interceptor, kind: ListKind) {
    let list = kind.list(ic).clone();
    let mut d = Dialog::new(kind.key())
        .title(kind.title(), DEFAULT_ICON)
        .small(kind.help())
        .field("Status", if list.enabled { "`2on``" } else { "`4off``" })
        .button("toggle", if list.enabled { "Turn off" } else { "Turn on" })
        .spacer()
        .text_input("add", if kind == ListKind::BanWords { "Add word" } else { "Add name" }, "", 30)
        .button("add", "`2Add``")
        .spacer();
    if list.entries.is_empty() {
        d = d.small("The list is empty.");
    }
    for (i, entry) in list.entries.iter().enumerate() {
        d = d.button(&format!("del_{i}"), &format!("Remove `w{entry}``"));
    }
    show(ic, d, "Close", "");
}

/// Buttons of the list windows; `false` when it is another dialog.
pub fn on_dialog(ic: &mut Interceptor, ret: &DialogReturn) -> bool {
    if ret.name == "hidename" {
        if let Some(net_id) = ret.button.and_then(|b| b.strip_prefix("hide_")).and_then(|v| v.parse().ok()) {
            if let Some(name) = ic.session.players.get(&net_id).cloned() {
                toggle_hidden(ic, &name);
            }
            hidename(ic, &[]);
        }
        return true;
    }
    let Some(kind) = ListKind::from_key(ret.name) else { return false };
    match ret.button {
        Some("toggle") => {
            let list = kind.list(ic);
            list.enabled = !list.enabled;
        }
        Some("add") => {
            let entry = ret.field("add").unwrap_or_default().trim().to_lowercase();
            if !entry.is_empty() {
                kind.list(ic).entries.insert(entry);
            }
        }
        Some(b) => {
            if let Some(i) = b.strip_prefix("del_").and_then(|v| v.parse::<usize>().ok()) {
                let list = kind.list(ic);
                if let Some(entry) = list.entries.iter().nth(i).cloned() {
                    list.entries.remove(&entry);
                }
            }
        }
        None => return true,
    }
    list_dialog(ic, kind);
    true
}

/// A player appeared in the world.
pub fn on_player_joined(ic: &mut Interceptor, name: &str) {
    let action = if ic.features.auto_ban.contains(name) {
        "ban"
    } else if ic.features.auto_pull.contains(name) {
        "pull"
    } else {
        return;
    };
    tracing::info!("auto {action}: {name}");
    ic.emit(Effect::chat(&format!("/{action} {name}")).later(JOIN_DELAY));
    if action == "pull" {
        ic.features.last_pulled = Some(name.to_string());
    }
    say(ic, format!("auto {action} `w{name}``"));
}

/// Talk bubble text of someone's chat: `CP:0_PL:0_OID:_player_chat=hello`.
pub fn chat_text(bubble: &str) -> Option<&str> {
    Some(&bubble[bubble.find("player_chat=")? + "player_chat=".len()..])
}

/// `/banwords`: bans whoever says a listed word.
pub fn on_player_chat(ic: &mut Interceptor, net_id: i32, text: &str) {
    if !ic.features.ban_words.enabled || Some(net_id) == ic.session.net_id {
        return;
    }
    let lower = text.to_lowercase();
    let Some(word) = ic.features.ban_words.entries.iter().find(|w| lower.contains(w.as_str())).cloned() else {
        return;
    };
    let Some(name) = ic.session.players.get(&net_id).cloned() else { return };
    tracing::info!("ban word {word:?} from {name}");
    ic.emit(Effect::chat(&format!("/ban {name}")));
    say(ic, format!("banned `w{name}`` for saying `w{word}``"));
}

// ── /lp ──────────────────────────────────────────────────────────────────────

/// Remembers a `/pull name` you typed yourself.
pub fn note_typed_command(ic: &mut Interceptor, line: &str) {
    let mut parts = line.split_whitespace();
    if parts.next().is_some_and(|c| c.eq_ignore_ascii_case("/pull")) {
        if let Some(name) = parts.next() {
            ic.features.last_pulled = Some(name.to_string());
        }
    }
}

pub fn last_pull(ic: &mut Interceptor, _: &[&str]) {
    match ic.features.last_pulled.clone() {
        Some(name) => {
            ic.emit(Effect::chat(&format!("/pull {name}")));
            say(ic, format!("pull `w{name}``"));
        }
        None => warn(ic, "nobody pulled yet"),
    }
}

// ── /hidename ────────────────────────────────────────────────────────────────

pub fn hidename(ic: &mut Interceptor, args: &[&str]) {
    if let Some(name) = args.first() {
        toggle_hidden(ic, name);
        return;
    }
    let me = ic.session.net_id;
    let players: Vec<(i32, String)> =
        ic.session.players.iter().filter(|(id, _)| Some(**id) != me).map(|(k, v)| (*k, v.clone())).collect();
    let mut d = Dialog::new("hidename")
        .title("Hide names", DEFAULT_ICON)
        .small("Click a player to hide or show their name tag (only on your screen).")
        .spacer();
    if players.is_empty() {
        d = d.small("Nobody else is in this world.");
    }
    for (id, name) in players {
        let hidden = ic.features.hidden_names.contains(&name.to_lowercase());
        let label = if hidden { format!("`4hidden`` {name}") } else { name.clone() };
        d = d.button(&format!("hide_{id}"), &label);
    }
    show(ic, d, "Close", "");
}

fn toggle_hidden(ic: &mut Interceptor, name: &str) {
    let key = name.to_lowercase();
    let hide = !ic.features.hidden_names.remove(&key);
    if hide {
        ic.features.hidden_names.insert(key.clone());
    }
    // Apply now to whoever is in the world with that name.
    let present: Vec<i32> =
        ic.session.players.iter().filter(|(_, n)| n.to_lowercase() == key).map(|(id, _)| *id).collect();
    for id in present {
        let shown = if hide { HIDDEN_NAME.to_string() } else { ic.session.player_raw.get(&id).cloned().unwrap_or_default() };
        ic.emit(Effect::client_call(id, vec![Variant::String("OnNameChanged".into()), Variant::String(shown)]));
    }
    say(ic, format!("`w{name}``'s name {}", if hide { "hidden" } else { "shown again" }));
}

/// An empty name tag (a bare colour code renders nothing).
const HIDDEN_NAME: &str = "``";

/// Blanks the name in a hidden player's spawn packet.
pub fn hide_spawn_name(ic: &Interceptor, mut pkt: GameUpdatePacket, mut vl: VariantList, name: &str) -> Option<Verdict> {
    if !ic.features.hidden_names.contains(&name.to_lowercase()) {
        return None;
    }
    let mut spawn = TextPacket::parse(vl.get(1)?.as_str()?);
    spawn.set("name", HIDDEN_NAME);
    vl.0[1] = Variant::String(spawn.render());
    pkt.extra = vl.serialize();
    Some(Verdict::Replace(Message::Game(pkt).encode()))
}

// ── /hideclothes ─────────────────────────────────────────────────────────────

pub fn hideclothes(ic: &mut Interceptor, _: &[&str]) {
    ic.features.hide_clothes = !ic.features.hide_clothes;
    let state = if ic.features.hide_clothes { "`2on``" } else { "`4off``" };
    say(ic, format!("hide clothes {state} (applies as players enter or change clothes; re-enter the world for everyone)"));
}

/// `OnSetClothing` of another player with every item slot emptied; the skin
/// colour (`[4]`) is kept.
pub fn strip_clothing(ic: &Interceptor, mut pkt: GameUpdatePacket, mut vl: VariantList) -> Verdict {
    if !ic.features.hide_clothes || Some(pkt.net_id) == ic.session.net_id {
        return Verdict::Forward;
    }
    for i in [1, 2, 3, 5] {
        if let Some(v @ Variant::Vec3(..)) = vl.0.get_mut(i) {
            *v = Variant::Vec3(0.0, 0.0, 0.0);
        }
    }
    pkt.extra = vl.serialize();
    Verdict::Replace(Message::Game(pkt).encode())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bubble_chat_text() {
        assert_eq!(chat_text("CP:0_PL:0_OID:_player_chat=hello there"), Some("hello there"));
        assert_eq!(chat_text("`7[``spun``]"), None);
    }
}
