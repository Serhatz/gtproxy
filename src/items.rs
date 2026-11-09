//! Item names from the game's own `items.dat` cache.
//!
//! The record layout changes with every items.dat version (the game is at 27,
//! public parsers stop at 24-25), so instead of decoding every field this
//! scans for each record's head: `u32 id, u16 flags, u8, u8`, then the
//! XOR-encrypted name and a plain texture name ending in `.rttex`. That shape
//! has been stable across versions and is unlikely to match by accident.

use std::collections::HashMap;
use std::path::PathBuf;

const XOR_KEY: &[u8] = b"PBG892FXX982ABC*";

#[derive(Default)]
pub struct ItemNames {
    names: HashMap<u32, String>,
    /// Collision type: 0 none, 1 solid, 2 platform, 3/10/11 lockable gate,
    /// 4/9 switchable, 5 one-way sideways, 6 VIP entrance, 7 no going up.
    collision: HashMap<u32, u8>,
}

impl ItemNames {
    /// Loads from the game's cache; empty (IDs shown instead of names) on failure.
    pub fn load() -> Self {
        let Some(path) = cache_path() else { return Self::default() };
        match std::fs::read(&path) {
            Ok(data) => {
                let items = Self::parse(&data);
                tracing::info!("loaded {} item names from {}", items.names.len(), path.display());
                items
            }
            Err(e) => {
                tracing::warn!("cannot read {}: {e}; item names unavailable", path.display());
                Self::default()
            }
        }
    }

    pub fn parse(data: &[u8]) -> Self {
        let Some(count) = data.get(2..6).map(|b| u32::from_le_bytes(b.try_into().unwrap())) else {
            return Self::default();
        };
        let mut names = HashMap::with_capacity(count as usize);
        let mut collision = HashMap::with_capacity(count as usize);
        let mut pos = 6;
        for id in 0..count {
            let Some((name, next)) = find_record(data, pos, id) else { continue };
            names.insert(id, name);
            // After the texture name: u32 hash, u8 effect, u32 cooking, u8 x, u8 y,
            // u8 render type, u8 stripey, then u8 collision type.
            if let Some(&c) = data.get(next + 13) {
                collision.insert(id, c);
            }
            pos = next;
        }
        Self { names, collision }
    }

    pub fn name(&self, id: u32) -> String {
        self.names.get(&id).cloned().unwrap_or_else(|| format!("item #{id}"))
    }

    #[cfg(test)]
    pub fn with_collision(pairs: &[(u32, u8)]) -> Self {
        Self { names: HashMap::new(), collision: pairs.iter().copied().collect() }
    }

    pub fn collision(&self, id: u32) -> Option<u8> {
        self.collision.get(&id).copied()
    }

    /// Case-insensitive exact name lookup.
    pub fn find(&self, name: &str) -> Option<u32> {
        self.names.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(&id, _)| id)
    }

    /// The shortest name containing `part` (case-insensitive), lowest ID on ties.
    pub fn search(&self, part: &str) -> Option<u32> {
        let q = part.to_lowercase();
        self.names
            .iter()
            .filter(|(_, n)| n.to_lowercase().contains(&q))
            .min_by_key(|(id, n)| (n.len(), **id))
            .map(|(&id, _)| id)
    }
}

fn cache_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(base).join(r"Growtopia\cache\items.dat"))
}

/// Finds the record of `id` at or after `from`; returns its name and the
/// position just past the texture name.
fn find_record(data: &[u8], from: usize, id: u32) -> Option<(String, usize)> {
    let needle = id.to_le_bytes();
    // Records are a few hundred bytes; give up well before the next IDs.
    let window_end = data.len().min(from + 64 * 1024);
    let mut start = from;
    while start + 4 <= window_end {
        let off = data[start..window_end].windows(4).position(|w| w == needle)? + start;
        if let Some(found) = record_at(data, off, id) {
            return Some(found);
        }
        start = off + 1;
    }
    None
}

fn record_at(data: &[u8], off: usize, id: u32) -> Option<(String, usize)> {
    let mut p = off + 4 + 2 + 1 + 1;
    let name_len = u16::from_le_bytes(data.get(p..p + 2)?.try_into().ok()?) as usize;
    if name_len == 0 || name_len > 128 {
        return None;
    }
    p += 2;
    let enc = data.get(p..p + name_len)?;
    let name: String = enc
        .iter()
        .enumerate()
        .map(|(i, b)| (b ^ XOR_KEY[(id as usize + i) % XOR_KEY.len()]) as char)
        .collect();
    if !name.chars().all(|c| (' '..='~').contains(&c)) {
        return None;
    }
    p += name_len;
    let tex_len = u16::from_le_bytes(data.get(p..p + 2)?.try_into().ok()?) as usize;
    p += 2;
    let tex = data.get(p..p + tex_len)?;
    tex.ends_with(b".rttex").then_some((name, p + tex_len))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u32, name: &str, tex: &str) -> Vec<u8> {
        let mut r = id.to_le_bytes().to_vec();
        r.extend_from_slice(&[0, 0, 0, 0]);
        r.extend_from_slice(&(name.len() as u16).to_le_bytes());
        r.extend(name.bytes().enumerate().map(|(i, b)| b ^ XOR_KEY[(id as usize + i) % 16]));
        r.extend_from_slice(&(tex.len() as u16).to_le_bytes());
        r.extend_from_slice(tex.as_bytes());
        r.extend_from_slice(&[0u8; 40]); // rest of the record, unknown layout
        r
    }

    #[test]
    fn finds_names() {
        let mut data = vec![27, 0];
        data.extend_from_slice(&3u32.to_le_bytes());
        data.extend(record(0, "Blank", "tiles_page1.rttex"));
        data.extend(record(1, "Dirt", "tiles_page1.rttex"));
        data.extend(record(2, "World Lock", "tiles_page2.rttex"));
        let items = ItemNames::parse(&data);
        assert_eq!(items.name(2), "World Lock");
        assert_eq!(items.find("dirt"), Some(1));
        assert_eq!(items.name(9), "item #9");
    }

    #[test]
    fn real_cache_if_present() {
        let Some(path) = cache_path().filter(|p| p.exists()) else { return };
        let items = ItemNames::parse(&std::fs::read(path).unwrap());
        assert_eq!(items.name(242), "World Lock");
        assert_eq!(items.name(1796), "Diamond Lock");
        assert_eq!(items.name(7188), "Blue Gem Lock");
        assert_eq!(items.name(758), "Roulette Wheel");
        // Blank, Dirt, Wooden Platform
        assert_eq!((items.collision(0), items.collision(2), items.collision(102)), (Some(0), Some(1), Some(2)));
    }
}
