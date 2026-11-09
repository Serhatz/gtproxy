//! Packet logging with runtime toggles (`/log`).

use crate::config::LogConfig;
use crate::protocol::game_packet::PacketFlags;
use crate::protocol::{GamePacketType, GameUpdatePacket, TextPacket, VariantList};

use super::Direction;

/// Login / redirect fields that identify the account or device.
const SENSITIVE_KEYS: &[&str] = &[
    "ltoken", "token", "tankIDPass", "UUIDToken", "klv", "hash", "hash2", "fhash", "mac", "rid", "wk",
    "meta", "zf", "fz", "doorID", "user", "aat",
];

#[derive(Debug, Clone)]
pub struct PacketLog {
    pub text: bool,
    pub game_packet: bool,
    pub variant: bool,
    pub skip_spam: bool,
    pub redact: bool,
}

impl From<&LogConfig> for PacketLog {
    fn from(c: &LogConfig) -> Self {
        Self {
            text: c.text,
            game_packet: c.game_packet,
            variant: c.variant,
            skip_spam: c.skip_spam,
            redact: c.redact_login,
        }
    }
}

impl PacketLog {
    pub fn text(&self, dir: Direction, kind: &str, text: &str) {
        if !self.text {
            return;
        }
        let shown = if self.redact { redact(text) } else { text.to_string() };
        tracing::info!("{} [{kind}]\n{}", dir.arrow(), shown.trim_end());
    }

    pub fn game(&self, dir: Direction, pkt: &GameUpdatePacket) {
        if !self.game_packet {
            return;
        }
        // Plain movement and pings are noise; a State that punches/places is a click, keep it.
        let click = pkt.flags.intersects(PacketFlags::PUNCH | PacketFlags::PLACE);
        if self.skip_spam
            && !click
            && matches!(
                pkt.packet_type,
                GamePacketType::State | GamePacketType::PingRequest | GamePacketType::PingReply
            )
        {
            return;
        }
        tracing::info!(
            "{} [game] {:?} net_id={} target={} flags={:?} value={} pos=({:.0},{:.0}) speed=({:.0},{:.0}) tile=({},{}) \
             obj={} jump={} anim={} float={} rot={} extra={}B",
            dir.arrow(),
            pkt.packet_type,
            pkt.net_id,
            pkt.target_net_id,
            pkt.flags,
            pkt.value,
            pkt.vec_x,
            pkt.vec_y,
            pkt.vec_x2,
            pkt.vec_y2,
            pkt.int_x,
            pkt.int_y,
            pkt.object_type,
            pkt.jump_count,
            pkt.animation_type,
            pkt.float_var,
            pkt.particle_rotation,
            pkt.extra.len()
        );
    }

    pub fn variant(&self, dir: Direction, vl: &VariantList) {
        if !self.variant {
            return;
        }
        let mut s = vl.to_string();
        if self.redact && vl.function() == Some("OnSendToServer") {
            s = format!("[0] \"OnSendToServer\" (redacted, {} args)", vl.0.len());
        }
        tracing::info!("{} [call] {s}", dir.arrow());
    }
}

pub fn redact(text: &str) -> String {
    let tp = TextPacket::parse(text);
    if !tp.lines().iter().any(|(k, _)| SENSITIVE_KEYS.contains(&k.as_str())) {
        return text.to_string();
    }
    let mut out = String::new();
    for (k, v) in tp.lines() {
        out.push_str(k);
        if let Some(v) = v {
            out.push('|');
            if SENSITIVE_KEYS.contains(&k.as_str()) && !v.is_empty() {
                out.push_str("<redacted>");
            } else {
                out.push_str(v);
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_login() {
        let r = redact("tankIDName|abc\nltoken|SECRET\nmac|02:00\nprotocol|225\n");
        assert!(!r.contains("SECRET"));
        assert!(r.contains("tankIDName|abc"));
        assert!(r.contains("protocol|225"));
    }
}
