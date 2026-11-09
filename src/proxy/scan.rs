//! `/scan`: what a Growscan 9000 shows, from the world map the proxy already has.
//!
//! - Blocks / backgrounds: every tile's foreground and background, counted.
//! - Floating items: the objects lying in the world, kept up to date from
//!   `ItemChangeObject` (new drop, stack size change, picked up).
//!
//! Nothing is sent to the server; the dialog is ours.

use std::collections::HashMap;

use crate::protocol::GameUpdatePacket;

use super::commands::warn;
use super::dialog::{Dialog, DialogReturn};
use super::interceptor::Interceptor;

/// `ItemChangeObject.net_id` values.
const NEW_OBJECT: i32 = -1;
const COUNT_CHANGED: i32 = -3;
/// Rows shown per list; the dialog gets slow beyond this.
const MAX_ROWS: usize = 150;
const FALLBACK_ICON: u32 = 6016;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Blocks,
    Backgrounds,
    Floating,
}

/// Server -> client `ItemChangeObject`, before anything else touches it.
pub fn track_object(ic: &mut Interceptor, pkt: &GameUpdatePacket) {
    let Some(map) = ic.world.as_mut() else { return };
    let Ok(item) = u16::try_from(pkt.value) else { return };
    let count = pkt.float_var.clamp(0.0, 255.0) as u8;
    match pkt.net_id {
        NEW_OBJECT => map.add_object(item, pkt.vec_x, pkt.vec_y, count),
        COUNT_CHANGED => map.set_object_count(item, pkt.vec_x, pkt.vec_y, count),
        id if id > 0 => map.remove_object(pkt.value),
        _ => {}
    }
}

/// `(item, count)` for one tab, most first.
pub fn counts(ic: &Interceptor, tab: Tab) -> Vec<(u32, u32)> {
    let Some(map) = ic.world.as_ref() else { return Vec::new() };
    let mut by_item: HashMap<u32, u32> = HashMap::new();
    match tab {
        Tab::Blocks => map.tiles.iter().filter(|t| t.fg != 0).for_each(|t| *by_item.entry(t.fg as u32).or_default() += 1),
        Tab::Backgrounds => {
            map.tiles.iter().filter(|t| t.bg != 0).for_each(|t| *by_item.entry(t.bg as u32).or_default() += 1)
        }
        Tab::Floating => map
            .objects
            .iter()
            .filter(|o| o.item != 0)
            .for_each(|o| *by_item.entry(o.item as u32).or_default() += o.count as u32),
    }
    let mut list: Vec<(u32, u32)> = by_item.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    list
}

fn show_tab(ic: &mut Interceptor, tab: Tab, filter: Option<&str>) {
    let Some(map) = ic.world.as_ref() else {
        warn(ic, "Enter a world first, there is nothing to scan here");
        return;
    };
    let (name, w, h) = (map.name.clone(), map.width, map.height);
    let floating: u32 = map.objects.iter().map(|o| o.count as u32).sum();
    let icon = ic.items.find("Growscan 9000").unwrap_or(FALLBACK_ICON);
    let filter = filter.map(str::to_lowercase);
    let list: Vec<(u32, u32, String)> = counts(ic, tab)
        .into_iter()
        .map(|(id, n)| (id, n, ic.items.name(id)))
        .filter(|(_, _, item)| filter.as_ref().is_none_or(|f| item.to_lowercase().contains(f)))
        .collect();

    let heading = match tab {
        Tab::Blocks => "World Blocks",
        Tab::Backgrounds => "Backgrounds",
        Tab::Floating => "Floating Items",
    };
    // Laid out like the game's own item lists (Extract-O-Snap): icon, "Name : count".
    let mut dialog = Dialog::new("scan")
        .title("Growscan 9000", icon)
        .text(&format!("`w{name}`` ({w}x{h}), {floating} floating items."))
        .spacer()
        .button("scan_blocks", "World Blocks")
        .button("scan_bg", "Backgrounds")
        .button("scan_float", "Floating Items")
        .spacer()
        .text(&format!("`w{heading}:``"));
    if let Some(f) = &filter {
        dialog = dialog.small(&format!("Only items with \"{f}\" in the name"));
    }
    if list.is_empty() {
        dialog = dialog.text("Nothing found.");
    }
    for (id, n, item) in list.iter().take(MAX_ROWS) {
        dialog = dialog.item(*id, &format!("{item} : {n}"));
    }
    if list.len() > MAX_ROWS {
        dialog = dialog.small(&format!("... and {} more", list.len() - MAX_ROWS));
    }
    super::tools::show(ic, dialog, "Close", "");
}

/// `/scan [blocks|bg|float] [name]`.
pub fn scan(ic: &mut Interceptor, args: &[&str]) {
    let (tab, rest) = match args.first().map(|a| a.to_lowercase()).as_deref() {
        Some("blocks" | "block" | "fg") => (Tab::Blocks, &args[1..]),
        Some("bg" | "background" | "backgrounds") => (Tab::Backgrounds, &args[1..]),
        Some("float" | "floating" | "drops") => (Tab::Floating, &args[1..]),
        _ => (Tab::Blocks, args),
    };
    let filter = rest.join(" ");
    show_tab(ic, tab, (!filter.is_empty()).then_some(filter.as_str()));
}

/// `/blocks [name]`.
pub fn blocks(ic: &mut Interceptor, args: &[&str]) {
    let filter = args.join(" ");
    show_tab(ic, Tab::Blocks, (!filter.is_empty()).then_some(filter.as_str()));
}

/// `/items [name]`: floating items.
pub fn items(ic: &mut Interceptor, args: &[&str]) {
    let filter = args.join(" ");
    show_tab(ic, Tab::Floating, (!filter.is_empty()).then_some(filter.as_str()));
}

pub fn on_dialog(ic: &mut Interceptor, ret: &DialogReturn) -> bool {
    if ret.name != "scan" {
        return false;
    }
    let tab = match ret.button {
        Some("scan_blocks") => Tab::Blocks,
        Some("scan_bg") => Tab::Backgrounds,
        Some("scan_float") => Tab::Floating,
        _ => return true,
    };
    show_tab(ic, tab, None);
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
        let tiles = vec![
            Tile { fg: 2, bg: 14, ..Default::default() },
            Tile { fg: 2, ..Default::default() },
            Tile { fg: 242, bg: 14, ..Default::default() },
            Tile::default(),
        ];
        ic.world = Some(WorldMap { name: "T".into(), width: 2, height: 2, tiles, last_object_uid: Some(9), objects: Vec::new() });
        ic
    }

    fn object(ic: &mut Interceptor, net_id: i32, value: u32, count: f32) {
        let pkt = GameUpdatePacket {
            packet_type: GamePacketType::ItemChangeObject,
            net_id,
            value,
            float_var: count,
            vec_x: 32.0,
            vec_y: 64.0,
            ..Default::default()
        };
        ic.handle(Direction::ToClient, &Message::Game(pkt).encode());
    }

    #[test]
    fn counts_blocks_and_backgrounds() {
        let ic = ic();
        assert_eq!(counts(&ic, Tab::Blocks), vec![(2, 2), (242, 1)]);
        assert_eq!(counts(&ic, Tab::Backgrounds), vec![(14, 2)]);
    }

    #[test]
    fn follows_floating_items() {
        let mut ic = ic();
        object(&mut ic, -1, 1796, 3.0); // uid 10
        object(&mut ic, -1, 242, 50.0); // uid 11
        assert_eq!(counts(&ic, Tab::Floating), vec![(242, 50), (1796, 3)]);
        object(&mut ic, -3, 242, 20.0); // someone took 30
        assert_eq!(counts(&ic, Tab::Floating), vec![(242, 20), (1796, 3)]);
        object(&mut ic, 5, 10, 0.0); // the Diamond Locks were picked up
        assert_eq!(counts(&ic, Tab::Floating), vec![(242, 20)]);
    }
}
