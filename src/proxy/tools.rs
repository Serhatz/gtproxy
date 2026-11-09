//! Account, inventory, world-owner and convenience commands, plus the packet
//! hooks they need (`/wrench`, `/country`, `/change`).

use std::time::{Duration, Instant};

use crate::protocol::{GameUpdatePacket, Message, TextPacket, Variant, VariantList};

use super::commands::{go_to, plan_drop, say, warn};
use super::dialog::{DEFAULT_ICON, Dialog, DialogReturn};
use super::effect::Effect;
use super::features::{ChangeStage, SPAM_MIN_INTERVAL, WrenchAction};
use super::interceptor::{Interceptor, Verdict};
use super::locks::{self, BGL, DL, Step, WL};

/// Gap between successive automated packets (conversions, pulls, drops).
const STEP_SPACING: Duration = Duration::from_millis(350);
const TELEPHONE: u32 = 3898;
const SALES_MAN_NUMBER: &str = "53785";

/// Opens one of our dialogs with the `/ui` colours applied.
pub fn show(ic: &mut Interceptor, dialog: Dialog, cancel: &str, ok: &str) {
    let markup = ic.style_dialog(dialog.build(cancel, ok));
    ic.emit(Effect::dialog(markup));
}

fn onoff(b: bool) -> &'static str {
    if b { "`2on``" } else { "`4off``" }
}

// ── Information ──────────────────────────────────────────────────────────────

pub fn balance(ic: &mut Interceptor, _: &[&str]) {
    let (wl, dl, bgl) = (ic.inventory.count(WL), ic.inventory.count(DL), ic.inventory.count(BGL));
    if wl + dl + bgl == 0 {
        warn(ic, "you don't have any locks");
        return;
    }
    say(
        ic,
        format!(
            "`w{bgl}`` Blue Gem Lock, `w{dl}`` Diamond Lock, `w{wl}`` World Lock = `2{} WL``",
            locks::total_wl(wl, dl, bgl)
        ),
    );
}

pub fn inventory(ic: &mut Interceptor, _: &[&str]) {
    let mut d = Dialog::new("inventory")
        .title("Inventory", DEFAULT_ICON)
        .small(&format!("{} items, {} slots", ic.inventory.len(), ic.inventory.slots))
        .spacer();
    if ic.inventory.len() == 0 {
        d = d.text("Nothing yet. The list fills in when you log in through the proxy.");
    }
    let rows: Vec<(u32, u32)> = ic.inventory.iter().collect();
    for (id, count) in rows {
        d = d.item(id, &format!("`w{}`` x{count}", ic.items.name(id)));
    }
    show(ic, d, "Close", "");
}

pub fn account(ic: &mut Interceptor, _: &[&str]) {
    let s = &ic.session;
    let or_dash = |v: Option<String>| v.unwrap_or_else(|| "-".to_string());
    let d = Dialog::new("account")
        .title("Account", DEFAULT_ICON)
        .spacer()
        .field("Name", &or_dash(s.name.clone()))
        .field("userID", &or_dash(s.user_id.map(|v| v.to_string())))
        .field("netID", &or_dash(s.net_id.map(|v| v.to_string())))
        .field("Level", &or_dash(s.level.map(|v| v.to_string())))
        .field("Gems", &or_dash(s.gems.map(|v| v.to_string())))
        .field("Country", &or_dash(s.country.clone()))
        .field("World", s.world.as_deref().unwrap_or("EXIT"))
        .field(
            "Locks",
            &format!(
                "{} WL worth",
                locks::total_wl(ic.inventory.count(WL), ic.inventory.count(DL), ic.inventory.count(BGL))
            ),
        );
    show(ic, d, "Close", "");
}

pub fn news(ic: &mut Interceptor, _: &[&str]) {
    match ic.features.news.clone() {
        Some(markup) => {
            let markup = ic.style_dialog(markup);
            ic.emit(Effect::dialog(markup));
        }
        None => warn(ic, "no news yet (the server sends it when you log in)"),
    }
}

pub fn debug(ic: &mut Interceptor, _: &[&str]) {
    let mut d = Dialog::new("debug")
        .title("Debug", DEFAULT_ICON)
        .spacer()
        .heading("Session")
        .field("World", ic.session.world.as_deref().unwrap_or("EXIT"))
        .field("Server", &ic.session.server.map_or("-".into(), |a| a.to_string()))
        .field("Redirect pending", &ic.redirect.map_or("-".into(), |a| a.to_string()))
        .field("Inventory", &format!("{} items / {} slots", ic.inventory.len(), ic.inventory.slots))
        .field("Planned drops", &ic.planned_drops.len().to_string())
        .field("Item names", if ic.items.name(242) == "World Lock" { "loaded" } else { "missing" })
        .spacer()
        .heading(&format!("Players here ({})", ic.session.players.len()));
    let players: Vec<(i32, String)> = ic.session.players.iter().map(|(k, v)| (*k, v.clone())).collect();
    for (id, name) in players {
        d = d.text(&format!("`w{name}`` `s(netID {id})``"));
    }
    show(ic, d, "Close", "");
}

// ── Locks ────────────────────────────────────────────────────────────────────

/// `/cd 250`: drop that many World Locks' worth, breaking bigger locks if needed.
pub fn custom_drop(ic: &mut Interceptor, args: &[&str]) {
    let Some(amount) = args.first().and_then(|a| a.parse::<u32>().ok()) else {
        say(ic, format!("`4usage:`` {}cd <WL amount>  (e.g. 250 = 2 DL + 50 WL)", ic.cfg.command.prefix));
        return;
    };
    let (wl, dl, bgl) = (ic.inventory.count(WL), ic.inventory.count(DL), ic.inventory.count(BGL));
    let steps = match locks::plan_custom_drop(amount, wl, dl, bgl) {
        Ok(steps) => steps,
        Err(e) => {
            warn(ic, e);
            return;
        }
    };
    tracing::info!("/cd {amount}: {steps:?}");
    run_steps(ic, steps);
    say(ic, format!("dropping `w{amount} WL`` worth..."));
}

/// `/dd <n>`: Diamond Locks, breaking or using Blue Gem Locks when the DL run short.
pub fn drop_dl(ic: &mut Interceptor, args: &[&str]) {
    let Some(n) = args.first().and_then(|a| a.parse::<u32>().ok()) else {
        warn(ic, format!("Usage: {}dd <DL amount>  (100 = 1 BGL)", ic.cfg.command.prefix));
        return;
    };
    let (dl, bgl) = (ic.inventory.count(DL), ic.inventory.count(BGL));
    if ic.inventory.slots == 0 {
        // Inventory not known yet: just ask, the server's dialog checks the amount.
        if n <= 200 {
            plan_drop(ic, DL, "Diamond Lock", Some(n), Duration::ZERO);
        } else {
            warn(ic, "Inventory not known yet, open it once");
        }
        return;
    }
    if dl == 0 && bgl == 0 {
        return warn(ic, "You have no Diamond Lock");
    }
    let steps = match locks::plan_dl_drop(n, dl, bgl) {
        Ok(steps) => steps,
        Err(e) => return warn(ic, e),
    };
    tracing::info!("/dd {n}: {steps:?}");
    if steps.iter().any(|s| matches!(s, Step::Convert(_))) {
        say(ic, "Not enough Diamond Locks, breaking a Blue Gem Lock");
    }
    run_steps(ic, steps);
}

/// Converts and drops one after another, spaced out.
fn run_steps(ic: &mut Interceptor, steps: Vec<Step>) {
    let mut delay = Duration::ZERO;
    for step in steps {
        match step {
            Step::Convert(id) => {
                ic.emit(Effect::activate_item(id).later(delay));
            }
            Step::Drop { item_id, amount } => {
                let name = lock_name(item_id);
                plan_drop(ic, item_id, name, Some(amount), delay);
            }
        }
        delay += STEP_SPACING;
    }
}

fn lock_name(id: u32) -> &'static str {
    match id {
        WL => "World Lock",
        DL => "Diamond Lock",
        _ => "Blue Gem Lock",
    }
}

pub fn change(ic: &mut Interceptor, _: &[&str]) {
    ic.features.auto_change = !ic.features.auto_change;
    ic.features.change_stage = ChangeStage::Idle;
    let state = onoff(ic.features.auto_change);
    say(ic, format!("auto change {state} (wrench a Telephone: 100 Diamond Lock -> 1 Blue Gem Lock)"));
}

/// Value after `key|` anywhere in dialog markup (`embed_data|tilex|12`).
fn dialog_value<'a>(markup: &'a str, key: &str) -> Option<&'a str> {
    markup.lines().find_map(|line| {
        let parts: Vec<&str> = line.split('|').collect();
        parts.windows(2).find(|w| w[0] == key).map(|w| w[1])
    })
}

/// `/change`: answers the Telephone and Sales-Man dialogs to buy a Blue Gem
/// Lock for 100 Diamond Locks. `None` = not one of those dialogs.
pub fn auto_change(ic: &mut Interceptor, markup: &str) -> Option<Verdict> {
    if !ic.features.auto_change {
        return None;
    }
    let tile = || Some((dialog_value(markup, "tilex")?.to_string(), dialog_value(markup, "tiley")?.to_string()));
    let affordable = ic.inventory.count(DL) >= 100 && ic.inventory.count(BGL) < 200;

    if markup.contains(&format!("`wTelephone``|left|{TELEPHONE}|")) {
        let (x, y) = tile()?;
        if !affordable {
            warn(ic, "auto change: need 100 Diamond Locks and room for a Blue Gem Lock");
            return None;
        }
        ic.features.change_stage = ChangeStage::Dialled;
        ic.emit(Effect::text(format!(
            "action|dialog_return\ndialog_name|phonecall\ntilex|{x}|\ntiley|{y}|\nnum|-2|\ndial|{SALES_MAN_NUMBER}\n"
        )));
        return Some(Verdict::Drop);
    }
    if ic.features.change_stage != ChangeStage::Dialled {
        return None;
    }
    if markup.contains("`wSales-Man``|left|4358|") {
        let (x, y) = tile()?;
        ic.emit(Effect::text(format!(
            "action|dialog_return\ndialog_name|phonecall\ntilex|{x}|\ntiley|{y}|\nnum|{SALES_MAN_NUMBER}|\nbuttonClicked|chc5\n"
        )));
        return Some(Verdict::Drop);
    }
    if markup.contains("`wBlue Gem Lock``|left|7188|") {
        if markup.contains("fast delivery") {
            ic.features.change_stage = ChangeStage::Idle;
            say(ic, "`2changed 100 Diamond Lock -> 1 Blue Gem Lock``");
            return Some(Verdict::Drop);
        }
        let (x, y) = tile()?;
        ic.emit(Effect::text(format!(
            "action|dialog_return\ndialog_name|phonecall\ntilex|{x}|\ntiley|{y}|\nnum|-34|\nbuttonClicked|chc0\n"
        )));
        return Some(Verdict::Drop);
    }
    None
}

// ── Appearance (only on your own screen) ─────────────────────────────────────

pub fn country(ic: &mut Interceptor, args: &[&str]) {
    let code = args.first().map(|c| c.to_ascii_lowercase());
    match code {
        Some(c) if c.len() == 2 && c.chars().all(|ch| ch.is_ascii_lowercase()) => {
            ic.features.country = Some(c.clone());
            // Apply at once when we know the current state string.
            if let Some(state) = ic.features.last_country_state.clone() {
                let net_id = ic.session.net_id.unwrap_or(-1);
                ic.emit(Effect::client_call(
                    net_id,
                    vec![Variant::String("OnCountryState".into()), Variant::String(with_country(&state, &c))],
                ));
            }
            say(ic, format!("flag set to `w{c}`` (only you see it; re-enter the world if it does not update)"));
        }
        Some(c) if c == "off" => {
            ic.features.country = None;
            say(ic, "flag override off");
        }
        _ => say(ic, format!("`4usage:`` {}country <2-letter code|off>  e.g. tr, us", ic.cfg.command.prefix)),
    }
}

/// "us|showGuild|doctor" -> "tr|showGuild|doctor"
fn with_country(state: &str, code: &str) -> String {
    match state.split_once('|') {
        Some((_, rest)) => format!("{code}|{rest}"),
        None => code.to_string(),
    }
}

pub fn rewrite_country_state(ic: &mut Interceptor, mut pkt: GameUpdatePacket, mut vl: VariantList) -> Verdict {
    let Some(state) = vl.get(1).and_then(Variant::as_str).map(String::from) else { return Verdict::Forward };
    ic.features.last_country_state = Some(state.clone());
    let Some(code) = ic.features.country.clone() else { return Verdict::Forward };
    vl.0[1] = Variant::String(with_country(&state, &code));
    pkt.extra = vl.serialize();
    Verdict::Replace(Message::Game(pkt).encode())
}

/// Applies `/country` and unlimited zoom to our own spawn packet.
pub fn rewrite_local_spawn(ic: &mut Interceptor, mut pkt: GameUpdatePacket, mut vl: VariantList) -> Verdict {
    let Some(text) = vl.get(1).and_then(Variant::as_str) else { return Verdict::Forward };
    let mut spawn = TextPacket::parse(text);
    let mut changed = super::zoom::rewrite_spawn(&mut spawn);
    if let (Some(code), Some(current)) = (ic.features.country.clone(), spawn.get("country").map(String::from)) {
        spawn.set("country", with_country(&current, &code));
        changed = true;
    }
    if !changed {
        return Verdict::Forward;
    }
    vl.0[1] = Variant::String(spawn.render());
    pkt.extra = vl.serialize();
    Verdict::Replace(Message::Game(pkt).encode())
}

/// `/nick <name>`; plain `/nick` puts your real name back.
pub fn nick(ic: &mut Interceptor, args: &[&str]) {
    let Some(net_id) = ic.session.net_id else {
        warn(ic, "enter a world first");
        return;
    };
    let (name, message) = if args.is_empty() {
        let Some(original) = ic.session.raw_name.clone() else {
            warn(ic, "original name not known yet, re-enter the world");
            return;
        };
        (original, "name restored".to_string())
    } else {
        let name = args.join(" ");
        let message = format!("name shown as `w{name}`` (only on your screen, {}nick to undo)", ic.cfg.command.prefix);
        (name, message)
    };
    ic.emit(Effect::client_call(net_id, vec![Variant::String("OnNameChanged".into()), Variant::String(name)]));
    say(ic, message);
}

/// `/skin <code>`; plain `/skin` puts your real skin colour back.
pub fn skin(ic: &mut Interceptor, args: &[&str]) {
    if args.is_empty() {
        let (Some(net_id), Some(original)) = (ic.session.net_id, ic.session.skin) else {
            warn(ic, "original skin not known yet, re-enter the world");
            return;
        };
        ic.emit(Effect::client_call(net_id, vec![Variant::String("OnChangeSkin".into()), Variant::Unsigned(original)]));
        say(ic, "skin colour restored");
        return;
    }
    let code = match args {
        ["hex", h, ..] => u32::from_str_radix(h.trim_start_matches("0x"), 16).ok(),
        [v, ..] if v.starts_with("0x") => u32::from_str_radix(&v[2..], 16).ok(),
        [v, ..] => v.parse().ok(),
        [] => unreachable!(),
    };
    let Some(code) = code else {
        say(ic, format!("`4usage:`` {}skin <code> or {}skin hex <code>", ic.cfg.command.prefix, ic.cfg.command.prefix));
        return;
    };
    let Some(net_id) = ic.session.net_id else {
        warn(ic, "enter a world first");
        return;
    };
    ic.emit(Effect::client_call(net_id, vec![Variant::String("OnChangeSkin".into()), Variant::Unsigned(code)]));
    say(ic, format!("skin colour set to `w{code}`` (only on your screen, {}skin to undo)", ic.cfg.command.prefix));
}

pub fn ui(ic: &mut Interceptor, args: &[&str]) {
    if args.first() == Some(&"off") {
        ic.features.ui_color = None;
        say(ic, "dialog colours back to normal");
        return;
    }
    ui_dialog(ic);
}

fn ui_dialog(ic: &mut Interceptor) {
    let [r, g, b, a] = ic.features.ui_color.unwrap_or([0, 0, 0, 160]);
    let d = Dialog::new("ui")
        .title("Dialog colours", DEFAULT_ICON)
        .small("Background colour for every dialog the game opens. Values 0-255.")
        .spacer()
        .text_input("red", "`4Red``", &r.to_string(), 3)
        .text_input("green", "`2Green``", &g.to_string(), 3)
        .text_input("blue", "`1Blue``", &b.to_string(), 3)
        .text_input("alpha", "`wAlpha`` (opacity)", &a.to_string(), 3)
        .spacer()
        .button("reset", "Reset to default");
    show(ic, d, "Cancel", "Apply");
}

// ── Worlds ───────────────────────────────────────────────────────────────────

pub fn set_save(ic: &mut Interceptor, args: &[&str]) {
    let world = args.first().map(|w| w.to_ascii_uppercase());
    match world {
        Some(w) if w.len() <= 23 && w.chars().all(|c| c.is_ascii_alphanumeric()) => {
            ic.features.save_world = Some(w.clone());
            say(ic, format!("save world set to `w{w}``"));
        }
        _ => say(ic, format!("`4usage:`` {}setsave <world>", ic.cfg.command.prefix)),
    }
}

pub fn save(ic: &mut Interceptor, _: &[&str]) {
    match ic.features.save_world.clone() {
        Some(w) => {
            say(ic, format!("warping to save world `w{w}``..."));
            go_to(ic, &w);
        }
        None => warn(ic, format!("no save world yet, use {}setsave <world>", ic.cfg.command.prefix)),
    }
}

// ── World owner tools ────────────────────────────────────────────────────────

pub fn wrench(ic: &mut Interceptor, args: &[&str]) {
    let mode = match args.first() {
        None => match ic.features.wrench {
            None => Some(WrenchAction::Pull),
            Some(_) => None,
        },
        Some(&"off") => None,
        Some(a) => match WrenchAction::parse(a) {
            Some(m) => Some(m),
            None => {
                say(ic, format!("`4usage:`` {}wrench [pull|kick|ban|off]", ic.cfg.command.prefix));
                return;
            }
        },
    };
    ic.features.wrench = mode;
    match mode {
        Some(m) => say(ic, format!("wrench mode: wrenching a player will `w{}`` them", m.command())),
        None => say(ic, "wrench mode `4off``"),
    }
}

/// `/wrench` mode: turns wrenching a player into `/pull|kick|ban <name>`.
pub fn auto_wrench(ic: &mut Interceptor, tp: &TextPacket) -> Option<Verdict> {
    let mode = ic.features.wrench?;
    let net_id: i32 = tp.get("netid").and_then(|v| v.parse().ok())?;
    if Some(net_id) == ic.session.net_id {
        return None;
    }
    let name = ic.session.player_by_net_id(net_id)?.to_string();
    if mode == WrenchAction::Pull {
        ic.features.last_pulled = Some(name.clone());
    }
    ic.emit(Effect::chat(&format!("/{} {name}", mode.command())));
    say(ic, format!("{} `w{name}``", mode.command()));
    Some(Verdict::Drop)
}

pub fn pull_all(ic: &mut Interceptor, _: &[&str]) {
    everyone(ic, WrenchAction::Pull);
}

pub fn kick_all(ic: &mut Interceptor, _: &[&str]) {
    everyone(ic, WrenchAction::Kick);
}

pub fn ban_all(ic: &mut Interceptor, _: &[&str]) {
    everyone(ic, WrenchAction::Ban);
}

fn everyone(ic: &mut Interceptor, action: WrenchAction) {
    let me = ic.session.net_id;
    let names: Vec<String> =
        ic.session.players.iter().filter(|(id, _)| Some(**id) != me).map(|(_, n)| n.clone()).collect();
    if names.is_empty() {
        warn(ic, "nobody else is in this world");
        return;
    }
    for (i, name) in names.iter().enumerate() {
        let cmd = Effect::chat(&format!("/{} {name}", action.command()));
        ic.emit(if i == 0 { cmd } else { cmd.later(STEP_SPACING * i as u32) });
    }
    say(ic, format!("{} `w{}`` players...", action.command(), names.len()));
}

// ── Spam ─────────────────────────────────────────────────────────────────────

pub fn spam(ic: &mut Interceptor, args: &[&str]) {
    if args.first() == Some(&"stop") {
        ic.features.spam.running = false;
        say(ic, "spam stopped");
        return;
    }
    spam_dialog(ic);
}

fn spam_dialog(ic: &mut Interceptor) {
    let s = &ic.features.spam;
    let d = Dialog::new("spam")
        .title("Auto spam", DEFAULT_ICON)
        .small("Sends the texts to chat in turn. Too short an interval gets you muted.")
        .field("Status", onoff(s.running))
        .spacer()
        .text_input("text1", "Text 1", &s.text1, 120)
        .text_input("text2", "Text 2 (optional)", &s.text2, 120)
        .text_input("interval", "Interval (seconds, min 2)", &s.interval.as_secs().to_string(), 4)
        .checkbox("rainbow", "Rainbow colours", s.rainbow)
        .spacer()
        .button(if s.running { "stop" } else { "start" }, if s.running { "`4Stop``" } else { "`2Start``" });
    show(ic, d, "Close", "");
}

// ── Dialog answers ───────────────────────────────────────────────────────────

/// Handles our tool dialogs; `false` if the dialog is not one of them.
pub fn on_dialog(ic: &mut Interceptor, ret: &DialogReturn) -> bool {
    match ret.name {
        "ui" => {
            if ret.button == Some("reset") {
                ic.features.ui_color = None;
                say(ic, "dialog colours back to normal");
                return true;
            }
            let get = |k: &str| ret.field(k).and_then(|v| v.trim().parse::<u8>().ok());
            match (get("red"), get("green"), get("blue"), get("alpha")) {
                (Some(r), Some(g), Some(b), Some(a)) => {
                    ic.features.ui_color = Some([r, g, b, a]);
                    say(ic, format!("dialog colour set to `w{r},{g},{b},{a}``"));
                }
                _ => warn(ic, "colours must be numbers from 0 to 255"),
            }
            true
        }
        "spam" => {
            let s = &mut ic.features.spam;
            s.text1 = ret.field("text1").unwrap_or_default().trim().to_string();
            s.text2 = ret.field("text2").unwrap_or_default().trim().to_string();
            if let Some(secs) = ret.field("interval").and_then(|v| v.trim().parse::<u64>().ok()) {
                s.interval = Duration::from_secs(secs).max(SPAM_MIN_INTERVAL);
            }
            s.rainbow = ret.field("rainbow") == Some("1");
            match ret.button {
                Some("start") if s.text1.is_empty() => warn(ic, "Text 1 can't be empty"),
                Some("start") => {
                    s.running = true;
                    s.next_at = Instant::now();
                    s.turn = 0;
                    let every = s.interval.as_secs();
                    say(ic, format!("spam started, every {every}s (`w/spam stop`` to stop)"));
                }
                Some("stop") => {
                    s.running = false;
                    say(ic, "spam stopped");
                }
                _ => {}
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::protocol::message::encode_text;
    use crate::protocol::{GamePacketType, MessageType};
    use crate::proxy::Direction;
    use std::sync::Arc;

    fn ic() -> Interceptor {
        let mut cfg = Config::default();
        cfg.log.text = false;
        cfg.log.variant = false;
        Interceptor::new(Arc::new(cfg))
    }

    fn input(ic: &mut Interceptor, text: &str) -> Verdict {
        ic.handle(Direction::ToServer, &encode_text(MessageType::GenericText, &format!("action|input\n|text|{text}")))
    }

    /// Chat lines the proxy sends to the server, including delayed ones.
    fn chats(effects: Vec<Effect>) -> Vec<String> {
        effects
            .into_iter()
            .filter_map(|e| {
                let e = match e {
                    Effect::Later(_, inner) => *inner,
                    e => e,
                };
                match e {
                    Effect::ToServer(b) => {
                        let t = String::from_utf8_lossy(&b[4..b.len() - 1]).into_owned();
                        t.strip_prefix("action|input\n|text|").map(|c| c.trim_end().to_string())
                    }
                    _ => None,
                }
            })
            .collect()
    }

    fn spawn(ic: &mut Interceptor, net_id: i32, name: &str, local: bool) {
        let kind = if local { "type|local\n" } else { "" };
        let vl = VariantList::new(vec![
            Variant::String("OnSpawn".into()),
            Variant::String(format!("spawn|avatar\nnetID|{net_id}\nname|`w{name}``\ncountry|us\n{kind}")),
        ]);
        let pkt = GameUpdatePacket::call_function(-1, vl.serialize());
        ic.handle(Direction::ToClient, &Message::Game(pkt).encode());
    }

    #[test]
    fn inventory_from_server_and_balance() {
        let mut ic = ic();
        let mut body = vec![1];
        body.extend_from_slice(&30u32.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&[242, 0, 50, 0, 0x04, 0x07, 3, 0]);
        let pkt = GameUpdatePacket {
            packet_type: GamePacketType::SendInventoryState,
            flags: crate::protocol::game_packet::PacketFlags::EXTENDED,
            extra: body,
            ..Default::default()
        };
        ic.handle(Direction::ToClient, &Message::Game(pkt).encode());
        assert_eq!((ic.inventory.count(WL), ic.inventory.count(DL)), (50, 3));

        let removed = GameUpdatePacket {
            packet_type: GamePacketType::ModifyItemInventory,
            value: WL,
            jump_count: 10,
            ..Default::default()
        };
        ic.handle(Direction::ToClient, &Message::Game(removed).encode());
        assert_eq!(ic.inventory.count(WL), 40);
        assert_eq!(input(&mut ic, "/balance"), Verdict::Drop);
    }

    #[test]
    fn wrench_mode_and_pull_all() {
        let mut ic = ic();
        spawn(&mut ic, 1, "Me", true);
        spawn(&mut ic, 5, "Bob", false);
        spawn(&mut ic, 6, "Alice", false);

        input(&mut ic, "/wrench kick");
        ic.take_effects();
        let wrench = encode_text(MessageType::GenericText, "action|wrench\n|netid|5");
        assert_eq!(ic.handle(Direction::ToServer, &wrench), Verdict::Drop);
        assert_eq!(chats(ic.take_effects()), vec!["/kick Bob"]);

        // Wrenching yourself still opens your own menu.
        let me = encode_text(MessageType::GenericText, "action|wrench\n|netid|1");
        assert_eq!(ic.handle(Direction::ToServer, &me), Verdict::Forward);

        input(&mut ic, "/pullall");
        assert_eq!(chats(ic.take_effects()), vec!["/pull Bob", "/pull Alice"]);
    }

    #[test]
    fn custom_drop_breaks_a_diamond_lock() {
        let mut ic = ic();
        ic.inventory.add(WL, 40);
        ic.inventory.add(DL, 2);
        input(&mut ic, "/cd 150");
        let effects = ic.take_effects();
        // convert 1 DL, then drop requests for DL and WL
        let converts = effects
            .iter()
            .filter(|e| matches!(e, Effect::ToServer(b) if b[0] == 4) || matches!(e, Effect::Later(_, i) if matches!(**i, Effect::ToServer(ref b) if b[0] == 4)))
            .count();
        assert_eq!(converts, 1);
        let plans: Vec<(u32, Option<u32>)> = ic.planned_drops.iter().map(|p| (p.item_id, p.amount)).collect();
        assert_eq!(plans, vec![(DL, Some(1)), (WL, Some(50))]);
    }

    #[test]
    fn country_override_rewrites_spawn() {
        let mut ic = ic();
        ic.features.country = Some("tr".into());
        let vl = VariantList::new(vec![
            Variant::String("OnSpawn".into()),
            Variant::String("spawn|avatar\nnetID|1\nname|`wMe``\ncountry|us|showGuild\ntype|local\n".into()),
        ]);
        let raw = Message::Game(GameUpdatePacket::call_function(-1, vl.serialize())).encode();
        let Verdict::Replace(out) = ic.handle(Direction::ToClient, &raw) else { panic!("not rewritten") };
        let Some(Message::Game(p)) = Message::decode(&out) else { panic!() };
        let text = VariantList::parse(&p.extra).unwrap().get(1).unwrap().as_str().unwrap().to_string();
        assert!(text.contains("country|tr|showGuild\n"), "{text}");
        assert_eq!(ic.session.country.as_deref(), Some("us"));
    }

    #[test]
    fn spam_settings_from_dialog() {
        let mut ic = ic();
        let ret = encode_text(
            MessageType::GenericText,
            "action|dialog_return\ndialog_name|gtproxy_spam\ntext1|hello\ntext2|\ninterval|1\nrainbow|0\nbuttonClicked|start\n",
        );
        assert_eq!(ic.handle(Direction::ToServer, &ret), Verdict::Drop);
        assert!(ic.features.spam.running);
        assert_eq!(ic.features.spam.interval, SPAM_MIN_INTERVAL);
        ic.take_effects();
        ic.tick();
        assert_eq!(chats(ic.take_effects()), vec!["hello"]);
    }

    /// Function name and first argument of every call sent to the client.
    fn client_calls(effects: Vec<Effect>) -> Vec<(String, Variant)> {
        effects
            .into_iter()
            .filter_map(|e| match e {
                Effect::ToClient(b) => match Message::decode(&b)? {
                    Message::Game(p) => {
                        let vl = VariantList::parse(&p.extra)?;
                        Some((vl.function()?.to_string(), vl.get(1)?.clone()))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn nick_and_skin_undo() {
        let mut ic = ic();
        spawn(&mut ic, 1, "Me", true);
        let clothing = VariantList::new(vec![
            Variant::String("OnSetClothing".into()),
            Variant::Vec3(0.0, 0.0, 0.0),
            Variant::Vec3(0.0, 0.0, 0.0),
            Variant::Vec3(0.0, 0.0, 0.0),
            Variant::Unsigned(4042322175),
            Variant::Vec3(0.0, 0.0, 0.0),
        ]);
        ic.handle(Direction::ToClient, &Message::Game(GameUpdatePacket::call_function(1, clothing.serialize())).encode());

        input(&mut ic, "/nick Fake");
        input(&mut ic, "/nick");
        input(&mut ic, "/skin 100");
        input(&mut ic, "/skin");
        let calls: Vec<(String, Variant)> =
            client_calls(ic.take_effects()).into_iter().filter(|(f, _)| f != "OnConsoleMessage").collect();
        assert_eq!(
            calls,
            vec![
                ("OnNameChanged".into(), Variant::String("Fake".into())),
                ("OnNameChanged".into(), Variant::String("`wMe``".into())),
                ("OnChangeSkin".into(), Variant::Unsigned(100)),
                ("OnChangeSkin".into(), Variant::Unsigned(4042322175)),
            ]
        );
    }

    #[test]
    fn warnings_go_to_the_middle_of_the_screen() {
        let mut ic = ic();
        input(&mut ic, "/pullall");
        let calls = client_calls(ic.take_effects());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "OnTextOverlay");
    }

    #[test]
    fn country_state() {
        assert_eq!(with_country("us|showGuild|doctor", "tr"), "tr|showGuild|doctor");
        assert_eq!(with_country("us", "tr"), "tr");
    }

    #[test]
    fn markup_values() {
        let m = "add_label_with_icon|big|`wTelephone``|left|3898|\nembed_data|tilex|12\nembed_data|tiley|30\n";
        assert_eq!(dialog_value(m, "tilex"), Some("12"));
        assert_eq!(dialog_value(m, "tiley"), Some("30"));
        assert_eq!(dialog_value(m, "num"), None);
    }
}
