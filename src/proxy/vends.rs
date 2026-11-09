//! Vending machine finder.
//!
//! Every world map we receive lists its vending machines (tile extra 24: item
//! and price). On world entry, and when a machine changes, the world's
//! machines are written to `vends.json`. `/vendfind <item>` searches them.
//! Only what the map already tells the client is used; nothing is sent.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::commands::{go_to, say, warn};
use super::dialog::{Dialog, DialogReturn};
use super::interceptor::Interceptor;
use super::world::Special;

/// Results per page.
const PAGE_SIZE: usize = 15;
const VEND_ICON: u32 = 2978;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vend {
    pub item: u32,
    /// `> 0`: WL each. `< 0`: that many items per WL.
    pub price: i32,
    pub x: i32,
    pub y: i32,
    /// Unix seconds when we last saw it.
    pub seen: u64,
}

impl Vend {
    /// World Locks paid for one item, for sorting.
    fn wl_per_item(&self) -> f64 {
        if self.price > 0 { self.price as f64 } else { 1.0 / (-(self.price as f64)) }
    }

    /// Short form for one-line rows: `5 WL` or `20/WL`.
    pub fn price_short(&self) -> String {
        if self.price > 0 { format!("{} WL", self.price) } else { format!("{}/WL", -self.price) }
    }
}

#[derive(Debug, Default)]
pub struct VendDb {
    /// `None` in tests: nothing is written.
    path: Option<PathBuf>,
    pub worlds: BTreeMap<String, Vec<Vend>>,
    /// Last search and the page shown, for the row and page buttons.
    last: Vec<(String, Vend)>,
    query: String,
    page: usize,
}

impl VendDb {
    pub fn load(path: PathBuf) -> Self {
        let worlds = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| match serde_json::from_str(&s) {
                Ok(w) => Some(w),
                Err(e) => {
                    tracing::warn!("{} not readable: {e}", path.display());
                    None
                }
            })
            .unwrap_or_default();
        Self { path: Some(path), worlds, ..Default::default() }
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        match serde_json::to_string(&self.worlds) {
            Ok(s) => {
                if let Err(e) = std::fs::write(path, s) {
                    tracing::warn!("cannot write {}: {e}", path.display());
                }
            }
            Err(e) => tracing::warn!("cannot save vends: {e}"),
        }
    }

    /// Vends whose item name contains `query`, cheapest first.
    pub fn search(&self, query: &str, name: impl Fn(u32) -> String) -> Vec<(String, Vend)> {
        let q = query.trim().to_lowercase();
        let mut out: Vec<(String, Vend)> = self
            .worlds
            .iter()
            .flat_map(|(w, list)| list.iter().map(move |v| (w.clone(), v.clone())))
            .filter(|(_, v)| q.is_empty() || name(v.item).to_lowercase().contains(&q))
            .collect();
        out.sort_by(|a, b| {
            a.1.wl_per_item().partial_cmp(&b.1.wl_per_item()).unwrap_or(std::cmp::Ordering::Equal).then(b.1.seen.cmp(&a.1.seen))
        });
        out
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// How long ago, or the date (`dd.mm`) when older than a week.
fn age_short(secs: u64, now: u64) -> String {
    let ago = now.saturating_sub(secs);
    match ago {
        0..3600 => format!("{}m", ago / 60),
        3600..86_400 => format!("{}h", ago / 3600),
        86_400..604_800 => format!("{}d", ago / 86_400),
        _ => date_text(secs, now).chars().take(5).collect(),
    }
}

/// `dd.mm.yyyy` (UTC) and how long ago.
fn date_text(secs: u64, now: u64) -> String {
    let days = (secs / 86_400) as i64;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let ago = now.saturating_sub(secs);
    let ago = match ago {
        0..3600 => format!("{}m ago", ago / 60),
        3600..86_400 => format!("{}h ago", ago / 3600),
        _ => format!("{}d ago", ago / 86_400),
    };
    format!("{d:02}.{m:02}.{y} ({ago})")
}

/// Records the vending machines of the world we are in (replacing the old list).
pub fn record_world(ic: &mut Interceptor) {
    let Some(map) = ic.world.as_ref() else { return };
    if map.name.is_empty() {
        return;
    }
    let t = now();
    let w = map.width.max(1) as i32;
    let list: Vec<Vend> = map
        .tiles
        .iter()
        .enumerate()
        .filter_map(|(i, tile)| match tile.special {
            Special::Vend { item, price } if item != 0 && price != 0 => {
                Some(Vend { item, price, x: i as i32 % w, y: i as i32 / w, seen: t })
            }
            _ => None,
        })
        .collect();
    let name = map.name.to_ascii_uppercase();
    if list.is_empty() && !ic.vends.worlds.contains_key(&name) {
        return;
    }
    crate::report::vends(
        &name,
        list.iter()
            .map(|v| crate::report::VendItem { x: v.x, y: v.y, item: v.item, name: ic.items.name(v.item), price: v.price })
            .collect(),
    );
    tracing::info!("{} vending machines with items in {name}", list.len());
    if list.is_empty() {
        ic.vends.worlds.remove(&name);
    } else {
        ic.vends.worlds.insert(name, list);
    }
    ic.vends.save();
}

/// Runs a search and shows its first page.
fn search(ic: &mut Interceptor, query: &str) {
    let items = ic.items.clone();
    ic.vends.last = ic.vends.search(query, |id| items.name(id));
    ic.vends.query = query.trim().to_string();
    ic.vends.page = 0;
    show(ic);
}

/// The window for the current search and page.
fn show(ic: &mut Interceptor) {
    let items = ic.items.clone();
    let db = &ic.vends;
    let total: usize = db.worlds.values().map(Vec::len).sum();
    let pages = db.last.len().div_ceil(PAGE_SIZE).max(1);
    let page = db.page.min(pages - 1);
    let t = now();
    let mut dialog = Dialog::new("vendfind")
        .title("Vend Finder", VEND_ICON)
        .text_input("query", "", &db.query, 30)
        .small(&format!("`s{total} vends from {} worlds. Tap a row to warp there.``", db.worlds.len()));
    if !db.query.is_empty() {
        dialog = dialog.spacer();
        if db.last.is_empty() {
            dialog = dialog.text(&format!("No vend sells `w{}`` in the worlds you visited.", db.query));
        } else {
            dialog = dialog.small(&format!(
                "`w{}`` results, cheapest first{}",
                db.last.len(),
                if pages > 1 { format!("  `s(page {} of {pages})``", page + 1) } else { String::new() }
            ));
            let start = page * PAGE_SIZE;
            for (i, (world, v)) in db.last.iter().enumerate().skip(start).take(PAGE_SIZE) {
                // Name, price, where. Age only once it is worth knowing.
                let age = t.saturating_sub(v.seen);
                let age = if age >= 3600 { format!("  `4{}``", age_short(v.seen, t)) } else { String::new() };
                let row = format!("`w{}``  `2{}``  `9{world}`` `s{},{}``{age}", items.name(v.item), v.price_short(), v.x, v.y);
                dialog = dialog.item_button(v.item, &row, &format!("go_{i}"));
            }
            if pages > 1 {
                dialog = dialog.spacer();
                if page + 1 < pages {
                    dialog = dialog.button("next", "Next page");
                }
                if page > 0 {
                    dialog = dialog.button("prev", "Previous page");
                }
            }
        }
    }
    ic.vends.page = page;
    super::tools::show(ic, dialog, "Close", "Search");
}

/// `/vendfind [item]`.
pub fn vendfind(ic: &mut Interceptor, args: &[&str]) {
    search(ic, &args.join(" "));
}

pub fn on_dialog(ic: &mut Interceptor, ret: &DialogReturn) -> bool {
    if ret.name != "vendfind" {
        return false;
    }
    match ret.button {
        Some("next") => {
            ic.vends.page += 1;
            show(ic);
        }
        Some("prev") => {
            ic.vends.page = ic.vends.page.saturating_sub(1);
            show(ic);
        }
        Some(b) if b.starts_with("go_") => {
            let Some((world, v)) = b[3..].parse::<usize>().ok().and_then(|i| ic.vends.last.get(i).cloned()) else {
                return true;
            };
            if ic.world.as_ref().is_some_and(|m| m.name.eq_ignore_ascii_case(&world)) {
                say(ic, format!("The vending machine is at `w({}, {})`` in this world", v.x, v.y));
            } else {
                say(ic, format!("Warping to `w{world}``, the vending machine is at `w({}, {})``", v.x, v.y));
                go_to(ic, &world);
            }
        }
        _ => {
            let query = ret.field("query").unwrap_or_default().to_string();
            if query.trim().is_empty() {
                warn(ic, "Type an item name first");
            }
            search(ic, &query);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vend(item: u32, price: i32) -> Vend {
        Vend { item, price, x: 1, y: 2, seen: 0 }
    }

    #[test]
    fn search_is_cheapest_first() {
        let mut db = VendDb::default();
        db.worlds.insert("A".into(), vec![vend(2, 3), vend(4, 1)]);
        db.worlds.insert("B".into(), vec![vend(2, -10)]);
        let name = |id: u32| if id == 2 { "Dirt".to_string() } else { "Rock".to_string() };
        let r = db.search("dirt", name);
        assert_eq!(r.iter().map(|(w, v)| (w.as_str(), v.price)).collect::<Vec<_>>(), vec![("B", -10), ("A", 3)]);
        assert_eq!((r[0].1.price_short(), r[1].1.price_short()), ("10/WL".to_string(), "3 WL".to_string()));
    }

    #[test]
    fn dates() {
        // 2026-11-06 12:00 UTC, seen 2 hours earlier.
        let now = 1_793_966_400;
        assert_eq!(date_text(now - 7200, now), "06.11.2026 (2h ago)");
        assert_eq!(age_short(now - 7200, now), "2h");
        assert_eq!(age_short(now - 30 * 86_400, now), "07.10");
    }
}
