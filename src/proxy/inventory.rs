//! The player's inventory, followed from server packets.
//!
//! - `SendInventoryState` (login): full list
//! - `ModifyItemInventory`: `value` = item, `jump_count` = removed, `animation_type` = added
//! - "Collected `w5 Dirt``" console lines: picked-up items

use std::collections::BTreeMap;

#[derive(Debug, Default, Clone)]
pub struct Inventory {
    pub slots: u32,
    items: BTreeMap<u16, u32>,
}

impl Inventory {
    /// Body layout: `u8 ?, u32 slots, u16 count, count x (u16 id, u8 amount, u8 flags)`.
    pub fn parse(d: &[u8]) -> Option<Self> {
        let slots = u32::from_le_bytes(d.get(1..5)?.try_into().ok()?);
        let count = u16::from_le_bytes(d.get(5..7)?.try_into().ok()?) as usize;
        let mut items = BTreeMap::new();
        for i in 0..count {
            let e = d.get(7 + i * 4..11 + i * 4)?;
            items.insert(u16::from_le_bytes([e[0], e[1]]), e[2] as u32);
        }
        Some(Self { slots, items })
    }

    pub fn count(&self, id: u32) -> u32 {
        u16::try_from(id).ok().and_then(|id| self.items.get(&id)).copied().unwrap_or(0)
    }

    pub fn add(&mut self, id: u32, amount: u32) {
        if let Ok(id) = u16::try_from(id) {
            *self.items.entry(id).or_default() += amount;
        }
    }

    pub fn remove(&mut self, id: u32, amount: u32) {
        let Ok(id) = u16::try_from(id) else { return };
        if let Some(c) = self.items.get_mut(&id) {
            *c = c.saturating_sub(amount);
            if *c == 0 {
                self.items.remove(&id);
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.items.iter().map(|(&id, &c)| (id as u32, c))
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

/// "Collected `w5 Dirt``. Rarity: `w1``" -> (5, "Dirt")
pub fn parse_collected(text: &str) -> Option<(u32, String)> {
    let rest = text.strip_prefix("Collected `w")?;
    let (amount, rest) = rest.split_once(' ')?;
    let name = rest.split("``").next()?.trim();
    Some((amount.parse().ok()?, name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_state() {
        let mut d = vec![1];
        d.extend_from_slice(&36u32.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&[242, 0, 23, 0]);
        d.extend_from_slice(&[0x04, 0x07, 3, 0]); // 1796
        let inv = Inventory::parse(&d).unwrap();
        assert_eq!((inv.slots, inv.count(242), inv.count(1796), inv.count(7188)), (36, 23, 3, 0));
    }

    #[test]
    fn updates() {
        let mut inv = Inventory::default();
        inv.add(242, 5);
        inv.remove(242, 2);
        assert_eq!(inv.count(242), 3);
        inv.remove(242, 9);
        assert_eq!(inv.len(), 0);
    }

    #[test]
    fn collected_line() {
        assert_eq!(parse_collected("Collected `w18 World Lock``.``"), Some((18, "World Lock".into())));
        assert_eq!(
            parse_collected("Collected `w172 Paint Bucket - Purple``. Rarity: `w39``"),
            Some((172, "Paint Bucket - Purple".into()))
        );
        assert_eq!(parse_collected("Collected 5 gems"), None);
    }
}
