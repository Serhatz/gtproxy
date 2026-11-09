//! Ctrl+click teleport.
//!
//! - The server tells the client its build/punch range in `SetCharacterState`
//!   (`jump_count` / `animation_type`, each `126 + range`). While Ctrl is
//!   held we show the client a bigger range, so every tile on screen becomes
//!   clickable; when Ctrl is released the real range is restored.
//! - A click sends `TileChangeRequest` (on a block) and/or a `State` with the
//!   `PUNCH`/`PLACE` flag carrying the tile in `int_x/int_y`.
//! - Ctrl held: the click is swallowed and we walk there along a path of
//!   passable tiles, one `State` packet per tile, then move the client with
//!   `OnSetPos`. Ctrl not held: clicks beyond the real range are dropped so
//!   the server never sees an impossible reach.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::items::ItemNames;
use crate::protocol::game_packet::PacketFlags;
use crate::protocol::{GamePacketType, GameUpdatePacket, Message, Variant};

use super::commands::{say, warn};
use super::effect::Effect;
use super::interceptor::{Interceptor, Verdict};
use super::world::{FLIPPED_X, IS_ON, Special, WorldMap};

const RANGE_BASE: u8 = 126;
/// Range shown to the client while teleport mode is on.
const EXTENDED_RANGE: u8 = 60;
/// The two packets of one click arrive within a few ms; ignore the second.
const CLICK_DEDUP: Duration = Duration::from_millis(300);
const DEFAULT_STEP: Duration = Duration::from_millis(40);
/// Paths longer than this are refused (huge worlds, very long detours).
const MAX_PATH: usize = 600;

pub struct Teleport {
    pub enabled: bool,
    pub step: Duration,
    /// Ranges the server really allows, from the last `SetCharacterState`.
    pub build_range: Option<u8>,
    pub punch_range: Option<u8>,
    /// Last `SetCharacterState` for us, as received, to re-send when toggling.
    last_state: Option<GameUpdatePacket>,
    /// While walking a path the client's own movement must not reach the server.
    pub busy_until: Option<Instant>,
    last_click: Option<(i32, i32, Instant)>,
    /// Whether the client currently has the enlarged range.
    shown_extended: bool,
    /// Reads the Ctrl key; replaceable in tests.
    pub ctrl: fn() -> bool,
}

impl Default for Teleport {
    fn default() -> Self {
        Self {
            enabled: false,
            step: DEFAULT_STEP,
            build_range: None,
            punch_range: None,
            last_state: None,
            busy_until: None,
            last_click: None,
            shown_extended: false,
            ctrl: ctrl_down,
        }
    }
}

#[cfg(windows)]
fn ctrl_down() -> bool {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetAsyncKeyState(key: i32) -> i16;
    }
    const VK_CONTROL: i32 = 0x11;
    // SAFETY: plain Win32 call without pointers.
    unsafe { GetAsyncKeyState(VK_CONTROL) as u16 & 0x8000 != 0 }
}

#[cfg(not(windows))]
fn ctrl_down() -> bool {
    false
}

// ── Passability and paths ────────────────────────────────────────────────────

/// Can the player move into (x, y) going in direction `dir`?
pub fn passable(map: &WorldMap, items: &ItemNames, user_id: u32, x: i32, y: i32, dir: (i32, i32)) -> bool {
    let Some(tile) = map.tile(x, y) else { return false };
    if tile.fg == 0 {
        return true;
    }
    match &tile.special {
        Special::VipEntrance(a) | Special::FriendsEntrance(a) => return a.allows(user_id),
        Special::StormyCloud { solid } => return !solid,
        _ => {}
    }
    match items.collision(tile.fg as u32) {
        Some(0) => true,
        Some(2) => dir != (0, 1), // platform: can't drop through
        Some(3 | 10 | 11) => map.has_access(x, y, user_id),
        Some(4) => tile.flags & IS_ON != 0,
        Some(5) => {
            if tile.flags & FLIPPED_X != 0 {
                dir != (1, 0)
            } else {
                dir != (-1, 0)
            }
        }
        Some(7) => dir != (0, -1),
        Some(9) => tile.flags & IS_ON == 0,
        // Solid, unknown items, gates we cannot judge.
        _ => false,
    }
}

/// Shortest 4-way path from `from` to `to`, excluding `from`. Gravity is ignored.
pub fn find_path(
    map: &WorldMap,
    items: &ItemNames,
    user_id: u32,
    from: (i32, i32),
    to: (i32, i32),
) -> Option<Vec<(i32, i32)>> {
    if from == to {
        return Some(Vec::new());
    }
    if !passable(map, items, user_id, to.0, to.1, (0, 0)) {
        return None;
    }
    let mut came: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
    let mut queue = VecDeque::from([from]);
    came.insert(from, from);
    while let Some(cur) = queue.pop_front() {
        if cur == to {
            let mut path = vec![cur];
            let mut at = cur;
            while came[&at] != from {
                at = came[&at];
                path.push(at);
            }
            path.reverse();
            return (path.len() <= MAX_PATH).then_some(path);
        }
        for dir in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let next = (cur.0 + dir.0, cur.1 + dir.1);
            if !came.contains_key(&next) && passable(map, items, user_id, next.0, next.1, dir) {
                came.insert(next, cur);
                queue.push_back(next);
            }
        }
    }
    None
}

/// Tile under a pixel position from `State` / `OnSetPos`.
pub fn tile_of(pos: (f32, f32)) -> (i32, i32) {
    (((pos.0 + 10.0) / 32.0).floor() as i32, ((pos.1 + 15.0) / 32.0).floor() as i32)
}

// ── Packet hooks ─────────────────────────────────────────────────────────────

/// Our own `SetCharacterState`: remember the real range, show the client a bigger one.
pub fn on_character_state(ic: &mut Interceptor, pkt: GameUpdatePacket) -> Verdict {
    // On world entry this can come before our OnSpawn, when our netID is not known yet.
    if ic.session.net_id.is_some_and(|me| me != pkt.net_id) {
        return Verdict::Forward;
    }
    ic.teleport.build_range = Some(pkt.jump_count.saturating_sub(RANGE_BASE));
    ic.teleport.punch_range = Some(pkt.animation_type.saturating_sub(RANGE_BASE));
    tracing::debug!(
        "range: build {:?}, punch {:?}",
        ic.teleport.build_range,
        ic.teleport.punch_range
    );
    ic.teleport.last_state = Some(pkt.clone());
    let extend = ic.teleport.enabled && (ic.teleport.ctrl)();
    ic.teleport.shown_extended = extend;
    if !extend {
        return Verdict::Forward;
    }
    Verdict::Replace(Message::Game(extended(pkt)).encode())
}

/// Called every bridge loop: enlarge the range while Ctrl is held, restore it after.
pub fn tick(ic: &mut Interceptor) {
    let want = ic.teleport.enabled && (ic.teleport.ctrl)();
    if want != ic.teleport.shown_extended {
        show_range(ic, want);
    }
}

/// Sends the client our last `SetCharacterState`, enlarged or as received.
fn show_range(ic: &mut Interceptor, extend: bool) {
    let Some(state) = ic.teleport.last_state.clone() else { return };
    ic.teleport.shown_extended = extend;
    let pkt = if extend { extended(state) } else { state };
    ic.emit(Effect::ToClient(Message::Game(pkt).encode()));
}

fn extended(mut pkt: GameUpdatePacket) -> GameUpdatePacket {
    pkt.jump_count = RANGE_BASE + EXTENDED_RANGE;
    pkt.animation_type = RANGE_BASE + EXTENDED_RANGE;
    pkt
}

/// A click from the client (`TileChangeRequest`, or `State` with PUNCH/PLACE).
pub fn on_click(ic: &mut Interceptor, x: i32, y: i32, place: bool) -> Verdict {
    if !ic.teleport.enabled {
        return Verdict::Forward;
    }
    let now = Instant::now();
    let repeat = ic.teleport.last_click.is_some_and(|(cx, cy, t)| (cx, cy) == (x, y) && now < t + CLICK_DEDUP);
    if (ic.teleport.ctrl)() {
        if !repeat {
            ic.teleport.last_click = Some((x, y, now));
            teleport_to(ic, (x, y));
        }
        return Verdict::Drop;
    }
    // Normal click: only what the server would accept anyway.
    let range = if place { ic.teleport.build_range } else { ic.teleport.punch_range };
    let (Some(range), Some(pos)) = (range, ic.session.pos) else { return Verdict::Forward };
    let (px, py) = tile_of(pos);
    let reach = (x - px).abs().max((y - py).abs());
    if reach > range as i32 + 1 {
        tracing::debug!("dropped out-of-range click at ({x},{y}), reach {reach} > {range}");
        return Verdict::Drop;
    }
    Verdict::Forward
}

/// The client's own movement while we are walking a path would undo it.
pub fn hold_client_movement(ic: &mut Interceptor) -> bool {
    match ic.teleport.busy_until {
        Some(t) if Instant::now() < t => true,
        Some(_) => {
            ic.teleport.busy_until = None;
            false
        }
        None => false,
    }
}

pub fn teleport_to(ic: &mut Interceptor, target: (i32, i32)) {
    let Some((start, net_id)) = walk_start(ic) else { return };
    let user_id = ic.session.user_id.unwrap_or(0) as u32;
    let Some(map) = ic.world.as_ref() else { return };
    let Some(path) = find_path(map, &ic.items, user_id, start, target) else {
        warn(ic, format!("can't reach ({}, {})", target.0, target.1));
        return;
    };
    if path.is_empty() {
        return;
    }
    let total = walk(ic, start, &path, net_id);
    ic.emit(
        Effect::console(format!("`9[proxy]`` Teleported to `w({}, {})`` ({} tiles)", target.0, target.1, path.len()))
            .later(total),
    );
}

/// Where a walk starts and our netID; warns and returns `None` when unknown.
pub fn walk_start(ic: &mut Interceptor) -> Option<((i32, i32), i32)> {
    if ic.world.is_none() {
        warn(ic, "world map not available here, cannot teleport");
        return None;
    }
    let (Some(pos), Some(net_id)) = (ic.session.pos, ic.session.net_id) else {
        warn(ic, "move a little first so the proxy knows where you are");
        return None;
    };
    Some((tile_of(pos), net_id))
}

/// Sends one `State` per tile of `path` (starting next to `start`), then
/// moves the client to the end. Returns when the last step is sent.
pub fn walk(ic: &mut Interceptor, start: (i32, i32), path: &[(i32, i32)], net_id: i32) -> Duration {
    let Some(&end_tile) = path.last() else { return Duration::ZERO };
    let step = ic.teleport.step;
    let mut prev = start;
    for (i, &(x, y)) in path.iter().enumerate() {
        let mut flags = PacketFlags::WALK | PacketFlags::STANDING;
        flags.set(PacketFlags::FACING_LEFT, x < prev.0);
        let pkt = GameUpdatePacket {
            packet_type: GamePacketType::State,
            vec_x: (x * 32) as f32,
            vec_y: (y * 32) as f32,
            int_x: -1,
            int_y: -1,
            flags,
            ..Default::default()
        };
        ic.emit(Effect::ToServer(Message::Game(pkt).encode()).later(step * i as u32));
        prev = (x, y);
    }
    let end = ((end_tile.0 * 32) as f32, (end_tile.1 * 32) as f32);
    let total = step * path.len() as u32;
    ic.emit(
        Effect::client_call(net_id, vec![Variant::String("OnSetPos".into()), Variant::Vec2(end.0, end.1)]).later(total),
    );
    ic.session.pos = Some(end);
    ic.teleport.busy_until = Some(Instant::now() + total + Duration::from_millis(250));
    tracing::info!("walk {start:?} -> {end_tile:?}: {} steps", path.len());
    total
}

// ── Command ──────────────────────────────────────────────────────────────────

/// `/tp` toggles Ctrl+click, `/tp <x> <y>` goes there, `/tp speed <ms>`.
pub fn tp(ic: &mut Interceptor, args: &[&str]) {
    match args {
        [] => {
            ic.teleport.enabled = !ic.teleport.enabled;
            if ic.teleport.shown_extended {
                show_range(ic, false);
            }
            if ic.teleport.enabled {
                say(ic, "teleport `2on``: hold `wCtrl`` and click anywhere you can reach");
            } else {
                say(ic, "teleport `4off``");
            }
        }
        ["speed", ms] => match ms.parse::<u64>() {
            Ok(ms) if (10..=500).contains(&ms) => {
                ic.teleport.step = Duration::from_millis(ms);
                say(ic, format!("teleport step `w{ms}ms`` per tile"));
            }
            _ => warn(ic, "speed must be 10-500 ms per tile"),
        },
        [x, y] => match (x.parse(), y.parse()) {
            (Ok(x), Ok(y)) => teleport_to(ic, (x, y)),
            _ => warn(ic, format!("usage: {}tp <x> <y>", ic.cfg.command.prefix)),
        },
        _ => warn(ic, format!("usage: {}tp, {}tp <x> <y>, {}tp speed <ms>", ic.cfg.command.prefix, ic.cfg.command.prefix, ic.cfg.command.prefix)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_from_pixels() {
        // From the click capture: standing at (1093,738) next to tile 34,23.
        assert_eq!(tile_of((1093.0, 738.0)), (34, 23));
        assert_eq!(tile_of((64.0, 64.0)), (2, 2));
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use crate::config::Config;
    use crate::proxy::Direction;
    use crate::proxy::world::Tile;
    use std::sync::Arc;

    const DIRT: u16 = 2;
    const PLATFORM: u16 = 102;

    /// 6x4 map, `#` dirt, `=` platform, `.` air.
    fn map(rows: &[&str]) -> WorldMap {
        let tiles = rows
            .iter()
            .flat_map(|r| r.chars())
            .map(|c| Tile {
                fg: match c {
                    '#' => DIRT,
                    '=' => PLATFORM,
                    _ => 0,
                },
                ..Default::default()
            })
            .collect();
        WorldMap { name: "T".into(), width: rows[0].len() as u32, height: rows.len() as u32, tiles, last_object_uid: None, objects: Vec::new() }
    }

    fn items() -> ItemNames {
        ItemNames::with_collision(&[(DIRT as u32, 1), (PLATFORM as u32, 2)])
    }

    #[test]
    fn paths_go_around_blocks() {
        let m = map(&["......", ".####.", "......", "######"]);
        let path = find_path(&m, &items(), 0, (0, 2), (5, 0)).unwrap();
        assert_eq!(path.last(), Some(&(5, 0)));
        assert!(path.iter().all(|&(x, y)| m.tile(x, y).unwrap().fg == 0));
        assert!(find_path(&m, &items(), 0, (0, 2), (2, 3)).is_none()); // inside dirt
    }

    #[test]
    fn platforms_block_going_down() {
        let m = map(&["......", "======", "......"]);
        assert!(find_path(&m, &items(), 0, (0, 0), (0, 2)).is_none());
        assert!(find_path(&m, &items(), 0, (0, 2), (0, 0)).is_some());
    }

    fn ic_in(m: WorldMap, ctrl: fn() -> bool) -> Interceptor {
        let mut cfg = Config::default();
        cfg.log.text = false;
        cfg.log.variant = false;
        let mut ic = Interceptor::with_items(Arc::new(cfg), Arc::new(items()));
        ic.world = Some(m);
        ic.session.net_id = Some(2);
        ic.session.pos = Some((0.0, 64.0)); // tile (0,2)
        ic.teleport.enabled = true;
        ic.teleport.ctrl = ctrl;
        ic.teleport.build_range = Some(2);
        ic.teleport.punch_range = Some(2);
        ic
    }

    fn click(ic: &mut Interceptor, x: i32, y: i32) -> Verdict {
        let pkt = GameUpdatePacket {
            packet_type: GamePacketType::State,
            flags: PacketFlags::STANDING | PacketFlags::PUNCH | PacketFlags::TILE_CHANGE,
            vec_x: 0.0,
            vec_y: 64.0,
            int_x: x,
            int_y: y,
            ..Default::default()
        };
        ic.handle(Direction::ToServer, &Message::Game(pkt).encode())
    }

    #[test]
    fn ctrl_click_walks_there() {
        let mut ic = ic_in(map(&["......", ".####.", "......", "######"]), || true);
        assert_eq!(click(&mut ic, 5, 0), Verdict::Drop);
        let effects = ic.take_effects();
        let states = effects
            .iter()
            .filter(|e| matches!(e, Effect::Later(_, inner) if matches!(**inner, Effect::ToServer(_))))
            .count();
        assert_eq!(states, 7); // (1,2)..(5,2), (5,1), (5,0)
        // Client-side calls, in order: OnSetPos, then the "Teleported to" line.
        let calls: Vec<(i32, crate::protocol::VariantList)> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::Later(_, inner) => match inner.as_ref() {
                    Effect::ToClient(b) => match Message::decode(b)? {
                        Message::Game(p) => Some((p.net_id, crate::protocol::VariantList::parse(&p.extra)?)),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(calls[0].0, 2);
        assert_eq!(calls[0].1.function(), Some("OnSetPos"));
        assert_eq!(calls[0].1.get(1), Some(&Variant::Vec2(160.0, 0.0)));
        let text = calls[1].1.get(1).and_then(Variant::as_str).unwrap();
        assert!(text.contains("Teleported to `w(5, 0)``"), "{text}");
        // The client's own movement is held back meanwhile.
        assert_eq!(click(&mut ic, 0, 0), Verdict::Drop);
    }

    #[test]
    fn plain_clicks_respect_the_real_range() {
        let mut ic = ic_in(map(&["......", "......", "......"]), || false);
        assert_eq!(click(&mut ic, 2, 2), Verdict::Forward);
        assert_eq!(click(&mut ic, 5, 0), Verdict::Drop);
        ic.teleport.enabled = false;
        assert_eq!(click(&mut ic, 5, 0), Verdict::Forward);
    }

    #[test]
    fn range_follows_the_ctrl_key() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static CTRL: AtomicBool = AtomicBool::new(false);
        let mut ic = ic_in(map(&["."]), || CTRL.load(Ordering::Relaxed));
        let state = GameUpdatePacket {
            packet_type: GamePacketType::SetCharacterState,
            net_id: 2,
            jump_count: 131,
            animation_type: 131,
            ..Default::default()
        };
        // Ctrl up: the real range goes through unchanged.
        assert_eq!(ic.handle(Direction::ToClient, &Message::Game(state).encode()), Verdict::Forward);
        let shown = |ic: &mut Interceptor| -> Option<u8> {
            ic.take_effects().iter().find_map(|e| match e {
                Effect::ToClient(b) => match Message::decode(b)? {
                    Message::Game(p) if p.packet_type == GamePacketType::SetCharacterState => Some(p.jump_count),
                    _ => None,
                },
                _ => None,
            })
        };
        ic.tick();
        assert_eq!(shown(&mut ic), None);
        CTRL.store(true, Ordering::Relaxed);
        ic.tick();
        assert_eq!(shown(&mut ic), Some(126 + EXTENDED_RANGE));
        ic.tick();
        assert_eq!(shown(&mut ic), None); // only on change
        CTRL.store(false, Ordering::Relaxed);
        ic.tick();
        assert_eq!(shown(&mut ic), Some(131));
    }

    #[test]
    fn range_is_enlarged_for_the_client_only() {
        let mut ic = ic_in(map(&["."]), || true);
        let state = GameUpdatePacket {
            packet_type: GamePacketType::SetCharacterState,
            net_id: 2,
            jump_count: 131,
            animation_type: 131,
            ..Default::default()
        };
        let Verdict::Replace(b) = ic.handle(Direction::ToClient, &Message::Game(state).encode()) else { panic!() };
        let Some(Message::Game(p)) = Message::decode(&b) else { panic!() };
        assert_eq!((p.jump_count, p.animation_type), (126 + EXTENDED_RANGE, 126 + EXTENDED_RANGE));
        assert_eq!((ic.teleport.build_range, ic.teleport.punch_range), (Some(5), Some(5)));
    }
}
