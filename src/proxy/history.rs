//! `/roulette` (spin history) and `/drophistory` (who dropped and picked up
//! what), plus `/track` that writes the same drop events to chat as they happen.
//!
//! Drops: the server's `ItemChangeObject` for a new object does not say who
//! dropped it, so it is put on the player standing closest to the spot.
//! Pickups name the player (`net_id`) and the object (`value` = uid).

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::protocol::GameUpdatePacket;

use super::commands::say;
use super::dialog::{Dialog, DialogReturn};
use super::effect::Effect;
use super::interceptor::Interceptor;
use super::roulette::Spin;
use super::teleport::tile_of;

const KEEP: usize = 200;
const SHOWN: usize = 60;
const ROULETTE_ICON: u32 = 758;
const HISTORY_ICON: u32 = 1796;
const NEW_OBJECT: i32 = -1;
/// A drop further than this (tiles) from everyone is "unknown".
const DROP_NEAR: i32 = 3;

#[derive(Debug, Clone)]
pub struct SpinEntry {
    pub player: String,
    pub number: u8,
    pub colour: char,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DropKind {
    Dropped,
    Collected,
}

#[derive(Debug, Clone)]
pub struct DropEntry {
    pub kind: DropKind,
    /// `None`: nobody was near the drop.
    pub player: Option<String>,
    pub item: u32,
    pub count: u32,
    pub world: String,
    pub at: u64,
}

#[derive(Debug, Default)]
pub struct History {
    /// Newest first.
    pub spins: VecDeque<SpinEntry>,
    pub drops: VecDeque<DropEntry>,
    /// `/track`: drop events of other players go to chat.
    pub track: bool,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn ago(at: u64) -> String {
    let s = now().saturating_sub(at);
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        _ => format!("{}h", s / 3600),
    }
}

fn push<T>(list: &mut VecDeque<T>, entry: T) {
    list.push_front(entry);
    list.truncate(KEEP);
}

// ── Roulette ─────────────────────────────────────────────────────────────────

pub fn record_spin(ic: &mut Interceptor, spin: &Spin) {
    crate::report::spin(ic.session.world.as_deref().unwrap_or("-"), spin.number, spin.colour);
    push(
        &mut ic.history.spins,
        SpinEntry { player: spin.player.clone(), number: spin.number, colour: spin.colour, at: now() },
    );
}

/// `/roulette`: the last spins with who spun them.
pub fn roulette(ic: &mut Interceptor, _: &[&str]) {
    let mode = ic.roulette.mode;
    let mut dialog = Dialog::new("roulette_history")
        .title("Roulette history", ROULETTE_ICON)
        .small(&format!("[REAL] spins since the proxy started, newest first. Game: `w{}``", mode.name()));
    if ic.history.spins.is_empty() {
        dialog = dialog.text("No spins seen yet.");
    }
    for s in ic.history.spins.iter().take(SHOWN) {
        let value = mode.value(s.number).map(|v| format!("  `s{} {v}``", mode.name())).unwrap_or_default();
        dialog = dialog.text(&format!("`{}{}``  `w{}``{value}  `s{} ago``", s.colour, s.number, s.player, ago(s.at)));
    }
    dialog = dialog.spacer().button("clear_spins", "`4Clear``");
    super::tools::show(ic, dialog, "Close", "");
}

// ── Drops ────────────────────────────────────────────────────────────────────

/// Who stands closest to pixel position `pos`, within [`DROP_NEAR`] tiles.
fn nearest_player(ic: &Interceptor, pos: (f32, f32)) -> Option<String> {
    let at = tile_of(pos);
    let me = ic.session.pos.map(|p| (ic.session.name.clone().unwrap_or_else(|| "You".into()), p));
    let others = ic
        .session
        .player_pos
        .iter()
        .filter_map(|(id, p)| ic.session.players.get(id).map(|n| (n.clone(), *p)));
    me.into_iter()
        .chain(others)
        .map(|(name, p)| {
            let t = tile_of(p);
            (name, (t.0 - at.0).abs().max((t.1 - at.1).abs()))
        })
        .filter(|(_, d)| *d <= DROP_NEAR)
        .min_by_key(|(_, d)| *d)
        .map(|(n, _)| n)
}

/// Server -> client `ItemChangeObject`, called before the world map forgets the object.
pub fn on_object(ic: &mut Interceptor, pkt: &GameUpdatePacket) {
    let Some(map) = ic.world.as_ref() else { return };
    let world = map.name.clone();
    let entry = match pkt.net_id {
        NEW_OBJECT => {
            let count = pkt.float_var.clamp(0.0, 255.0) as u32;
            // Object positions sit a little left/up of the tile they lie on.
            let player = nearest_player(ic, (pkt.vec_x - 4.0, pkt.vec_y - 8.0));
            DropEntry { kind: DropKind::Dropped, player, item: pkt.value, count, world, at: now() }
        }
        id if id > 0 => {
            let Some(o) = map.objects.iter().find(|o| o.uid == pkt.value) else { return };
            let player = if Some(id) == ic.session.net_id {
                ic.session.name.clone().or(Some("You".into()))
            } else {
                ic.session.players.get(&id).cloned()
            };
            DropEntry { kind: DropKind::Collected, player, item: o.item as u32, count: o.count as u32, world, at: now() }
        }
        _ => return,
    };
    let mine = entry.player.is_some() && entry.player == ic.session.name;
    if ic.history.track && !mine {
        let line = describe(ic, &entry);
        ic.emit(Effect::console(format!("`9[track]`` {line}")));
    }
    push(&mut ic.history.drops, entry);
}

fn describe(ic: &Interceptor, e: &DropEntry) -> String {
    let who = e.player.as_deref().map_or("`sSomeone``".to_string(), |p| format!("`w{p}``"));
    let verb = match e.kind {
        DropKind::Dropped => "`4dropped``",
        DropKind::Collected => "`2took``",
    };
    format!("{who} {verb} `w{} {}``", e.count, ic.items.name(e.item))
}

/// `/drophistory`: drops and pickups since the proxy started.
pub fn drophistory(ic: &mut Interceptor, _: &[&str]) {
    let mut dialog = Dialog::new("drophistory")
        .title("Drop history", HISTORY_ICON)
        .small("Newest first. A drop is put on the player standing closest to it.");
    if ic.history.drops.is_empty() {
        dialog = dialog.text("Nothing dropped or picked up yet.");
    }
    let mut last_world = String::new();
    for e in ic.history.drops.iter().take(SHOWN) {
        if e.world != last_world {
            dialog = dialog.small(&format!("`9{}``", e.world));
            last_world = e.world.clone();
        }
        dialog = dialog.item(e.item, &format!("{}  `s{} ago``", describe(ic, e), ago(e.at)));
    }
    dialog = dialog
        .spacer()
        .button("track", if ic.history.track { "Stop live tracking" } else { "Live tracking in chat" })
        .button("clear_drops", "`4Clear``");
    super::tools::show(ic, dialog, "Close", "");
}

/// `/track`: other players' drops and pickups in chat.
pub fn track(ic: &mut Interceptor, _: &[&str]) {
    ic.history.track = !ic.history.track;
    let state = if ic.history.track { "`2on``" } else { "`4off``" };
    say(ic, format!("Drop tracking {state}"));
}

pub fn on_dialog(ic: &mut Interceptor, ret: &DialogReturn) -> bool {
    match (ret.name, ret.button) {
        ("roulette_history", Some("clear_spins")) => {
            ic.history.spins.clear();
            roulette(ic, &[]);
        }
        ("drophistory", Some("clear_drops")) => {
            ic.history.drops.clear();
            drophistory(ic, &[]);
        }
        ("drophistory", Some("track")) => {
            track(ic, &[]);
            drophistory(ic, &[]);
        }
        ("roulette_history" | "drophistory", _) => {}
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::protocol::{GamePacketType, Message};
    use crate::proxy::Direction;
    use crate::proxy::world::{Tile, WorldMap};
    use std::sync::Arc;

    fn ic() -> Interceptor {
        let mut cfg = Config::default();
        cfg.log.text = false;
        cfg.log.variant = false;
        let mut ic = Interceptor::new(Arc::new(cfg));
        ic.world = Some(WorldMap {
            name: "T".into(),
            width: 10,
            height: 10,
            tiles: vec![Tile::default(); 100],
            last_object_uid: Some(0),
            objects: Vec::new(),
        });
        ic.session.net_id = Some(1);
        ic.session.name = Some("Me".into());
        ic.session.pos = Some((0.0, 0.0));
        ic.session.players.insert(5, "Bob".into());
        ic.session.player_pos.insert(5, (256.0, 256.0)); // tile 8,8
        ic
    }

    fn object(ic: &mut Interceptor, net_id: i32, value: u32, x: f32, y: f32) {
        let pkt = GameUpdatePacket {
            packet_type: GamePacketType::ItemChangeObject,
            net_id,
            value,
            float_var: 4.0,
            vec_x: x,
            vec_y: y,
            ..Default::default()
        };
        ic.handle(Direction::ToClient, &Message::Game(pkt).encode());
    }

    #[test]
    fn drops_go_to_the_nearest_player_and_pickups_name_the_taker() {
        let mut ic = ic();
        object(&mut ic, -1, 242, 262.0, 268.0); // next to Bob, uid 1
        object(&mut ic, -1, 2, 6.0, 8.0); // at my feet, uid 2
        object(&mut ic, 5, 2, 0.0, 0.0); // Bob takes the Dirt
        let d: Vec<(DropKind, Option<String>, u32)> =
            ic.history.drops.iter().map(|e| (e.kind.clone(), e.player.clone(), e.item)).collect();
        assert_eq!(
            d,
            vec![
                (DropKind::Collected, Some("Bob".into()), 2),
                (DropKind::Dropped, Some("Me".into()), 2),
                (DropKind::Dropped, Some("Bob".into()), 242),
            ]
        );
    }

    #[test]
    fn track_writes_others_to_chat() {
        let mut ic = ic();
        ic.history.track = true;
        ic.take_effects();
        object(&mut ic, -1, 242, 262.0, 268.0);
        assert_eq!(ic.take_effects().len(), 1);
        object(&mut ic, -1, 2, 6.0, 8.0); // mine: not shown
        assert!(ic.take_effects().is_empty());
    }
}
