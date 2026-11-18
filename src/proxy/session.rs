use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;

/// What the proxy knows about the current player, learned from passing traffic.
#[derive(Debug, Default, Clone)]
pub struct Session {
    pub name: Option<String>,
    pub net_id: Option<i32>,
    pub user_id: Option<i64>,
    pub world: Option<String>,
    pub previous_world: Option<String>,
    /// Real server the proxy is currently connected to.
    pub server: Option<SocketAddr>,
    pub country: Option<String>,
    /// Our position in pixels, from our `State` packets and `OnSetPos`.
    pub pos: Option<(f32, f32)>,
    /// From our last `State` packet: drops land on the tile we face.
    pub facing_left: bool,
    /// Display name as the server sent it, colour codes included (for `/nick` undo).
    pub raw_name: Option<String>,
    /// Skin colour from the server (for `/skin` undo).
    pub skin: Option<u32>,
    pub level: Option<u32>,
    pub gems: Option<u64>,
    /// Other players in the current world: netID -> display name (colours stripped).
    pub players: BTreeMap<i32, String>,
    /// Same players' names as the server sent them (colours kept), for /hidename undo.
    pub player_raw: BTreeMap<i32, String>,
    /// Other players' positions in pixels, from `OnSpawn` and their `State` packets.
    pub player_pos: BTreeMap<i32, (f32, f32)>,
    /// Other players' userIDs (netID -> userID), for lock owners.
    pub player_user: BTreeMap<i32, u32>,
    /// Every userID -> name seen this run (kept across worlds).
    pub known_users: HashMap<u32, String>,
    /// World owner's name from the world entry message.
    pub world_owner: Option<String>,
    /// Mods seen being added ("Calm nerves." -> when it ends, if said).
    pub mods: BTreeMap<String, Option<std::time::Instant>>,
}

/// "... (`$Calm nerves.`` mod added, `$2 mins`` left)" -> ("Calm nerves.", true, Some(120 s)).
pub fn parse_mod(text: &str) -> Option<(String, bool, Option<std::time::Duration>)> {
    let plain = strip_colors(text);
    let open = plain.rfind('(')?;
    let inner = plain[open + 1..].trim_end().trim_end_matches(')');
    if let Some((name, rest)) = inner.split_once(" mod added") {
        let time = rest.trim_start_matches(',').trim().strip_suffix(" left").and_then(|t| {
            let (n, unit) = t.split_once(' ')?;
            let n: u64 = n.parse().ok()?;
            let secs = match unit.trim_end_matches('s') {
                "sec" | "second" => n,
                "min" | "minute" => n * 60,
                "hour" => n * 3600,
                "day" => n * 86_400,
                _ => return None,
            };
            Some(std::time::Duration::from_secs(secs))
        });
        return Some((name.trim().to_string(), true, time));
    }
    let name = inner.strip_suffix(" mod removed")?;
    Some((name.trim().to_string(), false, None))
}

impl Session {
    /// A console line about a mod being added or removed.
    pub fn note_mod(&mut self, text: &str) {
        let Some((name, added, time)) = parse_mod(text) else { return };
        if added {
            self.mods.insert(name, time.map(|t| std::time::Instant::now() + t));
        } else {
            self.mods.remove(&name);
        }
    }

    /// Mods still on (the ones without a time stay until removed).
    pub fn active_mods(&self) -> impl Iterator<Item = (&str, Option<std::time::Instant>)> {
        let now = std::time::Instant::now();
        self.mods.iter().filter(move |(_, t)| t.is_none_or(|t| t > now)).map(|(n, t)| (n.as_str(), *t))
    }

    /// A mod whose name starts with `prefix` (lowercase) is on.
    pub fn has_mod(&self, prefix: &str) -> bool {
        self.active_mods().any(|(n, _)| n.to_lowercase().starts_with(prefix))
    }

    pub fn enter_world(&mut self, name: String) {
        if self.world.as_deref() != Some(name.as_str()) {
            if let Some(old) = self.world.take() {
                self.previous_world = Some(old);
            }
            self.world = Some(name);
        }
    }

    pub fn leave_world(&mut self) {
        if let Some(old) = self.world.take() {
            self.previous_world = Some(old);
        }
        self.net_id = None;
        self.players.clear();
        self.player_raw.clear();
        self.player_pos.clear();
        self.player_user.clear();
        self.world_owner = None;
    }

    /// Display name for a userID, when we have seen it.
    pub fn user_name(&self, user_id: u32) -> Option<&str> {
        self.known_users.get(&user_id).map(String::as_str)
    }

    /// The player whose name starts with `query` (case-insensitive); an exact match wins.
    pub fn find_player(&self, query: &str) -> Option<(i32, &str)> {
        let q = query.to_lowercase();
        let mut found = None;
        for (id, name) in &self.players {
            let n = name.to_lowercase();
            if n == q {
                return Some((*id, name));
            }
            if found.is_none() && n.starts_with(&q) {
                found = Some((*id, name.as_str()));
            }
        }
        found
    }

    pub fn player_by_net_id(&self, net_id: i32) -> Option<&str> {
        self.players.get(&net_id).map(String::as_str)
    }
}

/// Strips Growtopia colour codes (`` `X ``).
pub fn strip_colors(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '`' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(strip_colors("`#@Serhat``"), "@Serhat");
    }

    #[test]
    fn mods_from_console_lines() {
        use std::time::Duration;
        assert_eq!(
            parse_mod("Steady hands of a surgeon. (`$Calm nerves.`` mod added, `$2 mins`` left)"),
            Some(("Calm nerves.".into(), true, Some(Duration::from_secs(120))))
        );
        assert_eq!(parse_mod("Butterfingers again. (`$Calm nerves.`` mod removed)"), Some(("Calm nerves.".into(), false, None)));
        assert_eq!(
            parse_mod("You've clearly done far too much punching, go see a Doctor! (`$Malady: Torn Punching Muscle! Reduced Punch Damage`` mod added)").map(|m| m.2),
            Some(None)
        );
        assert_eq!(parse_mod("You are recovering from surgery. (`$Recovering`` mod added, `$2 hours`` left)").unwrap().2, Some(Duration::from_secs(7200)));
        assert_eq!(parse_mod("Collected `w1 Dirt``"), None);
        let mut s = Session::default();
        s.note_mod("Reduce your skill fails by half in both Surgery and Startopia missions. (`$Food: Spicey Skills`` mod added, `$30 mins`` left)");
        assert!(s.has_mod("food:") && !s.has_mod("calm nerves"));
        s.note_mod("Your stomach's too spicey. (`$Food: Spicey Skills`` mod removed)");
        assert!(!s.has_mod("food:"));
    }

    #[test]
    fn world_history() {
        let mut s = Session::default();
        s.enter_world("START".into());
        s.enter_world("BUY".into());
        s.leave_world();
        assert_eq!(s.world, None);
        assert_eq!(s.previous_world.as_deref(), Some("BUY"));
    }
}
