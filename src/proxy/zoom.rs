//! Unlimited zoom.
//!
//! The game itself lets moderators zoom out without a limit; whether we are
//! one comes from `mstate` in our own `OnSpawn`. With the feature on we set
//! `mstate|1` on the way to the client only, so the game's own wheel zoom
//! stays smooth and simply has no limit. The server is never told anything.
//! `OnSpawn` comes on world entry, so a change applies from the next world.
//!
//! `/zoom <level>` sets a level directly with `OnZoomCamera` (0.1 = furthest).

use std::sync::atomic::{AtomicBool, Ordering};

use crate::protocol::{TextPacket, Variant};

use super::commands::{say, warn};
use super::effect::Effect;
use super::interceptor::Interceptor;

pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 4.0;

/// On by default.
static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    ENABLED.store(on, Ordering::Relaxed);
    Ok(())
}

/// Our own spawn text: marks us as a moderator for the client, which lifts the zoom limit.
pub fn rewrite_spawn(spawn: &mut TextPacket) -> bool {
    if !enabled() || spawn.get("mstate") == Some("1") {
        return false;
    }
    spawn.set("mstate", "1");
    true
}

/// `/zoom` toggles unlimited zoom, `/zoom <level>` jumps to a level.
pub fn zoom(ic: &mut Interceptor, args: &[&str]) {
    let p = ic.cfg.command.prefix.clone();
    match args.first() {
        None => {
            let on = !enabled();
            let _ = set_enabled(on);
            if on {
                say(ic, format!("Unlimited zoom `2on``, works from the next world you enter ({p}relog to apply now)"));
            } else {
                say(ic, format!("Unlimited zoom `4off`` from the next world you enter ({p}relog to apply now)"));
            }
        }
        Some(v) => match v.parse::<f32>() {
            Ok(l) if (MIN_ZOOM..=MAX_ZOOM).contains(&l) => {
                ic.emit(Effect::client_call(
                    -1,
                    vec![Variant::String("OnZoomCamera".into()), Variant::Float(l), Variant::Signed(500)],
                ));
                say(ic, format!("Zoom `w{l}``"));
            }
            _ => warn(ic, format!("Usage: {p}zoom  or  {p}zoom <{MIN_ZOOM}-{MAX_ZOOM}>")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_mod_state_only_when_on() {
        let text = "spawn|avatar\nnetID|2\nmstate|0\nsmstate|0\ntype|local";
        let mut spawn = TextPacket::parse(text);
        set_enabled(false).unwrap();
        assert!(!rewrite_spawn(&mut spawn));
        set_enabled(true).unwrap();
        assert!(rewrite_spawn(&mut spawn));
        assert_eq!(spawn.get("mstate"), Some("1"));
        assert_eq!(spawn.get("smstate"), Some("0"));
    }
}
