//! `/dropall` (every stack) and `/dropall1` (one of each item), plus `/logoff`.
//!
//! Dropping everything cannot be undone, so both first show a window with
//! what will be dropped and a button to go on. The drops then go one by one
//! through the normal drop dialog (answered by the proxy, like `/dw`), spaced
//! out so the server is not flooded; `/dropall stop` ends it.
//!
//! Untradeable items (items.dat flag 0x8000: Fist, Wrench, My First World
//! Lock...) cannot be dropped and are left out. When the tile in front is
//! full ("You can't drop that here, find an emptier spot!") the player steps
//! one tile back, still facing the same way, so the drops land on the tile
//! it stood on, and the items that did not drop are tried again.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::commands::{say, warn};
use super::dialog::{Dialog, DialogReturn};
use super::effect::Effect;
use super::fast_item::PlannedDrop;
use super::interceptor::Interceptor;
use super::session::strip_colors;

/// Gap between two drop requests.
const SPACING: Duration = Duration::from_millis(400);
/// Never dropped: Fist and Wrench.
const KEEP: [u32; 2] = [18, 32];
const ICON: u32 = 2;
/// Steps back for full tiles in one `/dropall`, at most.
const MAX_STEPS: u32 = 40;
/// Overlays this soon after a step belong to drops sent before it.
const STEP_SETTLE: Duration = Duration::from_millis(900);

#[derive(Debug, Default)]
pub struct DropAll {
    /// Waiting for the confirm button: `None` = whole stacks, `Some(1)` = one each.
    pending: Option<Option<u32>>,
    queue: VecDeque<(u32, String)>,
    amount: Option<u32>,
    next_at: Option<Instant>,
    /// Steps back taken for full tiles, and until when overlays are old news.
    steps: u32,
    settle_until: Option<Instant>,
    /// The last drop request, for full-tile overlays that come after the queue ended.
    last_sent: Option<Instant>,
}

/// What would be dropped: (item, name, count owned), biggest stacks first.
fn items(ic: &Interceptor) -> Vec<(u32, String, u32)> {
    let mut list: Vec<(u32, String, u32)> = ic
        .inventory
        .iter()
        .filter(|(id, n)| *n > 0 && !KEEP.contains(id) && !ic.items.untradeable(*id))
        .map(|(id, n)| (id, ic.items.name(id), n))
        .collect();
    list.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    list
}

fn confirm(ic: &mut Interceptor, amount: Option<u32>) {
    if ic.session.world.is_none() {
        return warn(ic, "Enter a world first");
    }
    if ic.inventory.slots == 0 {
        return warn(ic, "Inventory not known yet, open it once or re-enter the world");
    }
    let list = items(ic);
    if list.is_empty() {
        return warn(ic, "Nothing to drop");
    }
    ic.drop_all.pending = Some(amount);
    let what = if amount.is_some() { "one of each item" } else { "every stack" };
    let total: u32 = list.iter().map(|(_, _, n)| amount.unwrap_or(*n).min(*n)).sum();
    let secs = (list.len() as u64 * SPACING.as_millis() as u64).div_ceil(1000);
    let world = ic.session.world.clone().unwrap_or_default();
    let mut d = Dialog::new("dropall")
        .title(if amount.is_some() { "Drop one of each" } else { "Drop everything" }, ICON)
        .text(&format!("`4This drops {what} in `w{world}``. Anyone here can pick it up.``"))
        .small(&format!("{} items, {total} in total, about {secs} s. Untradeable items stay. Stop with /dropall stop.", list.len()))
        .spacer();
    for (id, name, n) in list.iter().take(40) {
        let count = amount.unwrap_or(*n).min(*n);
        d = d.item(*id, &format!("{name} : {count}"));
    }
    if list.len() > 40 {
        d = d.small(&format!("... and {} more", list.len() - 40));
    }
    d = d.spacer().button("go", "`4Drop them``");
    super::tools::show(ic, d, "Cancel", "");
}

/// `/dropall [stop]`.
pub fn dropall(ic: &mut Interceptor, args: &[&str]) {
    if args.first().is_some_and(|a| a.eq_ignore_ascii_case("stop")) {
        return stop(ic);
    }
    confirm(ic, None);
}

/// `/dropall1`.
pub fn dropall1(ic: &mut Interceptor, _: &[&str]) {
    confirm(ic, Some(1));
}

fn stop(ic: &mut Interceptor) {
    let left = ic.drop_all.queue.len();
    ic.drop_all.queue.clear();
    ic.drop_all.next_at = None;
    if left > 0 {
        say(ic, format!("Drop all stopped, {left} items not dropped"));
    } else {
        warn(ic, "Nothing is being dropped");
    }
}

pub fn on_dialog(ic: &mut Interceptor, ret: &DialogReturn) -> bool {
    if ret.name != "dropall" {
        return false;
    }
    let Some(amount) = ic.drop_all.pending.take() else { return true };
    if ret.button != Some("go") {
        return true;
    }
    let list = items(ic);
    say(ic, format!("Dropping `w{}`` items, one every {} ms", list.len(), SPACING.as_millis()));
    ic.drop_all.queue = list.into_iter().map(|(id, name, _)| (id, name)).collect();
    ic.drop_all.amount = amount;
    ic.drop_all.next_at = Some(Instant::now());
    ic.drop_all.steps = 0;
    ic.drop_all.settle_until = None;
    true
}

/// The tile in front is full: step back and try the undropped items again.
pub fn on_overlay(ic: &mut Interceptor, text: &str) {
    let plain = strip_colors(text).to_lowercase();
    if !plain.contains("emptier spot") && !plain.contains("somewhere with open space") {
        return;
    }
    let now = Instant::now();
    let running = ic.drop_all.next_at.is_some() || ic.drop_all.last_sent.is_some_and(|t| now < t + Duration::from_secs(3));
    if !running || ic.drop_all.settle_until.is_some_and(|t| now < t) {
        return;
    }
    // Our drops whose dialog never came did not drop: again, in order.
    let (undropped, others): (Vec<_>, Vec<_>) = ic.planned_drops.drain(..).partition(|p| p.quiet);
    ic.planned_drops = others;
    for p in undropped.into_iter().rev() {
        ic.drop_all.queue.push_front((p.item_id, p.name));
    }
    if ic.drop_all.queue.is_empty() {
        return;
    }
    if ic.drop_all.steps >= MAX_STEPS {
        return give_up(ic, "Drop all stopped: too many full tiles");
    }
    let Some(walk) = step_back(ic) else {
        return give_up(ic, "Drop all stopped: the tile is full and there is no room behind you");
    };
    ic.drop_all.steps += 1;
    ic.drop_all.settle_until = Some(now + walk + STEP_SETTLE);
    ic.drop_all.next_at = Some(now + walk + STEP_SETTLE);
}

fn give_up(ic: &mut Interceptor, why: &str) {
    let left = ic.drop_all.queue.len();
    ic.drop_all.queue.clear();
    ic.drop_all.next_at = None;
    warn(ic, format!("{why} ({left} items left)"));
}

/// One tile away from where the player faces, still facing the same way.
fn step_back(ic: &mut Interceptor) -> Option<Duration> {
    let (pos, net_id) = (ic.session.pos?, ic.session.net_id?);
    let start = super::teleport::tile_of(pos);
    let left = ic.session.facing_left;
    let dir = if left { 1 } else { -1 };
    let target = (start.0 + dir, start.1);
    let user_id = ic.session.user_id.unwrap_or(0) as u32;
    let map = ic.world.as_ref()?;
    if !super::teleport::passable(map, &ic.items, user_id, target.0, target.1, (dir, 0)) {
        return None;
    }
    Some(super::teleport::walk_facing(ic, start, &[target], net_id, Some(left)))
}

/// Called every bridge loop: sends the next drop request when it is time.
pub fn tick(ic: &mut Interceptor, now: Instant) {
    let Some(at) = ic.drop_all.next_at else { return };
    if now < at {
        return;
    }
    if ic.session.world.is_none() {
        ic.drop_all.queue.clear();
    }
    let Some((id, name)) = ic.drop_all.queue.pop_front() else {
        ic.drop_all.next_at = None;
        say(ic, "Drop all finished");
        return;
    };
    let mut plan = PlannedDrop::new(id, name, ic.drop_all.amount, Duration::ZERO);
    plan.quiet = true;
    let request = Effect::text(plan.request());
    ic.planned_drops.push(plan);
    ic.emit(request);
    ic.drop_all.last_sent = Some(now);
    ic.drop_all.next_at = Some(now + SPACING);
}

/// `/logoff`: back to the login screen (`action|quit`, what the game's own log off sends).
pub fn logoff(ic: &mut Interceptor, _: &[&str]) {
    say(ic, "Logging off...");
    ic.emit(Effect::action("action|quit\n"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::protocol::TextPacket;
    use std::sync::Arc;

    fn ic() -> Interceptor {
        let mut cfg = Config::default();
        cfg.log.text = false;
        cfg.log.variant = false;
        let mut ic = Interceptor::new(Arc::new(cfg));
        ic.session.enter_world("T".into());
        ic.inventory.slots = 20;
        for (id, n) in [(18, 1), (32, 1), (2, 50), (242, 3)] {
            ic.inventory.add(id, n);
        }
        ic
    }

    fn sent(ic: &mut Interceptor) -> Vec<String> {
        ic.take_effects()
            .iter()
            .filter_map(|e| match e {
                Effect::ToServer(b) => Some(String::from_utf8_lossy(&b[4..]).trim_end_matches('\0').to_string()),
                _ => None,
            })
            .collect()
    }

    fn press(ic: &mut Interceptor, button: &str) {
        let tp = TextPacket::parse(&format!("action|dialog_return\ndialog_name|gtproxy_dropall\nbuttonClicked|{button}\n"));
        on_dialog(ic, &DialogReturn::parse(&tp).unwrap());
    }

    #[test]
    fn asks_first_then_drops_one_by_one_without_fist_and_wrench() {
        let mut ic = ic();
        dropall(&mut ic, &[]);
        assert!(sent(&mut ic).is_empty()); // only the window
        press(&mut ic, "go");
        let now = Instant::now();
        tick(&mut ic, now);
        assert_eq!(sent(&mut ic), vec!["action|drop\n|itemID|2\n"]);
        tick(&mut ic, now); // too soon
        assert!(sent(&mut ic).is_empty());
        tick(&mut ic, now + SPACING);
        assert_eq!(sent(&mut ic), vec!["action|drop\n|itemID|242\n"]);
        tick(&mut ic, now + SPACING * 2);
        assert!(sent(&mut ic).is_empty());
        assert!(ic.drop_all.next_at.is_none());
    }

    #[test]
    fn untradeable_items_stay() {
        use crate::items::ItemNames;
        let mut cfg = Config::default();
        cfg.log.text = false;
        cfg.log.variant = false;
        let names = ItemNames::with_flags(&[(9640, crate::items::UNTRADEABLE)]);
        let mut ic = Interceptor::with_items(Arc::new(cfg), Arc::new(names));
        for (id, n) in [(2, 5), (9640, 1)] {
            ic.inventory.add(id, n);
        }
        assert_eq!(items(&ic).iter().map(|i| i.0).collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn full_tile_steps_back_and_tries_again() {
        use super::super::world::{Tile, WorldMap};
        let mut ic = ic();
        // 5x2: air row, dirt floor; the player at 2,0 facing right.
        let mut tiles = vec![Tile::default(); 5];
        tiles.extend((0..5).map(|_| Tile { fg: 2, ..Default::default() }));
        ic.world = Some(WorldMap { name: "T".into(), width: 5, height: 2, tiles, last_object_uid: Some(0), objects: Vec::new() });
        ic.session.net_id = Some(1);
        ic.session.pos = Some((64.0, 0.0));
        dropall(&mut ic, &[]);
        press(&mut ic, "go");
        let now = Instant::now();
        tick(&mut ic, now);
        assert_eq!(sent(&mut ic), vec!["action|drop\n|itemID|2\n"]);
        // Full: step back to 1,0 (facing right still), item 2 goes back in the queue.
        on_overlay(&mut ic, "You can't drop that here, find an emptier spot!");
        let steps: Vec<_> = ic
            .take_effects()
            .into_iter()
            .filter_map(|e| match e {
                Effect::Later(_, inner) => match *inner {
                    Effect::ToServer(b) => crate::protocol::GameUpdatePacket::from_bytes(&b[4..]),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(steps.len(), 1);
        assert_eq!((steps[0].vec_x, steps[0].flags.contains(crate::protocol::game_packet::PacketFlags::FACING_LEFT)), (32.0, false));
        assert_eq!(ic.session.pos, Some((32.0, 0.0)));
        assert_eq!(ic.drop_all.queue.front().map(|q| q.0), Some(2));
        // A second overlay for a drop sent before the step is old news.
        on_overlay(&mut ic, "You can't drop that here, find an emptier spot!");
        assert_eq!(ic.drop_all.steps, 1);
        // Later: dropping goes on with item 2.
        tick(&mut ic, now + Duration::from_secs(3));
        assert_eq!(sent(&mut ic), vec!["action|drop\n|itemID|2\n"]);
        // At the wall (0,0 then -1): no room, it stops.
        ic.session.pos = Some((0.0, 0.0));
        ic.drop_all.settle_until = None;
        on_overlay(&mut ic, "You can't drop that here, find an emptier spot!");
        assert!(ic.drop_all.queue.is_empty() && ic.drop_all.next_at.is_none());
    }

    #[test]
    fn cancel_and_stop() {
        let mut ic = ic();
        dropall1(&mut ic, &[]);
        press(&mut ic, "");
        tick(&mut ic, Instant::now());
        assert!(sent(&mut ic).is_empty());
        dropall1(&mut ic, &[]);
        press(&mut ic, "go");
        assert_eq!(ic.drop_all.amount, Some(1));
        dropall(&mut ic, &["stop"]);
        tick(&mut ic, Instant::now());
        assert!(sent(&mut ic).is_empty());
    }
}
