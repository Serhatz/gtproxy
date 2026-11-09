//! Going somewhere inside the world, on the teleport path walker:
//! `/pvend`, `/pfind`, `/tpto`, and `/door` (re-joining the
//! world at a door ID).

use super::commands::{say, warn};
use super::effect::Effect;
use super::interceptor::Interceptor;
use super::teleport::{find_path, tile_of, walk, walk_start};
use super::world::{Special, WorldObject};

fn usage(ic: &mut Interceptor, rest: &str) {
    warn(ic, format!("Usage: {}{rest}", ic.cfg.command.prefix));
}

/// Tile an object lies on (its position is the top-left of a small sprite).
fn object_tile(o: &WorldObject) -> (i32, i32) {
    (((o.x + 8.0) / 32.0).floor() as i32, ((o.y + 8.0) / 32.0).floor() as i32)
}

/// The tile itself if we can stand there, else a reachable neighbour.
fn stand_at(ic: &Interceptor, start: (i32, i32), target: (i32, i32)) -> Option<Vec<(i32, i32)>> {
    let map = ic.world.as_ref()?;
    let user = ic.session.user_id.unwrap_or(0) as u32;
    [(0, 0), (0, -1), (-1, 0), (1, 0), (0, 1)]
        .into_iter()
        .filter_map(|(dx, dy)| find_path(map, &ic.items, user, start, (target.0 + dx, target.1 + dy)))
        .min_by_key(Vec::len)
}

fn go_along(ic: &mut Interceptor, start: (i32, i32), net_id: i32, path: Vec<(i32, i32)>, done: String) {
    let total = walk(ic, start, &path, net_id);
    ic.emit(Effect::console(format!("`9[proxy]`` {done}")).later(total));
}

// ── /pvend ───────────────────────────────────────────────────────────────────

/// `/pvend <item>`: to the cheapest vending machine in this world selling it.
pub fn pvend(ic: &mut Interceptor, args: &[&str]) {
    let q = args.join(" ").to_lowercase();
    if q.is_empty() {
        return usage(ic, "pvend <item name>");
    }
    let Some(map) = ic.world.as_ref() else { return warn(ic, "Enter a world first") };
    let mut vends: Vec<((i32, i32), u32, i32)> = map
        .tiles
        .iter()
        .enumerate()
        .filter_map(|(i, t)| match t.special {
            Special::Vend { item, price } if item != 0 && price != 0 => Some((map.xy(i), item, price)),
            _ => None,
        })
        .filter(|(_, item, _)| ic.items.name(*item).to_lowercase().contains(&q))
        .collect();
    // WL per item: 5 WL each is dearer than 20 per WL.
    let cost = |p: i32| if p > 0 { p as f64 } else { 1.0 / -(p as f64) };
    vends.sort_by(|a, b| cost(a.2).partial_cmp(&cost(b.2)).unwrap_or(std::cmp::Ordering::Equal));
    if vends.is_empty() {
        let known = !ic.vends.search(&q, |id| ic.items.name(id)).is_empty();
        let hint = if known { format!(", try {}vendfind {q}", ic.cfg.command.prefix) } else { String::new() };
        return warn(ic, format!("No vend sells \"{q}\" in this world{hint}"));
    }
    let Some((start, net_id)) = walk_start(ic) else { return };
    // Cheapest one we can get to.
    for (at, item, price) in vends {
        if let Some(path) = stand_at(ic, start, at) {
            let price = if price > 0 { format!("{price} WL each") } else { format!("{} per WL", -price) };
            let done = format!("At the cheapest `w{}`` vend: `2{price}`` `s({}, {})``", ic.items.name(item), at.0, at.1);
            if path.is_empty() {
                say(ic, done);
            } else {
                go_along(ic, start, net_id, path, done);
            }
            return;
        }
    }
    warn(ic, "Can't reach any vend that sells it");
}

// ── /pfind ───────────────────────────────────────────────────────────────────

/// `/pfind <item>`: to the closest floating item with that name.
pub fn pfind(ic: &mut Interceptor, args: &[&str]) {
    let q = args.join(" ").to_lowercase();
    if q.is_empty() {
        return usage(ic, "pfind <item name>");
    }
    let Some(map) = ic.world.as_ref() else { return warn(ic, "Enter a world first") };
    let objects: Vec<WorldObject> =
        map.objects.iter().filter(|o| ic.items.name(o.item as u32).to_lowercase().contains(&q)).cloned().collect();
    if objects.is_empty() {
        return warn(ic, format!("No floating \"{q}\" in this world"));
    }
    let Some((start, net_id)) = walk_start(ic) else { return };
    let best = objects
        .iter()
        .filter_map(|o| stand_at(ic, start, object_tile(o)).map(|p| (o, p)))
        .min_by_key(|(_, p)| p.len());
    let Some((o, path)) = best else { return warn(ic, "Can't reach any of them") };
    let (x, y) = object_tile(o);
    let done = format!("At `w{} {}`` `s({x}, {y})``", o.count, ic.items.name(o.item as u32));
    if path.is_empty() {
        say(ic, done);
    } else {
        go_along(ic, start, net_id, path, done);
    }
}

// ── /tpto ────────────────────────────────────────────────────────────────────

/// `/tpto <name>`: next to a player in this world.
pub fn tpto(ic: &mut Interceptor, args: &[&str]) {
    let Some(q) = args.first() else { return usage(ic, "tpto <player name>") };
    let Some((id, name)) = ic.session.find_player(q).map(|(id, n)| (id, n.to_string())) else {
        return warn(ic, format!("No player called \"{q}\" here"));
    };
    let Some(pos) = ic.session.player_pos.get(&id).copied() else {
        return warn(ic, format!("Don't know where {name} is yet, wait for them to move"));
    };
    let Some((start, net_id)) = walk_start(ic) else { return };
    match stand_at(ic, start, tile_of(pos)) {
        Some(path) if path.is_empty() => say(ic, format!("You are already next to `w{name}``")),
        Some(path) => go_along(ic, start, net_id, path, format!("Next to `w{name}``")),
        None => warn(ic, format!("Can't reach {name}")),
    }
}

// ── /door ────────────────────────────────────────────────────────────────────

/// `/door <id>`: re-enters this world at the door with that ID.
pub fn door(ic: &mut Interceptor, args: &[&str]) {
    let Some(id) = args.first() else { return usage(ic, "door <door ID>") };
    let Some(world) = ic.session.world.clone() else { return warn(ic, "Enter a world first") };
    if id.len() > 24 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return warn(ic, "Invalid door ID");
    }
    let id = id.to_ascii_uppercase();
    say(ic, format!("Going to door `w{id}`` in `w{world}``"));
    super::warp::go_to(ic, &format!("{world}|{id}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::items::ItemNames;
    use crate::proxy::world::{Tile, WorldMap};
    use std::sync::Arc;

    const DIRT: u16 = 2;

    fn ic() -> Interceptor {
        let mut cfg = Config::default();
        cfg.log.text = false;
        cfg.log.variant = false;
        let items = ItemNames::with_collision(&[(DIRT as u32, 1)]);
        let mut ic = Interceptor::with_items(Arc::new(cfg), Arc::new(items));
        // 8x3: air on top two rows, dirt floor.
        let mut tiles = vec![Tile::default(); 16];
        tiles.extend((0..8).map(|_| Tile { fg: DIRT, ..Default::default() }));
        ic.world = Some(WorldMap { name: "T".into(), width: 8, height: 3, tiles, last_object_uid: Some(0), objects: Vec::new() });
        ic.session.enter_world("T".into());
        ic.session.net_id = Some(1);
        ic.session.pos = Some((0.0, 32.0)); // tile 0,1
        ic
    }

    #[test]
    fn door_rejoins_with_the_id() {
        let mut ic = ic();
        door(&mut ic, &["abc"]);
        let sent: Vec<String> = ic
            .take_effects()
            .iter()
            .filter_map(|e| match e {
                Effect::ToServer(b) => Some(String::from_utf8_lossy(&b[4..]).to_string()),
                _ => None,
            })
            .collect();
        assert!(sent[0].starts_with("action|quit_to_exit"));
        assert_eq!(ic.warp.pending(), Some("T|ABC"));
    }
}
