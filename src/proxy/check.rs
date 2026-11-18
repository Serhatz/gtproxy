//! `/check`: the account at a glance (level, gems, untradeable items, active
//! mods). `/chest`: what the storage boxes in this world hold.
//!
//! Storage boxes (tile extra 54, e.g. Storage Box Xtreme) carry their items
//! in the world data every player receives: `u16` length, then 13 bytes per
//! item (3 unknown, `u32` item, 2 unknown, `u32` count).

use std::time::Instant;

use super::commands::warn;
use super::dialog::Dialog;
use super::interceptor::Interceptor;
use super::tools::show;
use super::world::Special;

const CHECK_ICON: u32 = 6336; // Growpedia
const CHEST_ICON: u32 = 6286; // Storage Box Xtreme - Level 1

fn left(until: Option<Instant>) -> String {
    let Some(t) = until else { return "no time given".into() };
    let secs = t.saturating_duration_since(Instant::now()).as_secs();
    match secs {
        0 => "ending".into(),
        s if s < 60 => format!("{s} s left"),
        s if s < 3600 => format!("{} min left", s / 60),
        s => format!("{} h {} min left", s / 3600, s / 60 % 60),
    }
}

/// `/check`.
pub fn check(ic: &mut Interceptor, _: &[&str]) {
    let s = &ic.session;
    let opt = |v: Option<String>| v.unwrap_or_else(|| "-".into());
    let mut d = Dialog::new("check")
        .title("Account Check", CHECK_ICON)
        .field("Name", &opt(s.name.clone()))
        .field("Level", &opt(s.level.map(|l| l.to_string())))
        .field("Gems", &opt(s.gems.map(|g| g.to_string())))
        .field("World", &opt(s.world.clone()))
        .field("Inventory", &format!("{} items, {} slots", ic.inventory.len(), ic.inventory.slots));
    let mods: Vec<(String, Option<Instant>)> = s.active_mods().map(|(n, t)| (n.to_string(), t)).collect();
    d = d.spacer().text(&format!("`wActive mods`` ({})", mods.len()));
    if mods.is_empty() {
        d = d.small("None seen since the proxy started.");
    }
    for (name, until) in mods {
        d = d.small(&format!("{name} - {}", left(until)));
    }
    let mut untradeable: Vec<(u32, u32)> = ic.inventory.iter().filter(|(id, _)| ic.items.untradeable(*id)).collect();
    untradeable.sort();
    d = d.spacer().text(&format!("`wUntradeable items`` ({})", untradeable.len()));
    if ic.inventory.slots == 0 {
        d = d.small("Inventory not known yet: open it once or re-enter the world.");
    }
    for (id, n) in untradeable {
        d = d.item(id, &format!("{} x{n}", ic.items.name(id)));
    }
    show(ic, d, "Close", "");
}

/// `/chest`.
pub fn chest(ic: &mut Interceptor, _: &[&str]) {
    let Some(map) = ic.world.as_ref() else { return warn(ic, "No world map yet: enter a world first") };
    let boxes: Vec<((i32, i32), u16, Vec<(u32, u32)>)> = map
        .tiles
        .iter()
        .enumerate()
        .filter_map(|(i, t)| match &t.special {
            Special::Storage(items) => Some((map.xy(i), t.fg, items.clone())),
            _ => None,
        })
        .collect();
    let full = boxes.iter().filter(|b| !b.2.is_empty()).count();
    let mut d = Dialog::new("chest")
        .title("Storage Boxes", CHEST_ICON)
        .small(&format!("{} storage boxes in this world, {full} with items.", boxes.len()));
    for ((x, y), fg, items) in boxes.iter().filter(|b| !b.2.is_empty()).take(30) {
        d = d.spacer().item(*fg as u32, &format!("`w{}`` at {x}, {y}", ic.items.name(*fg as u32)));
        for (id, n) in items.iter().take(40) {
            d = d.item(*id, &format!("{} x{n}", ic.items.name(*id)));
        }
    }
    if full > 30 {
        d = d.small(&format!("... and {} more boxes", full - 30));
    }
    show(ic, d, "Close", "");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::proxy::effect::Effect;
    use crate::proxy::world::{Tile, WorldMap};
    use std::sync::Arc;

    fn shown(ic: &mut Interceptor) -> String {
        ic.take_effects()
            .into_iter()
            .filter_map(|e| match e {
                Effect::ToClient(b) => Some(String::from_utf8_lossy(&b).into_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn chest_and_check_windows() {
        let mut ic = Interceptor::new(Arc::new(Config::default()));
        let mut tiles = vec![Tile::default(); 4];
        tiles[1] = Tile { fg: 6286, special: Special::Storage(vec![(242, 7)]), ..Default::default() };
        tiles[2] = Tile { fg: 6286, special: Special::Storage(Vec::new()), ..Default::default() };
        ic.world = Some(WorldMap { name: "T".into(), width: 2, height: 2, tiles, last_object_uid: None, objects: Vec::new() });
        chest(&mut ic, &[]);
        let d = shown(&mut ic);
        assert!(d.contains("2 storage boxes in this world, 1 with items"));
        assert!(d.contains("at 1, 0") && d.contains("x7"));
        ic.session.level = Some(6);
        ic.session.note_mod("Steady hands of a surgeon. (`$Calm nerves.`` mod added, `$2 mins`` left)");
        check(&mut ic, &[]);
        let d = shown(&mut ic);
        assert!(d.contains("Calm nerves.") && d.contains("min left"));
    }
}
