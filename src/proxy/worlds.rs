//! Saved worlds: `/sworld` and `/list`.

use super::commands::{go_to, say, warn};
use super::dialog::{DEFAULT_ICON, Dialog, DialogReturn};
use super::features::SavedWorld;
use super::interceptor::Interceptor;
use super::tools::show;

/// `/sworld`, `/sworld :DOOR`, `/sworld some text`, `/sworld :DOOR some text`.
pub fn save_world(ic: &mut Interceptor, args: &[&str]) {
    let Some(name) = ic.session.world.clone() else {
        warn(ic, "enter a world first");
        return;
    };
    let (door, rest) = match args.first() {
        Some(a) if a.starts_with(':') && a.len() > 1 => (Some(a[1..].to_ascii_uppercase()), &args[1..]),
        _ => (None, args),
    };
    let description = rest.join(" ");
    let worlds = &mut ic.features.saved_worlds;
    match worlds.iter_mut().find(|w| w.name == name && w.door == door) {
        Some(w) => w.description = description,
        None => worlds.push(SavedWorld { name, door, description }),
    }
    let n = worlds.len();
    say(ic, format!("world saved ({n} in {}list)", ic.cfg.command.prefix));
}

/// `/list` opens the window, `/list 3` goes to the third world.
pub fn list(ic: &mut Interceptor, args: &[&str]) {
    match args.first().map(|a| a.parse::<usize>()) {
        None => list_dialog(ic),
        Some(Ok(n)) if (1..=ic.features.saved_worlds.len()).contains(&n) => go(ic, n - 1),
        _ => warn(ic, format!("no world with that number, see {}list", ic.cfg.command.prefix)),
    }
}

fn go(ic: &mut Interceptor, index: usize) {
    let target = ic.features.saved_worlds[index].target();
    say(ic, format!("warping to `w{target}``..."));
    go_to(ic, &target);
}

fn list_dialog(ic: &mut Interceptor) {
    let p = ic.cfg.command.prefix;
    let mut d = Dialog::new("worlds")
        .title("Saved worlds", DEFAULT_ICON)
        .small(&format!("Save the world you are in with {p}sworld [:door] [note]. {p}list <number> warps directly."))
        .spacer();
    if ic.features.saved_worlds.is_empty() {
        d = d.small("No saved worlds yet.");
    }
    for (i, w) in ic.features.saved_worlds.iter().enumerate() {
        let note = if w.description.is_empty() { String::new() } else { format!(" `s- {}``", w.description) };
        d = d
            .button(&format!("go_{i}"), &format!("{}. `w{}``{note}", i + 1, w.target()))
            .button(&format!("del_{i}"), &format!("`4Remove`` {}", w.target()));
    }
    show(ic, d, "Close", "");
}

pub fn on_dialog(ic: &mut Interceptor, ret: &DialogReturn) -> bool {
    if ret.name != "worlds" {
        return false;
    }
    let Some(button) = ret.button else { return true };
    let index = |prefix: &str| button.strip_prefix(prefix).and_then(|v| v.parse::<usize>().ok());
    if let Some(i) = index("go_").filter(|&i| i < ic.features.saved_worlds.len()) {
        go(ic, i);
    } else if let Some(i) = index("del_").filter(|&i| i < ic.features.saved_worlds.len()) {
        ic.features.saved_worlds.remove(i);
        list_dialog(ic);
    }
    true
}
