//! In-game proxy commands, typed into chat with the configured prefix.
//!
//! A command the proxy does not know is passed on to the server untouched,
//! so the game's own commands (`/msg`, `/sb`, ...) keep working.

use std::time::Duration;

use super::dialog::{DEFAULT_ICON, Dialog, DialogReturn};
use super::effect::Effect;
use super::interceptor::Interceptor;
use super::fast_item::{LOCKS, PlannedDrop};
use super::roulette::SpinMode;
use super::tools;
use super::{chat_tools, collect, dropall, fake_drop, history, info, pathing, scan, vends, settings, social, stats, teleport, worlds, zoom};

/// Largest stack the game allows in one inventory slot.
const MAX_STACK: u32 = 200;
/// Gap between the drop requests of `/daw`.
const DROP_SPACING: Duration = Duration::from_millis(400);

/// Roulette Wheel item, shown in the roulette dialog title.
const ROULETTE_ICON: u32 = 758;

struct Command {
    name: &'static str,
    usage: &'static str,
    help: &'static str,
    run: fn(&mut Interceptor, &[&str]),
}

const COMMANDS: &[Command] = &[
    Command { name: "proxyhelp", usage: "", help: "window with all proxy commands", run: proxy_help },
    Command { name: "proxy", usage: "", help: "open the proxy status window", run: status },
    Command { name: "warp", usage: "<world>", help: "go to a world (fast, 3 s cooldown)", run: super::warp::warp },
    Command { name: "back", usage: "", help: "go to the previous world", run: back },
    Command { name: "relog", usage: "", help: "leave and rejoin this world", run: relog },
    Command { name: "exit", usage: "", help: "leave to the world menu", run: exit },
    Command { name: "logoff", usage: "", help: "log off to the login screen", run: dropall::logoff },
    Command { name: "logout", usage: "", help: "same as logoff", run: dropall::logoff },
    Command {
        name: "spin",
        usage: "[csn|qq|reme]",
        help: "roulette checker settings, or set the game mode",
        run: spin,
    },
    Command { name: "dw", usage: "<amount>", help: "drop World Locks", run: drop_wl },
    Command { name: "dd", usage: "<amount>", help: "drop Diamond Locks (breaks/uses BGL when short)", run: tools::drop_dl },
    Command { name: "db", usage: "<amount>", help: "drop Blue Gem Locks", run: drop_bgl },
    Command { name: "daw", usage: "", help: "drop all your locks", run: drop_all_locks },
    Command { name: "dropall", usage: "[stop]", help: "drop every item (asks first)", run: dropall::dropall },
    Command { name: "dropall1", usage: "", help: "drop one of each item (asks first)", run: dropall::dropall1 },
    Command { name: "cd", usage: "<WL amount>", help: "drop a WL value, breaking locks if needed", run: tools::custom_drop },
    Command { name: "balance", usage: "", help: "your locks and their WL total", run: tools::balance },
    Command { name: "inventory", usage: "", help: "inventory window", run: tools::inventory },
    Command { name: "account", usage: "", help: "account window", run: tools::account },
    Command { name: "change", usage: "", help: "Telephone: 100 DL -> 1 BGL automatically", run: tools::change },
    Command { name: "setsave", usage: "<world>", help: "set your save world", run: tools::set_save },
    Command { name: "save", usage: "", help: "go to your save world", run: tools::save },
    Command { name: "wrench", usage: "[pull|kick|ban|off]", help: "wrenching a player pulls/kicks/bans", run: tools::wrench },
    Command { name: "pullall", usage: "", help: "pull everyone in the world", run: tools::pull_all },
    Command { name: "kickall", usage: "", help: "kick everyone in the world", run: tools::kick_all },
    Command { name: "banall", usage: "", help: "ban everyone in the world", run: tools::ban_all },
    Command { name: "spam", usage: "[stop]", help: "auto spam window", run: tools::spam },
    Command { name: "country", usage: "<code|off>", help: "change your flag (only you see it)", run: tools::country },
    Command { name: "nick", usage: "<name>", help: "change your name (only you see it)", run: tools::nick },
    Command { name: "skin", usage: "<code>", help: "change your skin colour (only you see it)", run: tools::skin },
    Command { name: "ui", usage: "[off]", help: "dialog background colour", run: tools::ui },
    Command { name: "news", usage: "", help: "show the Growtopia Gazette again", run: tools::news },
    Command { name: "debug", usage: "", help: "proxy internals: session, players", run: tools::debug },
    Command { name: "options", usage: "", help: "options window (all switches)", run: settings::options },
    Command { name: "autosave", usage: "", help: "keep settings between runs", run: settings::autosave },
    Command { name: "sworld", usage: "[:door] [note]", help: "save the world you are in", run: worlds::save_world },
    Command { name: "list", usage: "[number]", help: "saved worlds window, or warp to one", run: worlds::list },
    Command { name: "hideclothes", usage: "", help: "hide other players' clothes", run: social::hideclothes },
    Command { name: "hidename", usage: "[name]", help: "hide a player's name tag", run: social::hidename },
    Command { name: "autopullpage", usage: "", help: "auto pull list window", run: social::autopull_page },
    Command { name: "autobanpage", usage: "", help: "auto ban list window", run: social::autoban_page },
    Command { name: "addpull", usage: "<name>", help: "add to the auto pull list", run: social::add_pull },
    Command { name: "addban", usage: "<name>", help: "add to the auto ban list", run: social::add_ban },
    Command { name: "banwords", usage: "", help: "ban whoever says a listed word", run: social::banwords_page },
    Command { name: "lp", usage: "", help: "pull the last pulled player again", run: social::last_pull },
    Command { name: "me", usage: "", help: "send every chat line as /me", run: chat_tools::me },
    Command { name: "formalize", usage: "", help: "capitalise and punctuate your chat", run: chat_tools::formalize },
    Command { name: "calc", usage: "<expression>", help: "calculator, e.g. 25+50*2", run: chat_tools::calc },
    Command { name: "timer", usage: "", help: "start/stop an on-screen timer", run: chat_tools::timer },
    Command { name: "pingalert", usage: "[ms|off]", help: "warn when ping gets high", run: chat_tools::ping_alert },
    Command { name: "spinstats", usage: "", help: "roulette statistics window", run: stats::spinstats },
    Command { name: "chat", usage: "<text>", help: "send a chat line or game command (e.g. /pull name)", run: chat },
    Command { name: "vendfind", usage: "[item]", help: "search the vending machines seen in worlds", run: vends::vendfind },
    Command { name: "scan", usage: "[blocks|bg|float] [name]", help: "Growscan: blocks and floating items in this world", run: scan::scan },
    Command { name: "dlocal", usage: "[amount|clear]", help: "fake Dirt drop only you can see", run: fake_drop::dlocal },
    Command { name: "zoom", usage: "[level]", help: "unlimited zoom on/off, or set a level", run: zoom::zoom },
    Command { name: "tp", usage: "[x y | speed ms]", help: "Ctrl+click teleport on/off, or go to a tile", run: teleport::tp },
    Command { name: "gs", usage: "", help: "Growscan shortcut", run: scan::scan },
    Command { name: "blocks", usage: "", help: "Growscan: world blocks", run: scan::blocks },
    Command { name: "items", usage: "", help: "Growscan: floating items", run: scan::items },
    Command { name: "vf", usage: "[item]", help: "vendfind shortcut", run: vends::vendfind },
    Command { name: "pvend", usage: "<item>", help: "walk to the cheapest vend selling it here", run: pathing::pvend },
    Command { name: "pfind", usage: "<item>", help: "walk to the closest floating item", run: pathing::pfind },
    Command { name: "tpto", usage: "<player>", help: "walk next to a player", run: pathing::tpto },
    Command { name: "collect", usage: "[now]", help: "collect window: range, one item or all, auto collect", run: collect::collect },
    Command { name: "autocollect", usage: "", help: "collect window", run: collect::collect },
    Command { name: "ac", usage: "", help: "auto collect on/off", run: collect::ac },
    Command { name: "door", usage: "<door ID>", help: "go to a door of this world", run: pathing::door },
    Command { name: "admin", usage: "", help: "lock owners and who has access", run: info::admin },
    Command { name: "locked", usage: "", help: "doors and their labels", run: info::locked },
    Command { name: "vendcopy", usage: "", help: "copy this world's vend prices", run: info::vendcopy },
    Command { name: "roulette", usage: "", help: "roulette spin history", run: history::roulette },
    Command { name: "drophistory", usage: "", help: "who dropped and picked up what", run: history::drophistory },
    Command { name: "track", usage: "", help: "others' drops and pickups in chat", run: history::track },
    Command { name: "hidelevel", usage: "", help: "hide \"You need to be Level\" text", run: info::hidelevel },
    Command { name: "minimal", usage: "", help: "hide proxy chat messages", run: info::minimal },
    Command { name: "terminal", usage: "[on|off]", help: "next start: terminal (on) or window (off)", run: info::terminal },
    Command { name: "rndm", usage: "", help: "warp to a random world", run: info::rndm },
    Command { name: "re", usage: "", help: "run the last command again", run: info::re },
    Command { name: "fastdrop", usage: "", help: "drop the whole stack without asking", run: fast_drop },
    Command { name: "fasttrash", usage: "", help: "trash the whole stack without asking", run: fast_trash },
    Command {
        name: "log",
        usage: "[text|game|variant|redact] [on|off]",
        help: "show or toggle packet logging",
        run: log,
    },
];

/// Returns `true` when the line was a proxy command and must not reach the server.
pub fn run(ic: &mut Interceptor, line: &str) -> bool {
    let mut parts = line.split_whitespace();
    let Some(name) = parts.next() else { return false };
    let args: Vec<&str> = parts.collect();
    let Some(cmd) = COMMANDS.iter().find(|c| c.name.eq_ignore_ascii_case(name)) else {
        return false;
    };
    tracing::info!("command: {line}");
    if cmd.name != "re" {
        ic.features.last_command = Some(line.to_string());
    }
    (cmd.run)(ic, &args);
    true
}

/// Capitalises a message that starts with a plain word ("dropped 5" ->
/// "Dropped 5"). Text that opens with a colour code is a name or value and
/// keeps its case.
pub(super) fn sentence(text: impl Into<String>) -> String {
    let text = text.into();
    let mut chars = text.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => c.to_ascii_uppercase().to_string() + chars.as_str(),
        _ => text,
    }
}

/// Important warnings: shown in the middle of the screen instead of chat.
pub(super) fn warn(ic: &mut Interceptor, text: impl Into<String>) {
    ic.emit(Effect::overlay(format!("`4{}``", sentence(text))));
}

pub(super) fn say(ic: &mut Interceptor, text: impl Into<String>) {
    if ic.features.minimal {
        return;
    }
    ic.emit(Effect::console(format!("`9[proxy]`` {}", sentence(text))));
}

fn proxy_help(ic: &mut Interceptor, _: &[&str]) {
    commands_dialog(ic);
}

fn status(ic: &mut Interceptor, _: &[&str]) {
    let s = &ic.session;
    let l = &ic.log;
    let ms = |d: Option<Duration>| d.map_or("-".to_string(), |d| format!("{}ms", d.as_millis()));
    let or_dash = |v: Option<String>| v.unwrap_or_else(|| "-".to_string());
    let onoff = |b: bool| if b { "`2on``" } else { "`4off``" };

    let dialog = Dialog::new("status")
        .title(&format!("GTProxy v{}", env!("CARGO_PKG_VERSION")), DEFAULT_ICON)
        .small(&format!("Running for {}", fmt_uptime(ic.started.elapsed())))
        .spacer()
        .heading("Connection")
        .field("Server", &or_dash(s.server.map(|a| a.to_string())))
        .field("Ping game <-> proxy", &ms(ic.link.client_rtt))
        .field("Ping proxy <-> server", &ms(ic.link.server_rtt))
        .spacer()
        .heading("Player")
        .field("Name", &or_dash(s.name.clone()))
        .field("netID", &or_dash(s.net_id.map(|n| n.to_string())))
        .field("userID", &or_dash(s.user_id.map(|n| n.to_string())))
        .spacer()
        .heading("World")
        .field("Current", s.world.as_deref().unwrap_or("EXIT"))
        .field("Previous", &or_dash(s.previous_world.clone()))
        .spacer()
        .heading("Features")
        .field("Fast drop", onoff(ic.fast_drop))
        .field("Fast trash", onoff(ic.fast_trash))
        .field("Roulette", &format!("{} ({})", onoff(ic.roulette.enabled), ic.roulette.mode.name()))
        .button("fast_drop", "Toggle fast drop")
        .button("fast_trash", "Toggle fast trash")
        .button("roulette", "Roulette settings")
        .spacer()
        .heading("Packet log")
        .text(&format!(
            "Text {} | Game {} | Variant {} | Redact {}",
            onoff(l.text),
            onoff(l.game_packet),
            onoff(l.variant),
            onoff(l.redact)
        ))
        .button("log_text", "Toggle text")
        .button("log_game", "Toggle game")
        .button("log_variant", "Toggle variant")
        .spacer()
        .button("refresh", "`2Refresh``")
        .button("commands", "Commands");
    tools::show(ic, dialog, "Close", "");
}

fn drop_wl(ic: &mut Interceptor, args: &[&str]) {
    drop_lock(ic, 0, args);
}

fn drop_bgl(ic: &mut Interceptor, args: &[&str]) {
    drop_lock(ic, 2, args);
}

/// `/dw 10`: asks the server to drop; the amount is checked against the
/// dialog's "you have" count before anything is dropped.
fn drop_lock(ic: &mut Interceptor, lock: usize, args: &[&str]) {
    let (item_id, name) = LOCKS[lock];
    let amount = args.first().and_then(|a| a.parse::<u32>().ok()).filter(|&n| (1..=MAX_STACK).contains(&n));
    let Some(amount) = amount else {
        let cmd = ["dw", "dd", "db"][lock];
        say(ic, format!("`4usage:`` {}{cmd} <1-{MAX_STACK}>", ic.cfg.command.prefix));
        return;
    };
    // The server stays silent when you have none, so check what we know first.
    if ic.inventory.slots > 0 {
        match ic.inventory.count(item_id) {
            0 => return warn(ic, format!("You have no {name}")),
            owned if owned < amount => return warn(ic, format!("You only have {owned} {name}")),
            _ => {}
        }
    }
    plan_drop(ic, item_id, name, Some(amount), Duration::ZERO);
}

/// `/daw`: every lock type, whole stacks. Requests are spaced out a little;
/// a lock you do not have simply gets no dialog and its plan expires.
fn drop_all_locks(ic: &mut Interceptor, _: &[&str]) {
    let known = ic.inventory.slots > 0;
    let owned: Vec<(u32, &'static str)> =
        LOCKS.into_iter().filter(|(id, _)| !known || ic.inventory.count(*id) > 0).collect();
    if owned.is_empty() {
        return warn(ic, "You don't have any locks");
    }
    for (i, (item_id, name)) in owned.into_iter().enumerate() {
        plan_drop(ic, item_id, name, None, DROP_SPACING * i as u32);
    }
    say(ic, "Dropping all locks...");
}

pub(super) fn plan_drop(ic: &mut Interceptor, item_id: u32, name: impl Into<String>, amount: Option<u32>, delay: Duration) {
    let plan = PlannedDrop::new(item_id, name, amount, delay);
    let request = Effect::text(plan.request());
    ic.planned_drops.push(plan);
    ic.emit(if delay.is_zero() { request } else { request.later(delay) });
}

fn fast_drop(ic: &mut Interceptor, _: &[&str]) {
    ic.fast_drop = !ic.fast_drop;
    let state = if ic.fast_drop { "`2on``" } else { "`4off``" };
    say(ic, format!("fast drop {state}"));
}

fn fast_trash(ic: &mut Interceptor, _: &[&str]) {
    ic.fast_trash = !ic.fast_trash;
    let state = if ic.fast_trash { "`2on``" } else { "`4off``" };
    say(ic, format!("fast trash {state}"));
}

fn chat(ic: &mut Interceptor, args: &[&str]) {
    if args.is_empty() {
        return warn(ic, format!("usage: {}chat <text>", ic.cfg.command.prefix));
    }
    let line = args.join(" ");
    super::social::note_typed_command(ic, &line);
    ic.emit(Effect::chat(&line));
}

fn spin(ic: &mut Interceptor, args: &[&str]) {
    match args.first() {
        None => spin_dialog(ic),
        Some(&"toggle") => {
            ic.roulette.enabled = !ic.roulette.enabled;
            let state = if ic.roulette.enabled { "`2on``" } else { "`4off``" };
            say(ic, format!("Roulette checker {state}"));
        }
        Some(&"instant") => {
            ic.roulette.instant = !ic.roulette.instant;
            let state = if ic.roulette.instant { "`2on``" } else { "`4off``" };
            say(ic, format!("Show spins instantly {state}"));
        }
        Some(arg) => match SpinMode::parse(arg) {
            Some(mode) => {
                ic.roulette.mode = mode;
                say(ic, format!("roulette mode: `w{}``", mode.name()));
            }
            None => say(ic, format!("`4usage:`` {}spin [csn|qq|reme]", ic.cfg.command.prefix)),
        },
    }
}

fn spin_dialog(ic: &mut Interceptor) {
    let r = &ic.roulette;
    let onoff = |b: bool| if b { "`2on``" } else { "`4off``" };
    let mode_button = |mode: SpinMode| {
        if r.mode == mode { format!("`2> {} <``", mode.name()) } else { mode.name().to_string() }
    };
    let dialog = Dialog::new("roulette")
        .title("Roulette checker", ROULETTE_ICON)
        .small("Spins in the server's exact format start with [`2REAL``]; QQ and Reme add [`2QQ`` n] or [`2REME`` n] at the end.")
        .spacer()
        .field("Checker", &onoff(r.enabled))
        .field("Show instantly", &onoff(r.instant))
        .field("Game", r.mode.name())
        .spacer()
        .heading("Game")
        .button("mode_csn", &mode_button(SpinMode::Csn))
        .button("mode_qq", &mode_button(SpinMode::Qq))
        .button("mode_reme", &mode_button(SpinMode::Reme))
        .small("QQ: last digit (23 -> 3). Reme: digit sum, last digit (23 -> 5, 29 -> 1, 19 -> 0).")
        .spacer()
        .button("toggle_enabled", if r.enabled { "Turn checker off" } else { "Turn checker on" })
        .button("toggle_instant", if r.instant { "Use normal delay" } else { "Show instantly" });
    tools::show(ic, dialog, "Close", "");
}

fn commands_dialog(ic: &mut Interceptor) {
    let p = ic.cfg.command.prefix;
    let mut dialog = Dialog::new("commands").title("Proxy commands", DEFAULT_ICON).spacer();
    for c in COMMANDS {
        dialog = dialog.text(&format!("`w{p}{} {}`` `s- {}``", c.name, c.usage, c.help));
    }
    let dialog = dialog
        .spacer()
        .small("Commands the proxy does not know are sent to the game as usual.")
        .button("status", "Back to status");
    tools::show(ic, dialog, "Close", "");
}

/// Handles a button press on one of the proxy's dialogs.
pub fn on_dialog(ic: &mut Interceptor, ret: DialogReturn) {
    tracing::debug!("dialog {} -> {:?}", ret.name, ret.button);
    if tools::on_dialog(ic, &ret)
        || settings::on_dialog(ic, &ret)
        || social::on_dialog(ic, &ret)
        || worlds::on_dialog(ic, &ret)
        || stats::on_dialog(ic, &ret)
        || scan::on_dialog(ic, &ret)
        || vends::on_dialog(ic, &ret)
        || history::on_dialog(ic, &ret)
        || collect::on_dialog(ic, &ret)
        || dropall::on_dialog(ic, &ret)
    {
        return;
    }
    let toggle = |ic: &mut Interceptor, slot: fn(&mut Interceptor) -> &mut bool| {
        let v = slot(ic);
        *v = !*v;
        status(ic, &[]);
    };
    match (ret.name, ret.button) {
        (_, Some("refresh")) | (_, Some("status")) => status(ic, &[]),
        (_, Some("commands")) => commands_dialog(ic),
        ("status", Some("log_text")) => toggle(ic, |ic| &mut ic.log.text),
        ("status", Some("log_game")) => toggle(ic, |ic| &mut ic.log.game_packet),
        ("status", Some("log_variant")) => toggle(ic, |ic| &mut ic.log.variant),
        ("status", Some("fast_drop")) => toggle(ic, |ic| &mut ic.fast_drop),
        ("status", Some("fast_trash")) => toggle(ic, |ic| &mut ic.fast_trash),
        ("status", Some("roulette")) => spin_dialog(ic),
        ("roulette", Some(button)) => {
            let r = &mut ic.roulette;
            match button {
                "mode_csn" => r.mode = SpinMode::Csn,
                "mode_qq" => r.mode = SpinMode::Qq,
                "mode_reme" => r.mode = SpinMode::Reme,
                "toggle_enabled" => r.enabled = !r.enabled,
                "toggle_instant" => r.instant = !r.instant,
                _ => return,
            }
            spin_dialog(ic);
        }
        _ => {}
    }
}

fn fmt_uptime(d: Duration) -> String {
    let s = d.as_secs();
    match (s / 3600, s / 60 % 60, s % 60) {
        (0, 0, sec) => format!("{sec}s"),
        (0, m, sec) => format!("{m}m {sec}s"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

fn back(ic: &mut Interceptor, _: &[&str]) {
    match ic.session.previous_world.clone() {
        Some(world) => {
            say(ic, format!("back to `w{world}``..."));
            go_to(ic, &world);
        }
        None => warn(ic, "no previous world yet"),
    }
}

fn relog(ic: &mut Interceptor, _: &[&str]) {
    match ic.session.world.clone() {
        Some(world) => {
            say(ic, format!("rejoining `w{world}``..."));
            super::warp::go_to(ic, &world);
        }
        None => warn(ic, "you are not in a world"),
    }
}

fn exit(ic: &mut Interceptor, _: &[&str]) {
    if ic.session.world.is_none() {
        warn(ic, "you are not in a world");
        return;
    }
    ic.emit(Effect::quit_to_exit());
}

/// Goes to `world` (`NAME` or `NAME|DOOR`); see [`super::warp`].
pub(super) fn go_to(ic: &mut Interceptor, world: &str) {
    super::warp::go_to(ic, world);
}

fn log(ic: &mut Interceptor, args: &[&str]) {
    let flag = |b: bool| if b { "`2on``" } else { "`4off``" };
    if let Some(&which) = args.first() {
        let value = match args.get(1).map(|s| s.to_ascii_lowercase()) {
            Some(v) if v == "on" => Some(true),
            Some(v) if v == "off" => Some(false),
            Some(_) => {
                say(ic, "expected on or off");
                return;
            }
            None => None,
        };
        let slot = match which.to_ascii_lowercase().as_str() {
            "text" => &mut ic.log.text,
            "game" => &mut ic.log.game_packet,
            "variant" => &mut ic.log.variant,
            "redact" => &mut ic.log.redact,
            _ => {
                say(ic, "unknown log kind (text, game, variant, redact)");
                return;
            }
        };
        *slot = value.unwrap_or(!*slot);
    }
    let l = &ic.log;
    let text = format!(
        "log: text {} | game {} | variant {} | redact {}",
        flag(l.text),
        flag(l.game_packet),
        flag(l.variant),
        flag(l.redact)
    );
    say(ic, text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::sync::Arc;

    fn ic() -> Interceptor {
        Interceptor::new(Arc::new(Config::default()))
    }

    fn server_texts(effects: &[Effect]) -> Vec<String> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::ToServer(b) => Some(String::from_utf8_lossy(&b[4..]).into_owned()),
                Effect::Later(_, inner) => match inner.as_ref() {
                    Effect::ToServer(b) => Some(format!("later:{}", String::from_utf8_lossy(&b[4..]))),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn relog_leaves_and_joins() {
        let mut ic = ic();
        ic.session.enter_world("START".into());
        assert!(run(&mut ic, "relog"));
        let sent = server_texts(&ic.take_effects());
        assert_eq!(sent.len(), 1);
        assert!(sent[0].starts_with("action|quit_to_exit"));
    }

    #[test]
    fn log_toggle() {
        let mut ic = ic();
        let before = ic.log.game_packet;
        run(&mut ic, "log game");
        assert_eq!(ic.log.game_packet, !before);
        run(&mut ic, "log game off");
        assert!(!ic.log.game_packet);
    }
}
