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
}

impl Session {
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
    fn world_history() {
        let mut s = Session::default();
        s.enter_world("START".into());
        s.enter_world("BUY".into());
        s.leave_world();
        assert_eq!(s.world, None);
        assert_eq!(s.previous_world.as_deref(), Some("BUY"));
    }
}
