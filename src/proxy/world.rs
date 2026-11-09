//! The current world's tiles, read from `SendMapData` and kept up to date
//! from tile change packets. Used to decide where the player can go.
//!
//! Layout (after the 56-byte game packet header):
//! `u16 version, u32 flags, u16 name_len, name, u32 width, u32 height,
//! u32 tile_count, 5 bytes`, then every tile:
//! `u16 fg, u16 bg, u16 parent, u16 flags, [u16 if HAS_PARENT],
//! [u8 kind + extra if HAS_EXTRA_DATA], [u32 len + CBOR for a few items]`.
//!
//! Extras have a different size per kind, so one unknown kind makes the rest
//! of the map unreadable; then parsing fails instead of guessing.

use anyhow::{Result, bail};

pub const HAS_EXTRA_DATA: u16 = 0x0001;
pub const HAS_PARENT: u16 = 0x0002;
pub const FLIPPED_X: u16 = 0x0020;
pub const IS_ON: u16 = 0x0040;
pub const IS_OPEN_TO_PUBLIC: u16 = 0x0080;

const MAX_TILES: u32 = 65_026;
/// Items whose tiles carry an extra CBOR blob after the tile extra.
const CBOR_TILE_IDS: &[u16] = &[
    15376, 15546, 3548, 14662, 14666, 8624, 8630, 8636, 8642, 8648, 8654, 8660, 8666, 8672, 8678, 8684, 8690,
    8696, 8702, 8708, 8714,
];
/// Small/big/huge locks and Builder's lock: area locks, not the world lock.
const AREA_LOCKS: &[u16] = &[202, 204, 206, 4994];

/// Who may pass a gate-like tile.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Access {
    pub owner: u32,
    pub users: Vec<u32>,
}

impl Access {
    pub fn allows(&self, user_id: u32) -> bool {
        self.owner == user_id || self.users.contains(&user_id)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Special {
    #[default]
    None,
    Lock(Access),
    VipEntrance(Access),
    FriendsEntrance(Access),
    StormyCloud { solid: bool },
    /// Vending machine: item for sale and price (`> 0` WL each, `< 0` items per WL).
    Vend { item: u32, price: i32 },
    /// Door, portal, entrance: the label shown on it. The destination is not sent.
    Door { label: String },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tile {
    pub fg: u16,
    pub bg: u16,
    pub flags: u16,
    /// Index of the area lock that owns this tile, when `HAS_PARENT`.
    pub parent: Option<u16>,
    pub special: Special,
}

/// An item lying in the world.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldObject {
    pub item: u16,
    pub x: f32,
    pub y: f32,
    pub count: u8,
    pub uid: u32,
}

#[derive(Debug, Clone)]
pub struct WorldMap {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub tiles: Vec<Tile>,
    /// UID of the last dropped object; the next drop gets this + 1.
    pub last_object_uid: Option<u32>,
    /// Items lying in the world, kept up to date from `ItemChangeObject`.
    pub objects: Vec<WorldObject>,
}

impl WorldMap {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader { d: data, pos: 0 };
        let _version = r.u16()?;
        let _flags = r.u32()?;
        let name = r.string()?;
        let width = r.u32()?;
        let height = r.u32()?;
        let count = r.u32()?;
        r.skip(5)?;
        if count >= MAX_TILES || count != width.saturating_mul(height) {
            bail!("bad tile count {count} for {width}x{height}");
        }
        let mut tiles = Vec::with_capacity(count as usize);
        for i in 0..count {
            let at = r.pos;
            match parse_tile(&mut r) {
                Ok(t) => tiles.push(t),
                Err(e) => bail!("tile {i} ({},{}) at byte {at}: {e}", i % width, i / width),
            }
        }
        // After the tiles: 12 bytes, `u32 object_count, u32 last_dropped_uid`,
        // then each object: `u16 item, f32 x, f32 y, u8 count, u8 flags, u32 uid`.
        let mut last_object_uid = None;
        let mut objects = Vec::new();
        if r.skip(12).is_ok() {
            if let (Ok(n), Ok(last)) = (r.u32(), r.u32()) {
                last_object_uid = Some(last);
                if n < MAX_OBJECTS {
                    match parse_objects(&mut r, n) {
                        Ok(o) => objects = o,
                        Err(e) => tracing::debug!("world objects not read: {e}"),
                    }
                }
            }
        }
        Ok(Self { name, width, height, tiles, last_object_uid, objects })
    }

    /// A new object the server dropped; it takes the next uid.
    pub fn add_object(&mut self, item: u16, x: f32, y: f32, count: u8) {
        let uid = self.last_object_uid.map_or(1, |u| u + 1);
        self.last_object_uid = Some(uid);
        self.objects.push(WorldObject { item, x, y, count, uid });
    }

    /// Someone took part of a stack: the object at that spot now has `count`.
    pub fn set_object_count(&mut self, item: u16, x: f32, y: f32, count: u8) {
        let near = |o: &WorldObject| o.item == item && (o.x - x).abs() < 1.0 && (o.y - y).abs() < 1.0;
        if let Some(o) = self.objects.iter_mut().find(|o| near(o)) {
            o.count = count;
        }
    }

    pub fn remove_object(&mut self, uid: u32) {
        self.objects.retain(|o| o.uid != uid);
    }

    pub fn tile(&self, x: i32, y: i32) -> Option<&Tile> {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return None;
        }
        self.tiles.get((y as u32 * self.width + x as u32) as usize)
    }

    fn tile_mut(&mut self, x: i32, y: i32) -> Option<&mut Tile> {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return None;
        }
        let w = self.width;
        self.tiles.get_mut((y as u32 * w + x as u32) as usize)
    }

    /// `SendTileUpdateData`: the extra data is one full tile.
    pub fn update_tile(&mut self, x: i32, y: i32, data: &[u8]) -> Result<()> {
        let tile = parse_tile(&mut Reader { d: data, pos: 0 })?;
        match self.tile_mut(x, y) {
            Some(t) => *t = tile,
            None => bail!("tile ({x},{y}) outside the map"),
        }
        Ok(())
    }

    /// `TileChangeRequest` from the server: `item` placed, or 18 (fist) = broken.
    pub fn change_tile(&mut self, x: i32, y: i32, item: u32) {
        let Some(t) = self.tile_mut(x, y) else { return };
        if item == 18 {
            if t.fg != 0 {
                t.fg = 0;
                t.special = Special::None;
            } else {
                t.bg = 0;
            }
        } else if let Ok(id) = u16::try_from(item) {
            t.fg = id;
        }
    }

    /// The world lock's access list, if the world is locked.
    pub fn world_lock(&self) -> Option<(&Tile, &Access)> {
        self.tiles.iter().find_map(|t| match &t.special {
            Special::Lock(a) if !AREA_LOCKS.contains(&t.fg) => Some((t, a)),
            _ => None,
        })
    }

    /// Every lock in the world: `(x, y, item, access)`, the world lock first.
    pub fn locks(&self) -> Vec<(i32, i32, u16, &Access)> {
        let w = self.width.max(1) as usize;
        let mut out: Vec<(i32, i32, u16, &Access)> = self
            .tiles
            .iter()
            .enumerate()
            .filter_map(|(i, t)| match &t.special {
                Special::Lock(a) => Some(((i % w) as i32, (i / w) as i32, t.fg, a)),
                _ => None,
            })
            .collect();
        out.sort_by_key(|l| AREA_LOCKS.contains(&l.2));
        out
    }

    /// `(x, y)` of tile index `i`.
    pub fn xy(&self, i: usize) -> (i32, i32) {
        let w = self.width.max(1) as usize;
        ((i % w) as i32, (i / w) as i32)
    }

    /// Whether `user_id` may use the gate at (x, y): its area lock, else the world lock, else public.
    pub fn has_access(&self, x: i32, y: i32, user_id: u32) -> bool {
        let Some(tile) = self.tile(x, y) else { return false };
        if tile.flags & IS_OPEN_TO_PUBLIC != 0 {
            return true;
        }
        if let Some(Special::Lock(a)) = tile.parent.and_then(|p| self.tiles.get(p as usize)).map(|t| &t.special) {
            return a.allows(user_id);
        }
        match self.world_lock() {
            Some((lock, a)) => a.allows(user_id) || lock.flags & IS_OPEN_TO_PUBLIC != 0,
            None => true,
        }
    }
}

fn parse_tile(r: &mut Reader) -> Result<Tile> {
    let fg = r.u16()?;
    let bg = r.u16()?;
    let parent_raw = r.u16()?;
    let flags = r.u16()?;
    let parent = (flags & HAS_PARENT != 0).then_some(parent_raw);
    if flags & HAS_PARENT != 0 {
        r.u16()?;
    }
    let special = if flags & HAS_EXTRA_DATA != 0 {
        let kind = r.u8()?;
        parse_extra(r, kind, fg)?
    } else {
        Special::None
    };
    if CBOR_TILE_IDS.contains(&fg) {
        let len = r.u32()? as usize;
        r.skip(len)?;
    }
    Ok(Tile { fg, bg, flags, parent, special })
}

/// Reads one tile extra; keeps only what pathing needs, skips the rest.
fn parse_extra(r: &mut Reader, kind: u8, fg: u16) -> Result<Special> {
    match kind {
        1 => {
            let label = r.string()?;
            r.skip(1)?;
            return Ok(Special::Door { label });
        }
        2 => {
            r.string()?; // sign text
            r.skip(4)?;
        }
        3 => {
            r.skip(1)?; // settings
            let owner = r.u32()?;
            let n = r.u32()?;
            let mut users = Vec::with_capacity(n as usize);
            for _ in 0..n {
                let id = r.i32()?;
                if id >= 0 {
                    users.push(id as u32); // negative = music BPM
                }
            }
            r.skip(8)?; // minimum level, world timer
            if fg == 5814 {
                r.skip(16)?; // guild lock
            }
            return Ok(Special::Lock(Access { owner, users }));
        }
        4 => r.skip(5)?,  // seed: age, fruit count
        8 => r.skip(1)?,  // dice
        9 => {
            r.skip(4)?; // provider age
            if fg == 10656 {
                r.skip(4)?;
            }
        }
        10 => r.skip(5)?, // achievement block
        11 => {
            r.skip(4)?; // heart monitor
            r.string()?;
        }
        14 => {
            r.string()?; // mannequin
            r.skip(1 + 2 + 2 + 9 * 2)?;
        }
        15 => r.skip(4)?,  // bunny egg
        16 => r.skip(1)?,  // game grave
        17 | 22 | 34 | 45 | 51 | 52 | 58 | 59 | 74 => {} // no data
        18 => r.skip(5)?,  // xenonite
        19 => r.skip(18)?, // phone booth
        20 => {
            let n = r.u16()? as usize; // crystal
            r.skip(n)?;
        }
        21 => {
            r.string()?; // crime in progress
            r.skip(5)?;
        }
        23 => r.skip(4)?, // display block
        24 => {
            let item = r.u32()?;
            let price = r.i32()?;
            return Ok(Special::Vend { item, price });
        }
        25 => {
            r.skip(1)?; // fish tank
            let n = r.u32()? as usize;
            r.skip(n / 2 * 8)?;
        }
        26 => r.skip(5)?, // solar collector
        27 => r.skip(4)?, // forge
        28 => r.skip(6)?, // giving tree
        30 => r.skip(5)?, // steam organ
        31 => {
            r.skip(1)?; // silk worm
            r.string()?;
            r.skip(4 + 4 + 4 + 1 + 4 + 4 + 4 + 4)?;
        }
        32 => {
            let n = r.u32()? as usize; // sewing machine
            r.skip(n * 4)?;
        }
        33 => {
            if fg == 3394 {
                r.string()?; // country flag
            }
        }
        35 => {
            r.skip(4)?; // painting easel
            r.string()?;
        }
        36 => {
            r.string()?; // pet battle cage
            r.skip(12)?;
        }
        37 => {
            r.string()?; // pet trainer
            let n = r.u32()? as usize;
            r.skip(4 + n * 4)?;
        }
        38 | 39 | 40 | 41 | 50 | 57 | 73 | 79 => r.skip(4)?,
        42 => r.skip(21)?, // data bedrock
        43 => r.skip(16)?, // shelf
        44 => {
            r.skip(1)?;
            let owner = r.u32()?;
            let n = r.u32()?;
            let users = (0..n).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;
            return Ok(Special::VipEntrance(Access { owner, users }));
        }
        47 => {
            r.string()?; // fish wall mount
            r.skip(5)?;
        }
        48 => {
            r.string()?; // portrait
            r.skip(4 + 4 + 5 + 1 + 2 + 2)?;
            let hat = r.u16()?;
            r.skip(2 + 4)?;
            if hat == 12958 {
                r.string()?;
            }
        }
        49 => r.skip(9)?, // weather machine 2
        53 => r.skip(8)?, // chemsynth tank
        54 => {
            let len = r.u16()? as usize; // storage block
            r.skip(len / 13 * 13)?;
        }
        55 => {
            r.skip(4)?; // cooking oven
            let n = r.u32()? as usize;
            r.skip(n / 2 * 8 + 12)?;
        }
        56 => {
            r.string()?; // audio rack
            r.skip(4)?;
        }
        60 => r.skip(5)?,  // balloon-o-matic
        61 => r.skip(35)?, // training port
        62 => r.skip(14)?, // item sucker
        63 => {
            let n = r.u32()? as usize; // cybot
            r.skip(n * 15 + 8)?;
        }
        65 => r.skip(17)?, // guild item
        66 => r.skip(1)?,  // growscan
        67 => {
            r.skip(4)?; // containment field node
            let n = r.u32()? as usize;
            r.skip(n * 4)?;
        }
        68 => {
            r.skip(4)?; // spirit board
            r.string()?;
            r.string()?;
            let n = r.u32()? as usize;
            r.skip(n * 4)?;
        }
        69 => r.skip(16)?, // tesseract manipulator
        72 => {
            r.skip(4)?;
            let solid = r.u32()? != 0;
            r.skip(4)?;
            return Ok(Special::StormyCloud { solid });
        }
        75 => {
            let state = r.u32()?; // angelic counting cloud
            r.skip(2)?;
            if state == 2 {
                r.skip(1)?;
            }
        }
        77 => {
            r.skip(4)?; // infinity weather machine
            let n = r.u32()? as usize;
            r.skip(n * 4)?;
        }
        80 => r.skip(8)?, // kraken galactic block
        81 => {
            let owner = r.u32()?;
            r.skip(2)?;
            let n = r.u16()?;
            let users = (0..n).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;
            return Ok(Special::FriendsEntrance(Access { owner, users }));
        }
        other => bail!("unknown tile extra kind {other} on item {fg}"),
    }
    Ok(Special::None)
}

const MAX_OBJECTS: u32 = 20_000;

fn parse_objects(r: &mut Reader, n: u32) -> Result<Vec<WorldObject>> {
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let item = r.u16()?;
        let x = f32::from_bits(r.u32()?);
        let y = f32::from_bits(r.u32()?);
        let count = r.u8()?;
        let _flags = r.u8()?;
        let uid = r.u32()?;
        out.push(WorldObject { item, x, y, count, uid });
    }
    Ok(out)
}

struct Reader<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let Some(s) = self.d.get(self.pos..self.pos.saturating_add(n)) else {
            bail!("need {n} bytes at {}, have {}", self.pos, self.d.len().saturating_sub(self.pos));
        };
        self.pos += n;
        Ok(s)
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn string(&mut self) -> Result<String> {
        let n = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3x2 map: air, dirt, locked world lock / a door, a VIP entrance, air.
    pub fn sample() -> Vec<u8> {
        let mut d = vec![0x19, 0, 0, 0, 0, 0];
        d.extend_from_slice(&5u16.to_le_bytes());
        d.extend_from_slice(b"TESTX");
        for v in [3u32, 2, 6] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&[0; 5]);
        let tile = |d: &mut Vec<u8>, fg: u16, flags: u16| {
            for v in [fg, 0, 0, flags] {
                d.extend_from_slice(&v.to_le_bytes());
            }
        };
        tile(&mut d, 0, 0);
        tile(&mut d, 2, 0);
        // world lock owned by 7, access for 9, plus a BPM entry
        tile(&mut d, 242, HAS_EXTRA_DATA);
        d.push(3);
        d.push(0);
        d.extend_from_slice(&7u32.to_le_bytes());
        d.extend_from_slice(&2u32.to_le_bytes());
        d.extend_from_slice(&9i32.to_le_bytes());
        d.extend_from_slice(&(-100i32).to_le_bytes());
        d.extend_from_slice(&[0; 8]);
        // door with a label
        tile(&mut d, 12, HAS_EXTRA_DATA);
        d.push(1);
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(b"HI");
        d.push(0);
        // VIP entrance owned by 7
        tile(&mut d, 3798, HAS_EXTRA_DATA);
        d.push(44);
        d.push(0);
        d.extend_from_slice(&7u32.to_le_bytes());
        d.extend_from_slice(&0u32.to_le_bytes());
        tile(&mut d, 0, 0);
        d.extend_from_slice(&[0; 12]);
        d.extend_from_slice(&1u32.to_le_bytes()); // object count
        d.extend_from_slice(&41u32.to_le_bytes()); // last dropped uid
        d.extend_from_slice(&242u16.to_le_bytes());
        d.extend_from_slice(&64f32.to_le_bytes());
        d.extend_from_slice(&32f32.to_le_bytes());
        d.push(5);
        d.push(0);
        d.extend_from_slice(&41u32.to_le_bytes());
        d
    }

    #[test]
    fn parses_sample() {
        let w = WorldMap::parse(&sample()).unwrap();
        assert_eq!((w.name.as_str(), w.width, w.height), ("TESTX", 3, 2));
        assert_eq!(w.tile(1, 0).unwrap().fg, 2);
        assert_eq!(w.tile(2, 0).unwrap().special, Special::Lock(Access { owner: 7, users: vec![9] }));
        assert!(matches!(w.tile(1, 1).unwrap().special, Special::VipEntrance(_)));
        assert!(w.tile(3, 0).is_none());
        assert!(w.has_access(0, 1, 9));
        assert_eq!(w.last_object_uid, Some(41));
        assert_eq!(w.objects, vec![WorldObject { item: 242, x: 64.0, y: 32.0, count: 5, uid: 41 }]);
        assert!(!w.has_access(0, 1, 5));
    }

    #[test]
    fn unknown_extra_fails_cleanly() {
        let mut d = sample();
        let pos = d.iter().position(|&b| b == 44).unwrap();
        d[pos] = 250;
        assert!(WorldMap::parse(&d).is_err());
    }

    #[test]
    fn tile_changes() {
        let mut w = WorldMap::parse(&sample()).unwrap();
        w.change_tile(1, 0, 18);
        assert_eq!(w.tile(1, 0).unwrap().fg, 0);
        w.change_tile(0, 0, 2);
        assert_eq!(w.tile(0, 0).unwrap().fg, 2);
    }

    #[test]
    fn real_capture_if_present() {
        // GrowBots kept a SendMapData dump (older game version) next to the repo.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../world.dat");
        let Ok(data) = std::fs::read(&path) else { return };
        let w = WorldMap::parse(&data).unwrap();
        assert_eq!(w.tiles.len() as u32, w.width * w.height);
    }
}
