//! `/dlocal`: a Dirt drop that exists only on your screen.
//!
//! The client numbers dropped objects itself (last uid from the map, +1 per
//! `ItemChangeObject` drop). A drop we inject takes a number the server never
//! assigned, so from then on the client's numbers run ahead of the server's.
//! We translate object uids both ways so real items keep working, and swallow
//! pickups of the fake ones so the server never hears of them.

use crate::protocol::{GamePacketType, GameUpdatePacket, Message};

use super::commands::{say, warn};
use super::effect::Effect;
use super::interceptor::{Interceptor, Verdict};

const DIRT: u32 = 2;
/// `net_id` of an `ItemChangeObject` that drops a new object.
const NEW_OBJECT: i32 = -1;
/// A player that does not exist, used to make fakes disappear without anyone
/// getting them into their inventory.
const NOBODY: i32 = 0x7fff_fff0;

#[derive(Debug, Default)]
pub struct FakeDrops {
    /// The uid the client will give the next dropped object; `None` if unknown.
    next_uid: Option<u32>,
    /// Client-side uids of our fakes, ascending.
    fakes: Vec<u32>,
}

impl FakeDrops {
    pub fn reset(&mut self, last_uid: Option<u32>) {
        self.next_uid = last_uid.map(|u| u + 1);
        self.fakes.clear();
    }

    /// The server dropped a real object.
    fn real_drop(&mut self) {
        if let Some(n) = self.next_uid.as_mut() {
            *n += 1;
        }
    }

    fn add_fake(&mut self) -> Option<u32> {
        let uid = self.next_uid?;
        self.next_uid = Some(uid + 1);
        self.fakes.push(uid);
        Some(uid)
    }

    pub fn is_fake(&self, client_uid: u32) -> bool {
        self.fakes.binary_search(&client_uid).is_ok()
    }

    pub fn to_server(&self, client_uid: u32) -> u32 {
        client_uid - self.fakes.iter().filter(|&&f| f < client_uid).count() as u32
    }

    pub fn to_client(&self, server_uid: u32) -> u32 {
        let mut uid = server_uid;
        for &f in &self.fakes {
            if f <= uid {
                uid += 1;
            }
        }
        uid
    }
}

// ── Packet hooks ─────────────────────────────────────────────────────────────

/// Server -> client `ItemChangeObject`.
pub fn on_server_object(ic: &mut Interceptor, mut pkt: GameUpdatePacket) -> Verdict {
    super::history::on_object(ic, &pkt);
    super::scan::track_object(ic, &pkt);
    if pkt.net_id == NEW_OBJECT {
        ic.fake_drops.real_drop();
        return Verdict::Forward;
    }
    // Picked up by someone: `value` is the object's uid.
    if pkt.net_id > 0 && !ic.fake_drops.fakes.is_empty() {
        let client_uid = ic.fake_drops.to_client(pkt.value);
        if client_uid != pkt.value {
            pkt.value = client_uid;
            return Verdict::Replace(Message::Game(pkt).encode());
        }
    }
    Verdict::Forward
}

/// Client -> server `ItemActivateObjectRequest` (picking an object up).
pub fn on_client_pickup(ic: &mut Interceptor, mut pkt: GameUpdatePacket) -> Verdict {
    if ic.fake_drops.fakes.is_empty() {
        return Verdict::Forward;
    }
    if ic.fake_drops.is_fake(pkt.value) {
        return Verdict::Drop;
    }
    let server_uid = ic.fake_drops.to_server(pkt.value);
    if server_uid == pkt.value {
        return Verdict::Forward;
    }
    pkt.value = server_uid;
    Verdict::Replace(Message::Game(pkt).encode())
}

// ── Command ──────────────────────────────────────────────────────────────────

/// `/dlocal [amount]` drops fake Dirt in front of you; `/dlocal clear` removes them.
pub fn dlocal(ic: &mut Interceptor, args: &[&str]) {
    if args.first() == Some(&"clear") {
        let fakes = std::mem::take(&mut ic.fake_drops.fakes);
        for uid in &fakes {
            let gone = GameUpdatePacket {
                packet_type: GamePacketType::ItemChangeObject,
                net_id: NOBODY,
                value: *uid,
                ..Default::default()
            };
            ic.emit(Effect::ToClient(Message::Game(gone).encode()));
        }
        // The client's numbering stays ahead, so keep translating for the removed ones.
        ic.fake_drops.fakes = fakes;
        say(ic, "Local drops removed");
        return;
    }
    let amount = match args.first().map(|a| a.parse::<u32>()) {
        None => 1,
        Some(Ok(n)) if (1..=200).contains(&n) => n,
        _ => return warn(ic, format!("usage: {}dlocal [1-200] or {}dlocal clear", ic.cfg.command.prefix, ic.cfg.command.prefix)),
    };
    let Some(pos) = ic.session.pos else {
        return warn(ic, "Move a little first so the proxy knows where you are");
    };
    let Some(uid) = ic.fake_drops.add_fake() else {
        return warn(ic, "Object numbers of this world are unknown; re-enter the world");
    };
    let drop = GameUpdatePacket {
        packet_type: GamePacketType::ItemChangeObject,
        net_id: NEW_OBJECT,
        value: DIRT,
        vec_x: pos.0 + 24.0,
        vec_y: pos.1 + 8.0,
        float_var: amount as f32,
        ..Default::default()
    };
    ic.emit(Effect::ToClient(Message::Game(drop).encode()));
    tracing::info!("local drop: {amount} Dirt as uid {uid}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uid_translation() {
        let mut f = FakeDrops::default();
        f.reset(Some(9)); // next uid 10
        f.real_drop(); // real 10
        assert_eq!(f.add_fake(), Some(11)); // fake 11
        f.real_drop(); // real: server 11, client 12
        assert_eq!(f.to_server(10), 10);
        assert_eq!(f.to_server(12), 11);
        assert_eq!(f.to_client(10), 10);
        assert_eq!(f.to_client(11), 12);
        assert!(f.is_fake(11));
        assert!(!f.is_fake(12));
    }

    #[test]
    fn unknown_numbering_refuses() {
        let mut f = FakeDrops::default();
        f.reset(None);
        assert_eq!(f.add_fake(), None);
    }
}
