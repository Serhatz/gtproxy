//! Going to another world.
//!
//! `/warp` is ours and never reaches the server. We leave the world and send
//! `join_request` the moment the world menu arrives (about 0.4 s in total;
//! the old fixed 1.5 s wait is gone). `/back`, `/list`, `/save`, `/relog`
//! and the vend finder go the same way. A short cooldown keeps it from being
//! spammed.

use std::time::{Duration, Instant};

use super::commands::{say, warn};
use super::effect::Effect;
use super::interceptor::Interceptor;

/// Minimum time between two warps.
pub const COOLDOWN: Duration = Duration::from_secs(3);
/// Give up waiting for the world menu after this.
const MENU_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
pub struct Warp {
    /// We left; join this world when the world menu comes.
    join_on_menu: Option<(String, Instant)>,
    last: Option<Instant>,
}

impl Warp {
    /// World we are about to join, for tests.
    #[cfg(test)]
    pub fn pending(&self) -> Option<&str> {
        self.join_on_menu.as_ref().map(|(w, _)| w.as_str())
    }
}

/// `/warp <world>`.
pub fn warp(ic: &mut Interceptor, args: &[&str]) {
    let Some(world) = args.first() else {
        warn(ic, format!("Usage: {}warp <world>", ic.cfg.command.prefix));
        return;
    };
    let world = world.to_ascii_uppercase();
    if world.len() > 23 || !world.chars().all(|c| c.is_ascii_alphanumeric()) {
        warn(ic, "Invalid world name (letters and digits, max 23)");
        return;
    }
    if world == "EXIT" {
        warn(ic, "Use the exit door, or /exit");
        return;
    }
    if ic.warp.last.is_none_or(|t| t.elapsed() >= COOLDOWN) {
        say(ic, format!("Warping to `w{world}``..."));
    }
    go_to(ic, &world);
}

/// Goes to `world` (`NAME` or `NAME|DOOR`): leave, then join as soon as the
/// world menu arrives.
pub fn go_to(ic: &mut Interceptor, world: &str) {
    let world = world.to_ascii_uppercase();
    if let Some(t) = ic.warp.last {
        let left = COOLDOWN.saturating_sub(t.elapsed());
        if !left.is_zero() {
            warn(ic, format!("Wait {:.1} s before warping again", left.as_secs_f32()));
            return;
        }
    }
    ic.warp.last = Some(Instant::now());
    if ic.session.world.is_some() {
        ic.emit(Effect::quit_to_exit());
        ic.warp.join_on_menu = Some((world, Instant::now()));
    } else {
        ic.emit(Effect::join_world(&world));
    }
}

/// `OnRequestWorldSelectMenu`: we are out, join right away if we meant to.
pub fn on_world_menu(ic: &mut Interceptor) {
    if let Some((world, at)) = ic.warp.join_on_menu.take() {
        if at.elapsed() <= MENU_TIMEOUT {
            ic.emit(Effect::join_world(&world));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::protocol::message::encode_text;
    use crate::protocol::{MessageType, Variant};
    use crate::proxy::Direction;
    use crate::proxy::interceptor::Verdict;
    use std::sync::Arc;

    fn ic() -> Interceptor {
        let mut cfg = Config::default();
        cfg.log.text = false;
        cfg.log.variant = false;
        let mut ic = Interceptor::new(Arc::new(cfg));
        ic.session.enter_world("START".into());
        ic
    }

    fn to_server(effects: &[Effect]) -> Vec<String> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::ToServer(b) => Some(String::from_utf8_lossy(&b[4..]).trim_end_matches('\0').to_string()),
                _ => None,
            })
            .collect()
    }

    fn world_menu(ic: &mut Interceptor) {
        let call = Effect::client_call(-1, vec![Variant::String("OnRequestWorldSelectMenu".into()), Variant::String(String::new())]);
        let Effect::ToClient(bytes) = call else { panic!() };
        ic.handle(Direction::ToClient, &bytes);
    }

    #[test]
    fn typed_warp_never_reaches_the_server() {
        let mut ic = ic();
        let typed = encode_text(MessageType::GenericText, "action|input\n|text|/warp buyall");
        assert_eq!(ic.handle(Direction::ToServer, &typed), Verdict::Drop);
        assert_eq!(to_server(&ic.take_effects()), vec!["action|quit_to_exit\n"]);
        // Joined the moment the menu shows up, no fixed wait.
        world_menu(&mut ic);
        assert_eq!(to_server(&ic.take_effects()), vec!["action|join_request\nname|BUYALL\ninvitedWorld|0\n"]);
    }

    #[test]
    fn short_cooldown() {
        let mut ic = ic();
        go_to(&mut ic, "A");
        world_menu(&mut ic);
        ic.take_effects();
        ic.session.enter_world("A".into());
        go_to(&mut ic, "B");
        assert!(to_server(&ic.take_effects()).is_empty());
        ic.warp.last = Some(Instant::now() - COOLDOWN);
        go_to(&mut ic, "B");
        assert_eq!(to_server(&ic.take_effects()), vec!["action|quit_to_exit\n"]);
    }

    #[test]
    fn from_the_menu_joins_at_once() {
        let mut ic = ic();
        ic.session.leave_world();
        go_to(&mut ic, "shop|door1");
        assert_eq!(to_server(&ic.take_effects()), vec!["action|join_request\nname|SHOP|DOOR1\ninvitedWorld|0\n"]);
    }

    #[test]
    fn bad_names_do_nothing() {
        let mut ic = ic();
        warp(&mut ic, &["a-b"]);
        assert!(to_server(&ic.take_effects()).is_empty());
    }
}
