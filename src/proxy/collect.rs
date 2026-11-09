//! `/collect` window and auto collect.
//!
//! Picking up does not need walking: the client sends
//! `ItemActivateObjectRequest` (`value` = object uid, `vec` = its position)
//! and the server accepts it from a few tiles away, as other proxies and
//! GrowBots do. We send one per object in range.
//!
//! Settings: range, only one item (or everything), skip gems, interval, a
//! chat line when something is collected, and auto collect on/off.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::protocol::{GamePacketType, GameUpdatePacket, Message};

use super::commands::{say, warn};
use super::dialog::{Dialog, DialogReturn};
use super::effect::Effect;
use super::interceptor::Interceptor;
use super::teleport::tile_of;
use super::world::WorldObject;

pub const MAX_RANGE: u32 = 10;
const DEFAULT_RANGE: u32 = 3;
const MIN_INTERVAL_MS: u64 = 100;
const DEFAULT_INTERVAL_MS: u64 = 500;
/// Requests sent in one go, so a pile of drops does not flood the server.
const MAX_PER_PASS: usize = 30;
/// An object we asked for is not asked for again this soon.
const RETRY_AFTER: Duration = Duration::from_secs(2);
const GEMS: u32 = 112;
const MAX_STACK: u32 = 200;
/// Gems icon, when the item list has no Magplant.
const FALLBACK_ICON: u32 = 112;

#[derive(Debug, Clone)]
pub struct Collect {
    pub auto: bool,
    pub range: u32,
    /// Only this item; `None` = everything.
    pub only: Option<u32>,
    pub skip_gems: bool,
    pub interval: Duration,
    /// Chat line for each auto collect pass that picked something up.
    pub show_message: bool,
    last_pass: Option<Instant>,
    asked: HashMap<u32, Instant>,
}

impl Default for Collect {
    fn default() -> Self {
        Self {
            auto: false,
            range: DEFAULT_RANGE,
            only: None,
            skip_gems: false,
            interval: Duration::from_millis(DEFAULT_INTERVAL_MS),
            show_message: false,
            last_pass: None,
            asked: HashMap::new(),
        }
    }
}

/// Distance in tiles (the larger of x and y).
fn distance(a: (i32, i32), b: (i32, i32)) -> u32 {
    (a.0 - b.0).unsigned_abs().max((a.1 - b.1).unsigned_abs())
}

/// Objects worth asking for now.
fn wanted(ic: &Interceptor, now: Instant) -> Vec<WorldObject> {
    let (Some(map), Some(pos)) = (ic.world.as_ref(), ic.session.pos) else { return Vec::new() };
    let c = &ic.collect;
    let me = tile_of(pos);
    let inv = &ic.inventory;
    let known_inv = inv.slots > 0;
    let mut out: Vec<WorldObject> = map
        .objects
        .iter()
        .filter(|o| o.item != 0)
        .filter(|o| c.only.is_none_or(|id| id == o.item as u32))
        .filter(|o| !(c.skip_gems && o.item as u32 == GEMS))
        .filter(|o| !ic.fake_drops.is_fake(o.uid))
        .filter(|o| c.asked.get(&o.uid).is_none_or(|t| now.duration_since(*t) >= RETRY_AFTER))
        .filter(|o| {
            // Room in the backpack: gems always fit, a stack below 200 or a free slot.
            let id = o.item as u32;
            !known_inv || id == GEMS || match inv.count(id) {
                0 => (inv.len() as u32) < inv.slots,
                n => n < MAX_STACK,
            }
        })
        .filter(|o| distance(tile_of((o.x, o.y)), me) <= c.range)
        .cloned()
        .collect();
    out.sort_by_key(|o| distance(tile_of((o.x, o.y)), me));
    out.truncate(MAX_PER_PASS);
    out
}

/// Asks the server for everything in range; returns how many were asked for.
pub fn collect_once(ic: &mut Interceptor) -> usize {
    let now = Instant::now();
    let list = wanted(ic, now);
    for o in &list {
        let pkt = GameUpdatePacket {
            packet_type: GamePacketType::ItemActivateObjectRequest,
            vec_x: o.x,
            vec_y: o.y,
            value: o.uid,
            ..Default::default()
        };
        ic.emit(Effect::ToServer(Message::Game(pkt).encode()));
        ic.collect.asked.insert(o.uid, now);
    }
    ic.collect.asked.retain(|_, t| now.duration_since(*t) < RETRY_AFTER * 5);
    list.len()
}

/// Called every bridge loop.
pub fn tick(ic: &mut Interceptor, now: Instant) {
    let c = &ic.collect;
    if !c.auto || ic.session.world.is_none() || ic.world.is_none() {
        return;
    }
    if c.last_pass.is_some_and(|t| now < t + c.interval) {
        return;
    }
    ic.collect.last_pass = Some(now);
    let n = collect_once(ic);
    if n > 0 && ic.collect.show_message {
        say(ic, format!("Auto collecting `w{n}`` items"));
    }
}

/// A world change forgets what we asked for.
pub fn reset(ic: &mut Interceptor) {
    ic.collect.asked.clear();
}

fn onoff(b: bool) -> &'static str {
    if b { "`2on``" } else { "`4off``" }
}

fn page(ic: &mut Interceptor) {
    let c = ic.collect.clone();
    let only = c.only.map(|id| ic.items.name(id)).unwrap_or_default();
    let floating: usize = ic.world.as_ref().map_or(0, |m| m.objects.len());
    let in_range = wanted(ic, Instant::now() + RETRY_AFTER).len();
    let icon = ic.items.find("Magplant 5000").unwrap_or(FALLBACK_ICON);
    let d = Dialog::new("collect")
        .title("Collect", icon)
        .small(&format!("{floating} objects in this world, `w{in_range}`` you can collect from here."))
        .spacer()
        .checkbox("auto", "`2Enable`` auto collect", c.auto)
        .checkbox("show", "`9Show`` a message when auto collecting", c.show_message)
        .checkbox("skip_gems", "Don't collect gems", c.skip_gems)
        .spacer()
        .small(&format!("Range in tiles, 1 to {MAX_RANGE}. The server may refuse far ones; 3 is safe."))
        .text_input("range", "Range:", &c.range.to_string(), 2)
        .small("Collect only this item (name or ID). Leave empty to collect everything.")
        .text_input("only", "Only:", &only, 40)
        .small(&format!("Auto collect interval in ms (at least {MIN_INTERVAL_MS})."))
        .text_input("interval", "Interval:", &c.interval.as_millis().to_string(), 5)
        .spacer()
        .button("collect_now", "`wCollect now``")
        .small(&format!("Auto collect is {}. Shortcuts: /ac on/off, /collect now", onoff(c.auto)));
    super::tools::show(ic, d, "Close", "Save");
}

/// `/collect` opens the window, `/collect now` collects once,
/// `/collect range <n>` and `/collect only <item|all>` change a setting.
pub fn collect(ic: &mut Interceptor, args: &[&str]) {
    match args.first().map(|a| a.to_ascii_lowercase()).as_deref() {
        None => page(ic),
        Some("now") => now_once(ic),
        Some("range") => match args.get(1).and_then(|a| a.parse::<u32>().ok()) {
            Some(r) if (1..=MAX_RANGE).contains(&r) => {
                ic.collect.range = r;
                say(ic, format!("Collect range `w{r}`` tiles"));
            }
            _ => warn(ic, format!("Range must be 1-{MAX_RANGE}")),
        },
        Some("only") => {
            let q = args[1..].join(" ");
            if q.is_empty() || q.eq_ignore_ascii_case("all") || q == "0" {
                ic.collect.only = None;
                say(ic, "Collecting every item");
                return;
            }
            match q.parse::<u32>().ok().or_else(|| ic.items.find(&q)).or_else(|| ic.items.search(&q)) {
                Some(id) => {
                    ic.collect.only = Some(id);
                    say(ic, format!("Collecting only `w{}``", ic.items.name(id)));
                }
                None => warn(ic, format!("No item called \"{q}\"")),
            }
        }
        Some(_) => warn(ic, format!("Usage: {0}collect, {0}collect now, {0}collect range <1-{MAX_RANGE}>, {0}collect only <item|all>", ic.cfg.command.prefix)),
    }
}

fn now_once(ic: &mut Interceptor) {
    if ic.world.is_none() {
        return warn(ic, "Enter a world first");
    }
    if ic.session.pos.is_none() {
        return warn(ic, "Move a little first so the proxy knows where you are");
    }
    match collect_once(ic) {
        0 => warn(ic, format!("Nothing to collect within {} tiles", ic.collect.range)),
        n => say(ic, format!("Collecting `w{n}`` items")),
    }
}

/// `/ac`: auto collect on/off.
pub fn ac(ic: &mut Interceptor, _: &[&str]) {
    ic.collect.auto = !ic.collect.auto;
    say(ic, format!("Auto collect {} (range {} tiles)", onoff(ic.collect.auto), ic.collect.range));
}

/// Reads the window's fields; bad values keep the old setting and say so.
fn apply_fields(ic: &mut Interceptor, ret: &DialogReturn) {
    let on = |id: &str| ret.field(id) == Some("1");
    let mut problems = Vec::new();
    ic.collect.auto = on("auto");
    ic.collect.show_message = on("show");
    ic.collect.skip_gems = on("skip_gems");
    match ret.field("range").map(str::trim).map(str::parse::<u32>) {
        Some(Ok(r)) if (1..=MAX_RANGE).contains(&r) => ic.collect.range = r,
        None => {}
        _ => problems.push(format!("range must be 1-{MAX_RANGE}")),
    }
    match ret.field("interval").map(str::trim).map(str::parse::<u64>) {
        Some(Ok(ms)) if ms >= MIN_INTERVAL_MS => ic.collect.interval = Duration::from_millis(ms),
        None => {}
        _ => problems.push(format!("interval must be at least {MIN_INTERVAL_MS} ms")),
    }
    let only = ret.field("only").unwrap_or_default().trim();
    if only.is_empty() || only == "0" {
        ic.collect.only = None;
    } else {
        let id = only.parse::<u32>().ok().or_else(|| ic.items.find(only)).or_else(|| ic.items.search(only));
        match id {
            Some(id) => ic.collect.only = Some(id),
            None => problems.push(format!("no item called \"{only}\"")),
        }
    }
    if !problems.is_empty() {
        warn(ic, problems.join(", "));
    }
}

pub fn on_dialog(ic: &mut Interceptor, ret: &DialogReturn) -> bool {
    if ret.name != "collect" {
        return false;
    }
    apply_fields(ic, ret);
    match ret.button {
        Some("collect_now") => {
            now_once(ic);
            page(ic);
        }
        _ => say(ic, format!("Collect settings saved, auto collect {}", onoff(ic.collect.auto))),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::protocol::TextPacket;
    use crate::proxy::world::{Tile, WorldMap};
    use std::sync::Arc;

    fn ic() -> Interceptor {
        let mut cfg = Config::default();
        cfg.log.text = false;
        cfg.log.variant = false;
        let mut ic = Interceptor::new(Arc::new(cfg));
        let mut map =
            WorldMap { name: "T".into(), width: 20, height: 5, tiles: vec![Tile::default(); 100], last_object_uid: Some(0), objects: Vec::new() };
        map.add_object(242, 64.0, 32.0, 1); // uid 1, 2 tiles away
        map.add_object(GEMS as u16, 32.0, 32.0, 5); // uid 2, 1 tile
        map.add_object(2, 15.0 * 32.0, 32.0, 5); // uid 3, 15 tiles
        ic.world = Some(map);
        ic.session.enter_world("T".into());
        ic.session.pos = Some((0.0, 32.0));
        ic
    }

    fn asked(ic: &mut Interceptor) -> Vec<u32> {
        ic.take_effects()
            .iter()
            .filter_map(|e| match e {
                Effect::ToServer(b) => match Message::decode(b) {
                    Some(Message::Game(p)) if p.packet_type == GamePacketType::ItemActivateObjectRequest => Some(p.value),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn collects_in_range_nearest_first_and_not_twice() {
        let mut ic = ic();
        assert_eq!(collect_once(&mut ic), 2);
        assert_eq!(asked(&mut ic), vec![2, 1]);
        assert_eq!(collect_once(&mut ic), 0); // already asked
    }

    #[test]
    fn only_one_item_and_skip_gems() {
        let mut ic = ic();
        ic.collect.range = MAX_RANGE;
        ic.collect.skip_gems = true;
        collect_once(&mut ic);
        assert_eq!(asked(&mut ic), vec![1]);
        let mut ic2 = self::ic();
        ic2.collect.range = 20;
        ic2.collect.only = Some(2);
        collect_once(&mut ic2);
        assert_eq!(asked(&mut ic2), vec![3]);
    }

    #[test]
    fn auto_collect_runs_on_tick_and_window_saves() {
        let mut ic = ic();
        let now = Instant::now();
        tick(&mut ic, now);
        assert!(asked(&mut ic).is_empty());
        let tp = TextPacket::parse(
            "action|dialog_return\ndialog_name|gtproxy_collect\nbuttonClicked|\nauto|1\nshow|0\nskip_gems|0\nrange|5\nonly|\ninterval|250\n",
        );
        on_dialog(&mut ic, &DialogReturn::parse(&tp).unwrap());
        assert!(ic.collect.auto);
        assert_eq!((ic.collect.range, ic.collect.interval), (5, Duration::from_millis(250)));
        ic.take_effects();
        tick(&mut ic, now);
        assert_eq!(asked(&mut ic), vec![2, 1]);
    }
}
