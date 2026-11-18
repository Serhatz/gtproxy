//! `/autosurg`: surgery help, chosen in its window.
//!
//! - **Assisted**: every Surg-E window (`end_dialog|surgery`) is read and the
//!   tool that fits best gets a yellow frame; you press it yourself.
//! - **Automatic**: the window is not shown; after the set delay the proxy
//!   answers it with the best tool (`action|dialog_return` /
//!   `dialog_name|surgery` / `buttonClicked|tool<ID>`, as the game sends it).
//!   This is botting by the game's rules and can get an account banned, so
//!   it is off at every start and says so. A failed surgery does not stop it.
//!   It stops when no tool fits (the window is shown instead), when the
//!   server does not answer a choice, on Malpractice without Legal Briefs and
//!   when no unused Surg-E is in reach.
//! - **Next Surg-E** (automatic): when a surgery ends the nearest unused
//!   Surg-E Anatomical Dummy in reach is wrenched (`TileChangeRequest`, item
//!   32, as the client sends it) and its "SurgeryPrePopup" (`end_dialog|surge`)
//!   is answered like the Okay button (`dialog_name|surge` / `tilex` / `tiley`).
//! - **Legal Briefs** (automatic): on Malpractice the briefs (3172) are worn,
//!   which removes the mod ("Your briefs have stunned the judge!"), and taken
//!   off again when it is gone. Using an item toggles it, so briefs that are
//!   already worn are first taken off, then worn (logged: worn briefs were
//!   only taken off and the Malpractice stayed).
//! - Any tool under 20 in a new surgery's window: a warning.
//! - **Tea and Gemonade** (automatic, when in the inventory): Tea gives
//!   "Calm nerves." (fewer skill fails, 2 minutes), Gemonade "Food: Sparkling
//!   Gems". They are eaten (placed on the player's own tile, like dragging
//!   them onto yourself) when the mod is off; Gemonade not while any other
//!   "Food:" mod is on (one food at a time, e.g. Skill Spice).
//!
//! The first surgery is started by hand (wrench the Surg-E, press Okay).
//! Turning it off during an automatic surgery finishes that surgery first.
//!
//! The window lists the patient's state in small text lines, e.g.
//! `Pulse: `2Strong``    Status: `3Awake``` / `Temp: `3101.6``    Operation
//! site: `6Unclean``` / `Incisions: `20``` / `Patient is losing blood `3slowly.```,
//! and one button per tool: `add_button_with_icon|tool1264|`$Antiseptic``|noflags|1264|`.
//!
//! Rules: the Growtopia Wiki "Surgery" page (success needs the problem fixed,
//! no broken bones, no incisions, no bleeding, temperature 100.4 or less and a
//! beating heart; Stitches work without incisions, Clamp and Pins need one,
//! Antibiotics appear only after the Lab Kit, flus are cured without cutting),
//! checked against logged surgeries. The first window of a surgery is matched
//! against the start states another proxy ships ([`LIKELY`]) to name the
//! illness and its plan before it is diagnosed. Every surgery window and the
//! tool pressed go to `surgery_log.txt`.

use std::io::Write;
use std::time::{Duration, Instant};

use super::commands::{say, warn};
use super::dialog::{Dialog, DialogReturn};
use super::effect::Effect;
use super::interceptor::Interceptor;
use super::session::strip_colors;

const LOG_FILE: &str = "surgery_log.txt";
/// The log is not kept growing forever.
const LOG_MAX: u64 = 2 * 1024 * 1024;
/// A surgery window this long after the last one belongs to a new surgery.
const NEW_SURGERY_AFTER: Duration = Duration::from_secs(90);
const FIX_IT: &str = "fix it";
pub const DEFAULT_DELAY_MS: u64 = 1200;
/// The player's own call: very short waits look even less human.
pub const MIN_DELAY_MS: u64 = 10;
pub const MAX_DELAY_MS: u64 = 10_000;
/// No new surgery window this long after an automatic choice: hand it over.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(6);
const SAVED: &str = "YOU SAVED YOUR PATIENT";
/// Console lines that mean the surgery went wrong (lowercase).
const FAILED: &[&str] = &[
    "malpractice",
    "not allowed to perform surgery",
    "medical license is revoked",
    "succumbed",
    "died",
    "bled out",
    "permanent coma",
    "lost the patient",
    "abandoned your patient",
    "couldn't save them",
];
/// Sponges in a row: after this many "hard to see" is ignored, or a bleeding
/// patient is mopped forever and never cut (logged: 55 moves, then lost).
const SPONGE_IN_A_ROW: u8 = 2;
/// Surg-E Anatomical Dummy, the block surgeries start on.
const SURGE_ITEM: u16 = 4296;
const WRENCH: u32 = 32;
/// Worn on Malpractice: the mod goes away.
const LEGAL_BRIEFS: u32 = 3172;
/// Fewer tools than this in a new surgery: a warning.
const LOW_SUPPLY: u32 = 20;
/// How far (in tiles) the next Surg-E may be.
const NEXT_REACH: i32 = 5;
/// The next Surg-E did not open or start a surgery this long after its wrench.
const NEXT_TIMEOUT: Duration = Duration::from_secs(4);
/// Pause after a surgery ends before the next Surg-E is wrenched.
const NEXT_PAUSE: Duration = Duration::from_millis(1500);
const TEA: u32 = 5114;
const GEMONADE: u32 = 6914;
/// An eaten item whose mod does not show up is tried this often, then left alone.
const FOOD_TRIES: u8 = 3;
const FOOD_RETRY: Duration = Duration::from_secs(10);
const FOOD_GAP: Duration = Duration::from_millis(700);

#[derive(Debug, Default)]
pub struct Surgery {
    /// "Fix It!" was used in this surgery: now it is only closing up.
    fixed: bool,
    last_seen: Option<Instant>,
    /// Index into [`LIKELY`] from the surgery's first window.
    likely: Option<usize>,
    /// Automatic: when the choice for this window goes out, and the window,
    /// shown to the player if the server does not answer.
    waiting: Option<(Instant, String)>,
    /// Sponges suggested in a row.
    sponge_streak: u8,
    /// This surgery's end was handled.
    ended: bool,
    /// Automatic: when to wrench the next Surg-E.
    next_at: Option<Instant>,
    /// The Surg-E wrenched for the next surgery, and when it times out.
    pending: Option<((i32, i32), Instant)>,
    /// Surg-Es already used in `used_world`.
    used: Vec<(i32, i32)>,
    used_world: String,
    /// When the Legal Briefs were last worn.
    briefs_at: Option<Instant>,
    /// The Legal Briefs are worn now (`OnSetClothing` of the player).
    briefs_worn: bool,
    /// The proxy put them on for a Malpractice: off again when it is gone.
    briefs_ours: bool,
    /// Turned off during an automatic surgery: off once it ends.
    off_after: bool,
    /// The illness of this surgery, for `/surgstats`.
    illness: Option<String>,
    /// Food eaten: item -> (when, tries without its mod showing up).
    food: Vec<(u32, Instant, u8)>,
}

/// What to do with a surgery window.
pub enum Window {
    /// Show this markup (the tool marked, or as it came).
    Show(String),
    /// Automatic mode answered it; the player does not see it.
    Hide,
}

/// Start states (the state lines of the first window) and the illness they
/// belong to, from another proxy's data; two logged surgeries matched them.
/// `(state lines, illness, plan from the Growtopia Wiki)`.
pub const LIKELY: &[(&str, &str, &str)] = &[
    ("Pulse: `2Strong``    Status: `3Awake``|Temp: `3101.6``    Operation site: `6Unclean``|`6It is becoming hard to see your work.``|Incisions: `20``    |Patient's fever is `4climbing rapidly!``",
     "turtle flu", "Lab Kit, then Antibiotics until the fever is gone. No cutting."),
    ("Pulse: `3Steady``    Status: `4Awake``|Temp: `298.6``    Operation site: `6Unclean``|`4You can't see what you are doing!``|Incisions: `30``    |Patient is `6losing blood!``",
     "serious trauma with a punctured lung", "Stitches until not bleeding, Ultrasound, Antiseptic, Anesthetic, cut until Fix It, Fix It, Stitches to one incision, Pins, Stitches, Splints."),
    ("Pulse: `2Strong``    Status: `4Awake``|Temp: `298.6``    Operation site: `3Not sanitized``|Incisions: `30``    ",
     "brain/lung tumor or heart attack", "Ultrasound, Antiseptic, Anesthetic, cut until Fix It (1-5 times), Fix It, Stitches."),
    ("Pulse: `2Strong``    Status: `3Awake``|Temp: `298.6``    Operation site: `3Not sanitized``|Incisions: `20``    |Patient is losing blood `3slowly.``",
     "broken leg/arm", "Ultrasound, Splint. Shattered bone: Anesthetic, 1 cut, Pins, Stitches, Splint."),
    ("Pulse: `2Strong``    Status: `4Awake``|Temp: `3100.4``    Operation site: `3Not sanitized``|Incisions: `30``    ",
     "damaged spine", "Ultrasound, Antiseptic until clean, Anesthetic, cut until Fix It (3), Fix It, Stitches."),
    ("Pulse: `3Steady``    Status: `4Awake``|Temp: `3101.6``    Operation site: `3Not sanitized``|Incisions: `30``    |Patient's fever is `4climbing rapidly!``",
     "kidney failure", "Lab Kit, Antibiotics (about 2), Ultrasound, Anesthetic, cut until Fix It (2), Fix It, Stitches."),
    ("Pulse: `2Strong``    Status: `3Awake``|Temp: `6104.6``    Operation site: `6Unclean``|`6It is becoming hard to see your work.``|Incisions: `20``    |Patient's fever is `4climbing rapidly!``",
     "bird flu", "Lab Kit, then Antibiotics until the fever is gone. No cutting."),
    ("Pulse: `3Steady``    Status: `4Awake``|Temp: `298.6``    Operation site: `4Unsanitary``|`4You can't see what you are doing!``|Incisions: `30``    |Patient is losing blood `4very quickly!``",
     "massive trauma", "Stitches, Ultrasound, Antiseptic, Anesthetic, cut until Fix It (3), Fix It, Stitches to one incision, Pins, Stitches, Splints."),
    ("Pulse: `3Steady``    Status: `4Awake``|Temp: `6104.6``    Operation site: `6Unclean``|Incisions: `30``    |Patient's fever is `6climbing.``",
     "liver infection", "Lab Kit, Antibiotics (about 2), Ultrasound, Anesthetic, cut until Fix It (2), Fix It, Stitches."),
    ("Pulse: `6Weak``    Status: `4Awake``|Temp: `298.6``    Operation site: `6Unclean``|`6It is becoming hard to see your work.``|Incisions: `30``    |Patient is losing blood `4very quickly!``",
     "serious head injury", "Stitches until not bleeding, Ultrasound, Anesthetic, 1 cut, Fix It, Stitches."),
    ("Pulse: `3Steady``    Status: `4Awake``|Temp: `6104.6``    Operation site: `6Unclean``|Incisions: `30``    |Patient's fever is `4climbing rapidly!``",
     "appendicitis", "Lab Kit, Antibiotics (about 2), Ultrasound, Antiseptic until clean, Anesthetic, cut until Fix It (3), Fix It, Stitches."),
    ("Pulse: `2Strong``    Status: `3Awake``|Temp: `6107.6``    Operation site: `6Unclean``|Incisions: `20``    |Patient's fever is `4climbing rapidly!``",
     "monkey flu", "Lab Kit, then Antibiotics until the fever is gone. No cutting."),
    ("Pulse: `3Steady``    Status: `4Awake``|Temp: `3100.58``    Operation site: `3Not sanitized``|`6It is becoming hard to see your work.``|Incisions: `30``    |Patient is losing blood `3slowly.``",
     "ran over by a truck", "Stitches, Ultrasound, Anesthetic, cut until Fix It, Fix It, Stitches to one incision, Pins, Stitches, Splints."),
    ("Pulse: `2Strong``    Status: `4Awake``|Temp: `3101.6``    Operation site: `3Not sanitized``|`6It is becoming hard to see your work.``|Incisions: `30``    |Patient is losing blood `3slowly.``",
     "swallowed world lock", "Ultrasound, Anesthetic, cut until Fix It (2), Fix It, Stitches; mop and clean often."),
];

/// The state lines of a surgery window (`Pulse` up to the spacer), joined with `|`.
fn state_signature(markup: &str) -> String {
    let mut out = Vec::new();
    let mut on = false;
    for line in markup.lines() {
        if line.starts_with("add_smalltext|Pulse:") {
            on = true;
        }
        if !on {
            continue;
        }
        if !line.starts_with("add_smalltext|") {
            break;
        }
        let text = line.trim_start_matches("add_smalltext|");
        out.push(text.strip_suffix("|left|").unwrap_or(text));
    }
    out.join("|")
}

/// The illness a first window looks like.
pub fn likely(markup: &str) -> Option<usize> {
    let sig = state_signature(markup);
    LIKELY.iter().position(|(start, _, _)| *start == sig)
}

#[derive(Debug, Default, PartialEq)]
pub struct State {
    pub heart_stopped: bool,
    pub pulse: Pulse,
    pub status: Status,
    pub temp: Option<f32>,
    pub site_dirty: bool,
    /// The worst state: with an incision or bleeding it causes fever (Wiki).
    pub site_unsanitary: bool,
    /// Unclean or Unsanitary ("Not sanitized" is the mild one).
    pub site_unclean: bool,
    pub incisions: Option<u32>,
    pub bleeding: Bleeding,
    pub fever: Fever,
    pub vision: Vision,
    pub bones: Bones,
    pub diagnosed: bool,
    /// "patient suffers from kidney failure." and the like, lowercase.
    pub diagnosis: String,
    /// `(button id, tool name lowercase)`.
    pub tools: Vec<(String, String)>,
}

#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub enum Pulse {
    #[default]
    Fine,
    Weak,
    ExtremelyWeak,
}

#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub enum Status {
    #[default]
    Unknown,
    Awake,
    ComingTo,
    Unconscious,
}

#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub enum Bleeding {
    #[default]
    None,
    Slow,
    Yes,
    Fast,
}

#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub enum Fever {
    #[default]
    None,
    Climbing,
    Rapid,
}

#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub enum Vision {
    #[default]
    Clear,
    Hard,
    Blind,
}

#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub enum Bones {
    #[default]
    Fine,
    Broken,
    Shattered,
}

/// The number right after `key` in `text` (`"temp: 101.6"` -> 101.6).
fn number_after(text: &str, key: &str) -> Option<f32> {
    let rest = &text[text.find(key)? + key.len()..];
    let n: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    n.parse().ok()
}

/// Reads a surgery window; `None` for any other dialog.
pub fn parse(markup: &str) -> Option<State> {
    if !markup.lines().any(|l| l.starts_with("end_dialog|surgery|")) {
        return None;
    }
    let mut s = State { diagnosed: true, ..Default::default() };
    for line in markup.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.first() == Some(&"add_button_with_icon") {
            if let (Some(id), Some(label)) = (parts.get(1), parts.get(2)) {
                if id.starts_with("tool") {
                    s.tools.push((id.to_string(), strip_colors(label).trim().to_lowercase()));
                }
            }
            continue;
        }
        let text = strip_colors(parts.get(1).copied().unwrap_or_default()).to_lowercase();
        if text.contains("pulse: extremely weak") {
            s.pulse = Pulse::ExtremelyWeak;
        } else if text.contains("pulse: weak") {
            s.pulse = Pulse::Weak;
        }
        if text.contains("heart stopped") || text.contains("heart has stopped") {
            s.heart_stopped = true;
        }
        if text.contains("status: awake") {
            s.status = Status::Awake;
        } else if text.contains("status: coming to") {
            s.status = Status::ComingTo;
        } else if text.contains("status: unconscious") {
            s.status = Status::Unconscious;
        }
        if let Some(t) = number_after(&text, "temp:") {
            s.temp = Some(t);
        }
        if ["site: not sanitized", "site: unclean", "site: unsanitary"].iter().any(|k| text.contains(k)) {
            s.site_dirty = true;
        }
        if text.contains("site: unsanitary") {
            s.site_unsanitary = true;
        }
        if text.contains("site: unclean") || text.contains("site: unsanitary") {
            s.site_unclean = true;
        }
        if let Some(n) = number_after(&text, "incisions:") {
            s.incisions = Some(n as u32);
        }
        if text.contains("losing blood") {
            s.bleeding = if text.contains("very quickly") {
                Bleeding::Fast
            } else if text.contains("slowly") {
                Bleeding::Slow
            } else {
                Bleeding::Yes
            };
        }
        if text.contains("fever is climbing") || text.contains("fever is slowly rising") {
            s.fever = if text.contains("rapidly") { Fever::Rapid } else { Fever::Climbing };
        }
        if text.contains("can't see what you are doing") {
            s.vision = Vision::Blind;
        } else if text.contains("hard to see your work") && s.vision != Vision::Blind {
            s.vision = Vision::Hard;
        }
        if text.contains("bones") && text.contains("shattered") {
            s.bones = Bones::Shattered;
        } else if text.contains("bones") && text.contains("broken") && s.bones != Bones::Shattered {
            s.bones = Bones::Broken;
        }
        if text.contains("has not been diagnosed") {
            s.diagnosed = false;
        }
        if s.diagnosis.is_empty()
            && parts.first() == Some(&"add_smalltext")
            && text.starts_with("patient ")
            && (!text.starts_with("patient is ") || text.starts_with("patient is showing signs"))
            && !text.starts_with("patient's ")
        {
            s.diagnosis = text.clone();
        }
    }
    Some(s)
}

/// The tool to use now: `(button id, tool name, why)`. `fixed`: "Fix It!"
/// worked this surgery; `flu`: the illness is a flu (cured without cutting).
#[cfg(test)]
pub fn recommend(s: &State, fixed: bool, flu: bool) -> Option<(String, String, &'static str)> {
    recommend_with(s, fixed, flu, true)
}

/// [`recommend`]; `sponge_ok`: mopping a "hard to see" site is allowed.
pub fn recommend_with(s: &State, fixed: bool, flu: bool, sponge_ok: bool) -> Option<(String, String, &'static str)> {
    let has = |name: &str| s.tools.iter().find(|(_, n)| n.contains(name)).cloned();
    let open = s.incisions.unwrap_or(0) > 0;
    let awake = matches!(s.status, Status::Awake | Status::ComingTo);
    let bleeding = matches!(s.bleeding, Bleeding::Yes | Bleeding::Fast);
    // Success needs 100.4 or less, so any fever has to go.
    let fever = s.fever != Fever::None || s.temp.is_some_and(|t| t > 100.4);
    let flu = flu || s.diagnosis.contains("flu");
    // Bone-only illnesses ("Patient broke his leg.") have no Fix It.
    let bones_only = s.diagnosis.contains("broke");
    let fix_button = has(FIX_IT).is_some();
    let fix_done = (fixed && !fix_button) || flu || bones_only;
    let shattered = s.bones == Bones::Shattered;
    let incisions = s.incisions.unwrap_or(0);
    // More cutting is needed to reach the problem or a shattered bone.
    let cutting = (!fix_done && !fix_button) || (fix_done && shattered && !open);
    // A fever that kills soon; a mild one (often from a dirty open site) waits.
    let urgent_fever = s.fever == Fever::Rapid || s.temp.is_some_and(|t| t >= 103.0);
    let mild_fever = s.fever != Fever::None;

    let mut wants: Vec<(&str, &'static str)> = Vec::new();
    let mut want = |cond: bool, tool: &'static str, why: &'static str| {
        if cond {
            wants.push((tool, why));
        }
    };
    // 1. Life. Blind: the game only offers the Sponge anyway.
    want(s.vision == Vision::Blind, "sponge", "you can't see the operation site");
    want(s.heart_stopped, "defibrillator", "the heart has stopped (2 moves to restart it)");
    // "Hard to see" and any other tool: the next window is blind about half the
    // time when bleeding (only the Sponge is offered then, and a stopped heart
    // misses its Defibrillator: most logged deaths); a Sponge first: 4-5 %.
    // Still before that: a fever about to kill (110), fast bleeding on a weak pulse.
    let deadly_fever = s.temp.is_some_and(|t| t >= 107.0);
    want(deadly_fever, "antibiotics", "the fever is close to deadly");
    want(deadly_fever, "lab kit", "the fever is close to deadly; the Lab Kit unlocks Antibiotics");
    want(s.bleeding == Bleeding::Fast && s.pulse != Pulse::Fine, "stitches", "bleeding fast on a weak pulse");
    want(s.pulse == Pulse::ExtremelyWeak, "transfusion", "the pulse is extremely weak");
    want(sponge_ok && s.vision == Vision::Hard, "sponge", "it is becoming hard to see");
    // Closing the only incision stops all bleeding at once: better than blood.
    want(bleeding && incisions == 1, "stitches", "closing the only incision stops all the bleeding");
    want(s.pulse == Pulse::ExtremelyWeak, "transfusion", "the pulse is extremely weak");
    // An awake patient with open incisions flails, bleeds and blinds the view
    // (two logged surgeries bled out that way): put it back to sleep first.
    want(awake && open, "anesthetic", "the patient is awake with open incisions");
    // 2. Bleeding: Stitches right away (Clamp and Antiseptic are not needed).
    want(bleeding && open, "stitches", "close an incision to stop the bleeding");
    want(bleeding, "stitches", "the patient is losing blood");
    // 3. A fever about to kill: Antibiotics show up only after the Lab Kit.
    want(urgent_fever, "antibiotics", "the fever is high or climbing fast");
    want(urgent_fever, "lab kit", "the fever is high; the Lab Kit unlocks Antibiotics");
    // 5. Diagnosis.
    want(!s.diagnosed && flu, "lab kit", "the patient has not been diagnosed (a flu)");
    want(!s.diagnosed, "ultrasound", "the patient has not been diagnosed");
    want(!s.diagnosed, "lab kit", "the patient has not been diagnosed");
    want(s.pulse == Pulse::Weak && (open || cutting), "transfusion", "the pulse is weak");
    // 7. The fix itself (pressed again if it failed).
    want(fix_button && awake, "anesthetic", "put the patient to sleep before fixing");
    want(fix_button, FIX_IT, "the problem can be fixed now");
    // 8. Bones and closing up: stitch down to one incision, Pins, close, Splint.
    want(fix_done && shattered && incisions > 1, "stitches", "close down to one incision before the Pins");
    want(fix_done && shattered && incisions == 1, "pins", "a bone is shattered");
    want(fix_done && open && !shattered, "stitches", "close the incisions");
    want(s.bones == Bones::Broken && !open, "splint", "a bone is broken");
    // 9. The mild fever.
    want(mild_fever, "antibiotics", "the fever is climbing");
    want(mild_fever, "lab kit", "the fever is climbing; the Lab Kit unlocks Antibiotics");
    // 10. Cutting, asleep.
    want(cutting && awake, "anesthetic", "never cut an awake patient");
    want(cutting, "scalpel", "cut until the Fix It button shows up");
    // 11. Finishing needs 100.4 or less, and no bleeding.
    want(fever, "antibiotics", "the temperature must be 100.4 or less to finish");
    want(fever, "lab kit", "the temperature must be 100.4 or less; the Lab Kit unlocks Antibiotics");
    want(s.bleeding == Bleeding::Slow, "stitches", "the patient is still bleeding a little");
    want(s.bones == Bones::Broken, "splint", "a bone is broken");
    for (tool, why) in wants {
        if let Some((id, name)) = has(tool) {
            return Some((id, name, why));
        }
    }
    None
}

/// Title case for a lowercase tool name ("lab kit" -> "Lab Kit").
fn title(name: &str) -> String {
    name.split(' ')
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Server surgery window: `None` leaves it as it is.
pub fn on_server_dialog(ic: &mut Interceptor, markup: &str) -> Option<Window> {
    let state = parse(markup)?;
    let now = Instant::now();
    let new_surgery = ic.surgery.last_seen.is_none_or(|t| now.duration_since(t) > NEW_SURGERY_AFTER)
        || strip_colors(markup).contains("prepped for surgery");
    if new_surgery {
        ic.surgery.fixed = false;
        ic.surgery.likely = likely(markup);
        ic.surgery.sponge_streak = 0;
        ic.surgery.ended = false;
        ic.surgery.illness = None;
    }
    if !state.diagnosis.is_empty() {
        ic.surgery.illness = Some(super::surgstats::illness_name(&state.diagnosis));
    }
    super::surgstats::on_window(ic, markup);
    ic.surgery.pending = None;
    ic.surgery.next_at = None;
    // "Fix It!" failed: the button is back.
    if state.tools.iter().any(|(_, n)| n.contains(FIX_IT)) {
        ic.surgery.fixed = false;
    }
    ic.surgery.last_seen = Some(now);
    ic.surgery.waiting = None;
    let auto = ic.features.surgery_auto;
    log(&format!("--- window (fixed: {}, tutor: {}, auto: {auto})\n{markup}\n", ic.surgery.fixed, ic.features.surgery_tips));
    if !ic.features.surgery_tips && !auto {
        return None;
    }
    if new_surgery {
        low_supply(ic, markup);
    }
    if auto {
        eat(ic);
    }
    let flu = ic.surgery.likely.is_some_and(|i| LIKELY[i].1.ends_with("flu"));
    let sponge_ok = ic.surgery.sponge_streak < SPONGE_IN_A_ROW;
    let Some((id, name, why)) = recommend_with(&state, ic.surgery.fixed, flu, sponge_ok) else {
        if auto {
            warn(ic, "Auto surgery: no tool fits this, your turn");
        }
        return None;
    };
    log(&format!("--- tip {}: {why}\n", title(&name)));
    ic.surgery.sponge_streak = if name.contains("sponge") { ic.surgery.sponge_streak.saturating_add(1) } else { 0 };
    if !auto {
        // Only the yellow frame: no text is added to the game's window.
        return Some(Window::Show(mark_button(markup, &id)));
    }
    let delay = jittered(Duration::from_millis(ic.features.surgery_delay_ms));
    ic.emit(Effect::text(format!("action|dialog_return\ndialog_name|surgery\nbuttonClicked|{id}\n")).later(delay));
    on_client_choice(ic, &id);
    ic.surgery.waiting = Some((now + delay, mark_button(markup, &id)));
    Some(Window::Hide)
}

/// The delay, give or take 15 %, so the clicks are not machine-even.
fn jittered(base: Duration) -> Duration {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    let factor = 0.85 + (nanos % 3001) as f64 / 10_000.0;
    base.mul_f64(factor)
}

/// Tool counts in a surgery window: `(name, count)`.
fn tool_counts(markup: &str) -> Vec<(String, u32)> {
    markup
        .lines()
        .filter_map(|line| {
            let p: Vec<&str> = line.split('|').collect();
            if p.first() != Some(&"add_button_with_icon") || !p.get(1).is_some_and(|id| id.starts_with("tool")) {
                return None;
            }
            let count = p.get(5)?.trim().parse().ok()?;
            Some((strip_colors(p.get(2)?), count))
        })
        .collect()
}

/// A new surgery with a tool under [`LOW_SUPPLY`]: a warning.
fn low_supply(ic: &mut Interceptor, markup: &str) {
    let low: Vec<String> = tool_counts(markup)
        .into_iter()
        .filter(|(name, n)| *n < LOW_SUPPLY && !name.to_lowercase().contains(FIX_IT))
        .map(|(name, n)| format!("{name} {n}"))
        .collect();
    if !low.is_empty() {
        warn(ic, format!("Low supplies: {}", low.join(", ")));
    }
}

/// Called every bridge loop: the server did not answer an automatic choice,
/// or it is time for the next Surg-E.
pub fn tick(ic: &mut Interceptor, now: Instant) {
    if ic.surgery.next_at.is_some_and(|at| now >= at) {
        ic.surgery.next_at = None;
        start_next(ic, now);
    }
    if ic.surgery.pending.is_some_and(|(_, until)| now >= until + NEXT_TIMEOUT) {
        // That Surg-E did not open or start: try another one.
        ic.surgery.pending = None;
        log("--- next surg-e did not start\n");
        start_next(ic, now);
    }
    let Some((sent, _)) = &ic.surgery.waiting else { return };
    if now < *sent + ANSWER_TIMEOUT {
        return;
    }
    let Some((_, markup)) = ic.surgery.waiting.take() else { return };
    ic.features.surgery_auto = false;
    let shown = ic.style_dialog(markup);
    ic.emit(Effect::dialog(shown));
    warn(ic, "Auto surgery stopped: no answer from the server, the window is yours");
}

/// After a surgery: the next Surg-E soon, when automatic.
fn schedule_next(ic: &mut Interceptor) {
    if std::mem::take(&mut ic.surgery.off_after) {
        if ic.features.surgery_auto {
            set_mode(ic, "off");
        }
        return;
    }
    if ic.features.surgery_auto && ic.features.surgery_next {
        let wait = NEXT_PAUSE.max(Duration::from_millis(ic.features.surgery_delay_ms));
        ic.surgery.next_at = Some(Instant::now() + wait);
    }
}

/// The nearest unused Surg-E within [`NEXT_REACH`] tiles of the player.
fn next_surge(ic: &mut Interceptor) -> Option<(i32, i32)> {
    let map = ic.world.as_ref()?;
    if ic.surgery.used_world != map.name {
        ic.surgery.used.clear();
        ic.surgery.used_world = map.name.clone();
    }
    let (px, py) = ic.session.pos?;
    let me = (((px + 10.0) / 32.0).floor() as i32, ((py + 15.0) / 32.0).floor() as i32);
    let item = ic.items.find("Surg-E Anatomical Dummy").and_then(|i| u16::try_from(i).ok()).unwrap_or(SURGE_ITEM);
    let used = &ic.surgery.used;
    map.tiles
        .iter()
        .enumerate()
        .filter(|(_, t)| t.fg == item)
        .map(|(i, _)| map.xy(i))
        .filter(|p| !used.contains(p))
        .map(|p| ((p.0 - me.0).abs().max((p.1 - me.1).abs()), p))
        .filter(|(d, _)| *d <= NEXT_REACH)
        .min()
        .map(|(_, p)| p)
}

/// Wrenches the next Surg-E, or stops when there is none.
fn start_next(ic: &mut Interceptor, now: Instant) {
    if !ic.features.surgery_auto || !ic.features.surgery_next {
        return;
    }
    // Eat first; the wrench follows once the food is in.
    let wait = eat(ic);
    if !wait.is_zero() {
        ic.surgery.next_at = Some(now + wait);
        return;
    }
    let Some((x, y)) = next_surge(ic) else {
        ic.features.surgery_auto = false;
        warn(ic, "No other Surg-E in reach: auto surgery stopped");
        return;
    };
    ic.surgery.used.push((x, y));
    ic.surgery.pending = Some(((x, y), now));
    let (px, py) = ic.session.pos.unwrap_or_default();
    let pkt = crate::protocol::GameUpdatePacket {
        packet_type: crate::protocol::GamePacketType::TileChangeRequest,
        value: WRENCH,
        vec_x: px,
        vec_y: py,
        int_x: x,
        int_y: y,
        ..Default::default()
    };
    ic.emit(Effect::ToServer(crate::protocol::Message::Game(pkt).encode()));
    log(&format!("--- next surg-e {x},{y}\n"));
    say(ic, format!("Auto surgery: next Surg-E at {x}, {y}"));
}

fn embed(markup: &str, key: &str) -> Option<i32> {
    let prefix = format!("embed_data|{key}|");
    markup.lines().find_map(|l| l.strip_prefix(prefix.as_str())).and_then(|v| v.trim_end_matches('|').trim().parse().ok())
}

/// The Surg-E's "SurgeryPrePopup": answered like Okay when the proxy wrenched
/// it for the next surgery. `true`: hide it.
pub fn on_surge_popup(ic: &mut Interceptor, markup: &str) -> bool {
    if !markup.contains("end_dialog|surge|") {
        return false;
    }
    let (Some(x), Some(y)) = (embed(markup, "tilex"), embed(markup, "tiley")) else { return false };
    if !ic.features.surgery_auto || ic.surgery.pending.is_none_or(|(t, _)| t != (x, y)) {
        return false;
    }
    log(&format!("--- surg-e popup {x},{y}\n{markup}\n"));
    let delay = jittered(Duration::from_millis(ic.features.surgery_delay_ms));
    ic.emit(Effect::text(format!("action|dialog_return\ndialog_name|surge\ntilex|{x}|\ntiley|{y}|\n")).later(delay));
    ic.surgery.pending = Some(((x, y), Instant::now() + delay));
    true
}

/// The player started a surgery on a Surg-E: it is used.
pub fn on_client_surge(ic: &mut Interceptor, x: Option<&str>, y: Option<&str>) {
    let parse = |v: Option<&str>| v.and_then(|v| v.trim_end_matches('|').trim().parse::<i32>().ok());
    if let (Some(x), Some(y)) = (parse(x), parse(y)) {
        if let Some(map) = &ic.world {
            if ic.surgery.used_world != map.name {
                ic.surgery.used.clear();
                ic.surgery.used_world = map.name.clone();
            }
        }
        if !ic.surgery.used.contains(&(x, y)) {
            ic.surgery.used.push((x, y));
        }
    }
}

/// The diagnosed illness, or the one the first window looked like.
fn current_illness(ic: &Interceptor) -> Option<String> {
    ic.surgery.illness.clone().or_else(|| ic.surgery.likely.map(|i| LIKELY[i].1.to_string()))
}

/// Eats Tea / Gemonade whose mod is off; returns how long to wait for them.
fn eat(ic: &mut Interceptor) -> Duration {
    if !ic.features.surgery_auto || !ic.features.surgery_food {
        return Duration::ZERO;
    }
    let mut wait = Duration::ZERO;
    let wanted = [
        (TEA, !ic.session.has_mod("calm nerves")),
        (GEMONADE, !ic.session.has_mod("food:")),
    ];
    for (item, needed) in wanted {
        // Its mod showed up: the tries start over.
        if !needed {
            ic.surgery.food.retain(|f| f.0 != item);
            continue;
        }
        if ic.inventory.count(item) == 0 {
            continue;
        }
        let tried = ic.surgery.food.iter().find(|f| f.0 == item).copied();
        if tried.is_some_and(|(_, at, n)| n >= FOOD_TRIES || at.elapsed() < FOOD_RETRY) {
            continue;
        }
        let tries = tried.map_or(0, |t| t.2) + 1;
        ic.surgery.food.retain(|f| f.0 != item);
        ic.surgery.food.push((item, Instant::now(), tries));
        let Some(pkt) = place_on_self(ic, item) else { continue };
        ic.emit(Effect::ToServer(pkt).later(wait));
        let name = ic.items.name(item);
        if tries == FOOD_TRIES {
            warn(ic, format!("{name} did not work {FOOD_TRIES} times, not trying it again"));
        } else {
            say(ic, format!("Auto surgery: eating {name}"));
        }
        log(&format!("--- eat {name}\n"));
        wait += FOOD_GAP;
    }
    wait
}

/// An item used on the player's own tile (dragged onto yourself).
fn place_on_self(ic: &Interceptor, item: u32) -> Option<Vec<u8>> {
    let pos = ic.session.pos?;
    let (x, y) = super::teleport::tile_of(pos);
    let pkt = crate::protocol::GameUpdatePacket {
        packet_type: crate::protocol::GamePacketType::TileChangeRequest,
        value: item,
        vec_x: pos.0,
        vec_y: pos.1,
        int_x: x,
        int_y: y,
        ..Default::default()
    };
    Some(crate::protocol::Message::Game(pkt).encode())
}

/// Malpractice: wear the Legal Briefs, or stop.
fn malpractice(ic: &mut Interceptor) {
    ic.surgery.next_at = None;
    ic.surgery.pending = None;
    if !ic.features.surgery_auto {
        return;
    }
    // The penalty comes as two lines at once: one answer.
    if ic.surgery.briefs_at.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
        return;
    }
    if ic.features.surgery_briefs && (ic.surgery.briefs_worn || ic.inventory.count(LEGAL_BRIEFS) > 0) {
        ic.surgery.briefs_at = Some(Instant::now());
        ic.surgery.briefs_ours = true;
        super::surgstats::on_briefs(ic);
        if ic.surgery.briefs_worn {
            // Already on: using them now would only take them off.
            ic.emit(Effect::activate_item(LEGAL_BRIEFS).later(Duration::from_millis(500)));
            ic.emit(Effect::activate_item(LEGAL_BRIEFS).later(Duration::from_millis(1400)));
            say(ic, "Malpractice: Legal Briefs off and on again");
        } else {
            ic.emit(Effect::activate_item(LEGAL_BRIEFS).later(Duration::from_millis(800)));
            say(ic, "Malpractice: wearing Legal Briefs");
        }
        return;
    }
    ic.features.surgery_auto = false;
    let why = if ic.features.surgery_briefs { "no Legal Briefs left" } else { "Legal Briefs are off" };
    warn(ic, format!("Malpractice ({why}): auto surgery stopped"));
}

/// The player's `OnSetClothing`: are the Legal Briefs worn?
pub fn on_clothing(ic: &mut Interceptor, vl: &crate::protocol::VariantList) {
    let briefs = LEGAL_BRIEFS as f32;
    ic.surgery.briefs_worn = (1..=3).any(|i| {
        matches!(vl.get(i), Some(crate::protocol::Variant::Vec3(a, b, c)) if [*a, *b, *c].contains(&briefs))
    });
}

/// Console lines and bubbles: the end of a surgery, Malpractice.
pub fn on_message(ic: &mut Interceptor, text: &str) {
    let plain = strip_colors(text).to_lowercase();
    if plain.contains("mod removed") || plain.contains("paid your debt") {
        if plain.contains("malpractice") {
            // Off again, so the next Malpractice can put them on.
            if std::mem::take(&mut ic.surgery.briefs_ours) {
                ic.emit(Effect::activate_item(LEGAL_BRIEFS).later(Duration::from_millis(700)));
            }
            schedule_next(ic);
        }
        return;
    }
    if plain.contains("not allowed to perform surgery") || plain.contains("malpractice mod added") {
        log(&format!("--- malpractice: {}\n", strip_colors(text)));
        super::surgstats::on_malpractice(ic);
        malpractice(ic);
        return;
    }
    let recent = ic.surgery.last_seen.is_some_and(|t| t.elapsed() < NEW_SURGERY_AFTER);
    if text.contains(SAVED) {
        ic.surgery.waiting = None;
        if !std::mem::replace(&mut ic.surgery.ended, true) {
            log("--- saved\n");
            let illness = current_illness(ic);
            super::surgstats::on_saved(ic, illness);
            if ic.features.surgery_auto {
                say(ic, "Auto surgery: patient saved");
                schedule_next(ic);
            }
        }
        return;
    }
    if FAILED.iter().any(|f| plain.contains(f)) && recent {
        ic.surgery.waiting = None;
        if !std::mem::replace(&mut ic.surgery.ended, true) {
            log(&format!("--- failed: {}\n", strip_colors(text)));
            let illness = current_illness(ic);
            super::surgstats::on_failed(ic, illness, text);
            if ic.features.surgery_auto {
                say(ic, "Auto surgery: the surgery failed, going on");
                schedule_next(ic);
            }
        }
    }
}

/// Gives the tool button a yellow frame.
fn mark_button(markup: &str, id: &str) -> String {
    markup
        .lines()
        .map(|line| {
            let mut parts: Vec<&str> = line.split('|').collect();
            if parts.first() == Some(&"add_button_with_icon") && parts.get(1) == Some(&id) && parts.len() > 3 {
                parts[3] = "staticYellowFrame";
                parts.join("|")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if markup.ends_with('\n') { "\n" } else { "" }
}

/// Client answer to a surgery window: remembers "Fix It!" and logs the choice.
pub fn on_client_choice(ic: &mut Interceptor, button: &str) {
    if button == "tool1296" {
        ic.surgery.fixed = true;
    }
    if button.starts_with("tool") {
        super::surgstats::on_move(ic);
    }
    log(&format!("--- pressed {button}\n"));
}

fn log(text: &str) {
    if cfg!(test) {
        return;
    }
    if std::fs::metadata(LOG_FILE).is_ok_and(|m| m.len() > LOG_MAX) {
        let _ = std::fs::remove_file(LOG_FILE);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(LOG_FILE) {
        let _ = f.write_all(text.as_bytes());
    }
}

/// The mode as one word.
fn mode_name(ic: &Interceptor) -> &'static str {
    if ic.features.surgery_auto && ic.surgery.off_after {
        "Automatic (off after this surgery)"
    } else if ic.features.surgery_auto {
        "Automatic"
    } else if ic.features.surgery_tips {
        "Assisted"
    } else {
        "Off"
    }
}

/// An automatic surgery is going on now.
fn in_surgery(ic: &Interceptor) -> bool {
    ic.features.surgery_auto && !ic.surgery.ended && ic.surgery.last_seen.is_some_and(|t| t.elapsed() < NEW_SURGERY_AFTER)
}

fn set_mode(ic: &mut Interceptor, mode: &str) {
    if mode == "off" && in_surgery(ic) {
        // Leaving the patient would fail the surgery: finish it, then off.
        ic.surgery.off_after = true;
        ic.surgery.next_at = None;
        ic.surgery.pending = None;
        say(ic, "Auto surgery: finishing this surgery, then off");
        return;
    }
    ic.surgery.off_after = false;
    match mode {
        "off" => {
            ic.features.surgery_tips = false;
            ic.features.surgery_auto = false;
            ic.surgery.waiting = None;
            ic.surgery.next_at = None;
            ic.surgery.pending = None;
        }
        "assist" | "on" => {
            ic.features.surgery_tips = true;
            ic.features.surgery_auto = false;
            ic.surgery.waiting = None;
        }
        "auto" => {
            ic.features.surgery_tips = true;
            ic.features.surgery_auto = true;
            ic.surgery.used.clear();
        }
        _ => return,
    }
    let text = match ic.features.surgery_auto {
        true => format!(
            "Auto surgery `2on`` ({} ms). Start the first surgery yourself. `4Botting: ban risk.``",
            ic.features.surgery_delay_ms
        ),
        false if ic.features.surgery_tips => "Surgery: `2assisted`` - the best tool gets a yellow frame".to_string(),
        false => "Surgery help `4off``".to_string(),
    };
    say(ic, text);
}

fn window(ic: &mut Interceptor) {
    let icon = ic.items.find("Surgical Scalpel").unwrap_or(1260);
    let mark = |on: bool, label: &str| if on { format!("`2> {label} <``") } else { label.to_string() };
    let f = &ic.features;
    let (off, assist, auto) = (!f.surgery_tips && !f.surgery_auto, f.surgery_tips && !f.surgery_auto, f.surgery_auto);
    let mode = mode_name(ic);
    let d = Dialog::new("autosurg")
        .title("Auto Surgery", icon)
        .field("Mode", mode)
        .spacer()
        .button("mode_off", &mark(off, "Off"))
        .button("mode_assist", &mark(assist, "Assisted"))
        .button("mode_auto", &mark(auto, "Automatic"))
        .small("Assisted marks the best tool. Automatic does it for you.")
        .small("`4Automatic is botting: ban risk.``")
        .spacer()
        .checkbox("next", "Go to the next Surg-E", f.surgery_next)
        .checkbox("briefs", "Use Legal Briefs on Malpractice", f.surgery_briefs)
        .checkbox("food", "Eat Tea & Gemonade", f.surgery_food)
        .text_input("delay", "Delay (ms):", &f.surgery_delay_ms.to_string(), 5)
        .small(&format!("{MIN_DELAY_MS}-{MAX_DELAY_MS} ms"));
    super::tools::show(ic, d, "Close", "Save");
}

fn set_delay(ic: &mut Interceptor, text: &str) {
    match text.trim().parse::<u64>() {
        Ok(ms) if (MIN_DELAY_MS..=MAX_DELAY_MS).contains(&ms) => {
            ic.features.surgery_delay_ms = ms;
            say(ic, format!("Surgery delay `w{ms} ms``"));
        }
        _ => warn(ic, format!("Delay must be {MIN_DELAY_MS}-{MAX_DELAY_MS} ms")),
    }
}

fn toggle(ic: &mut Interceptor, what: &str, on: bool) {
    match what {
        "next" => {
            ic.features.surgery_next = on;
            say(ic, format!("Next Surg-E {}", if on { "`2on``" } else { "`4off``" }));
        }
        "food" => {
            ic.features.surgery_food = on;
            say(ic, format!("Tea & Gemonade {}", if on { "`2on``" } else { "`4off``" }));
        }
        _ => {
            ic.features.surgery_briefs = on;
            say(ic, format!("Legal Briefs {}", if on { "`2on``" } else { "`4off``" }));
        }
    }
}

/// `/autosurg` opens the window; `/autosurg off|assist|auto|next|briefs`, `/autosurg delay <ms>`.
pub fn autosurg(ic: &mut Interceptor, args: &[&str]) {
    match args {
        [] => window(ic),
        ["delay", ms] => set_delay(ic, ms),
        ["next"] => toggle(ic, "next", !ic.features.surgery_next),
        ["briefs"] => toggle(ic, "briefs", !ic.features.surgery_briefs),
        ["food"] => toggle(ic, "food", !ic.features.surgery_food),
        [mode] => {
            let mode = mode.to_ascii_lowercase();
            if !["off", "assist", "on", "auto"].contains(&mode.as_str()) {
                return warn(ic, format!("Usage: {0}autosurg, {0}autosurg off|assist|auto|next|briefs|food, {0}autosurg delay <ms>", ic.cfg.command.prefix));
            }
            set_mode(ic, &mode);
        }
        _ => warn(ic, format!("Usage: {0}autosurg, {0}autosurg off|assist|auto|next|briefs|food, {0}autosurg delay <ms>", ic.cfg.command.prefix)),
    }
}

pub fn on_dialog(ic: &mut Interceptor, ret: &DialogReturn) -> bool {
    if ret.name != "autosurg" {
        return false;
    }
    for what in ["next", "briefs", "food"] {
        let now = match what {
            "next" => ic.features.surgery_next,
            "food" => ic.features.surgery_food,
            _ => ic.features.surgery_briefs,
        };
        if let Some(v) = ret.field(what) {
            if (v == "1") != now {
                toggle(ic, what, v == "1");
            }
        }
    }
    match ret.button {
        Some("mode_off") => set_mode(ic, "off"),
        Some("mode_assist") => set_mode(ic, "assist"),
        Some("mode_auto") => set_mode(ic, "auto"),
        _ => {
            if let Some(ms) = ret.field("delay") {
                if ms.trim() != ic.features.surgery_delay_ms.to_string() {
                    set_delay(ic, ms);
                }
            }
            return true;
        }
    }
    window(ic);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(lines: &str, tools: &[(u32, &str)]) -> String {
        let mut m = String::from("set_default_color|`o\nadd_label_with_icon|big|`wSurg-E``|left|18|\n");
        m.push_str(lines);
        for (id, name) in tools {
            m.push_str(&format!("add_button_with_icon|tool{id}|`${name}``|noflags|{id}|\n"));
        }
        m.push_str("add_button|cancel|Give up!|\nend_dialog|surgery|||\n");
        m
    }

    const ALL: &[(u32, &str)] = &[
        (1258, "Sponge"),
        (1260, "Scalpel"),
        (1262, "Anesthetic"),
        (1264, "Antiseptic"),
        (1266, "Antibiotics"),
        (1268, "Splint"),
        (1270, "Stitches"),
        (4308, "Pins"),
        (4310, "Transfusion"),
        (4312, "Defibrillator"),
        (4314, "Clamp"),
        (4316, "Ultrasound"),
        (4318, "Lab Kit"),
    ];

    fn tip(lines: &str, fixed: bool) -> String {
        let s = parse(&window(lines, ALL)).unwrap();
        recommend(&s, fixed, false).map(|r| r.1).unwrap_or_default()
    }

    #[test]
    fn reads_the_state_lines() {
        // A real start state from the other proxy's samples.
        let m = window(
            "add_smalltext|Pulse: `2Strong``    Status: `3Awake``|left|\nadd_smalltext|Temp: `3101.6``    Operation site: `6Unclean``|left|\nadd_smalltext|`6It is becoming hard to see your work.``|left|\nadd_smalltext|Incisions: `20``    |left|\nadd_smalltext|Patient is losing blood `3slowly.``|left|\n",
            ALL,
        );
        let s = parse(&m).unwrap();
        assert_eq!(s.status, Status::Awake);
        assert_eq!(s.temp, Some(101.6));
        assert!(s.site_dirty);
        assert_eq!(s.vision, Vision::Hard);
        assert_eq!(s.incisions, Some(0));
        assert_eq!(s.bleeding, Bleeding::Slow);
        assert_eq!(s.tools.len(), 13);
        assert!(parse("add_textbox|hi|left|\nend_dialog|other|||\n").is_none());
    }

    #[test]
    fn order_of_tools() {
        assert_eq!(tip("add_smalltext|Status: `4Heart stopped!``|left|\n", false), "defibrillator");
        assert_eq!(tip("add_smalltext|Pulse: `4Extremely Weak``    Status: `2Unconscious``|left|\n", false), "transfusion");
        assert_eq!(tip("add_smalltext|Pulse: `2Strong``    Status: `3Awake``|left|\n", false), "anesthetic");
        assert_eq!(
            tip("add_smalltext|Pulse: `2Strong``    Status: `2Unconscious``|left|\nadd_smalltext|Temp: `398.6``    Operation site: `3Not sanitized``|left|\n", false),
            "scalpel"
        );
        assert_eq!(tip("add_smalltext|`4The patient has not been diagnosed.``|left|\nadd_smalltext|Status: `2Unconscious``|left|\n", false), "ultrasound");
        assert_eq!(tip("add_smalltext|Status: `2Unconscious``|left|\nadd_smalltext|Incisions: `30``|left|\n", false), "scalpel");
        assert_eq!(tip("add_smalltext|Status: `2Unconscious``|left|\nadd_smalltext|Incisions: `32``|left|\n", true), "stitches");
        assert_eq!(tip("add_smalltext|Temp: `6104.6``    Status: `2Unconscious``|left|\n", false), "antibiotics");
    }

    /// Tools the user really had in the logged surgeries (no Antibiotics, no Clamp).
    const LOGGED: &[(u32, &str)] = &[
        (1258, "Sponge"),
        (1262, "Anesthetic"),
        (1270, "Stitches"),
        (1260, "Scalpel"),
        (4316, "Ultrasound"),
        (1264, "Antiseptic"),
        (4318, "Lab Kit"),
        (4310, "Transfusion"),
    ];

    fn logged(lines: &str, fixed: bool) -> String {
        let s = parse(&window(lines, LOGGED)).unwrap();
        recommend(&s, fixed, false).map(|r| r.1).unwrap_or_default()
    }

    #[test]
    fn logged_surgeries() {
        // First surgery: asleep and bleeding with no incision: hard to see: a Sponge first (bleeding and hard to see goes blind half the time).
        assert_eq!(
            logged("add_smalltext|`4The patient has not been diagnosed.``|left|\nadd_smalltext|Pulse: `6Weak``    Status: `2Unconscious``|left|\nadd_smalltext|Temp: `6103.28``    Operation site: `4Unsanitary``|left|\nadd_smalltext|`6It is becoming hard to see your work.``|left|\nadd_smalltext|Incisions: `30``    |left|\nadd_smalltext|Patient is `6losing blood!``|left|\nadd_smalltext|Patient's fever is `6climbing.``|left|\n", false),
            "sponge"
        );
        // Second surgery: fever at 108.8 must come down to 100.4 to finish at all,
        // and the Antibiotics need the Lab Kit first.
        assert_eq!(
            logged("add_smalltext|You popped in a fresh new kidney!|left|\nadd_smalltext|Pulse: `3Steady``    Status: `2Unconscious``|left|\nadd_smalltext|Temp: `4108.8``    Operation site: `2Clean``|left|\nadd_smalltext|`6It is becoming hard to see your work.``|left|\nadd_smalltext|Incisions: `62``    |left|\nadd_smalltext|Patient's fever is `4climbing rapidly!``|left|\n", true),
            "lab kit"
        );
        // Fever and no Antibiotics button yet: the Lab Kit unlocks them.
        assert_eq!(
            logged("add_smalltext|`4The patient has not been diagnosed.``|left|\nadd_smalltext|Pulse: `3Steady``    Status: `2Unconscious``|left|\nadd_smalltext|Temp: `3102.8``    Operation site: `3Not sanitized``|left|\nadd_smalltext|Incisions: `30``    |left|\nadd_smalltext|Patient's fever is `4climbing rapidly!``|left|\n", false),
            "lab kit"
        );
        // No fever: diagnose before cleaning.
        assert_eq!(
            logged("add_smalltext|`4The patient has not been diagnosed.``|left|\nadd_smalltext|Pulse: `3Steady``    Status: `2Unconscious``|left|\nadd_smalltext|Temp: `298.6``    Operation site: `3Not sanitized``|left|\nadd_smalltext|Incisions: `30``    |left|\n", false),
            "ultrasound"
        );
    }

    #[test]
    fn knows_the_illness_from_the_first_window() {
        // The first windows of both logged surgeries.
        let kidney = "add_smalltext|`4The patient has not been diagnosed.``|left|\nadd_smalltext|Pulse: `3Steady``    Status: `4Awake``|left|\nadd_smalltext|Temp: `3101.6``    Operation site: `3Not sanitized``|left|\nadd_smalltext|Incisions: `30``    |left|\nadd_smalltext|Patient's fever is `4climbing rapidly!``|left|\nadd_spacer|small|\nadd_smalltext|`3The patient is prepped for surgery.``|left|\n";
        assert_eq!(likely(&window(kidney, LOGGED)).map(|i| LIKELY[i].1), Some("kidney failure"));
        let head = "add_smalltext|`4The patient has not been diagnosed.``|left|\nadd_smalltext|Pulse: `6Weak``    Status: `4Awake``|left|\nadd_smalltext|Temp: `298.6``    Operation site: `6Unclean``|left|\nadd_smalltext|`6It is becoming hard to see your work.``|left|\nadd_smalltext|Incisions: `30``    |left|\nadd_smalltext|Patient is losing blood `4very quickly!``|left|\nadd_spacer|small|\n";
        assert_eq!(likely(&window(head, LOGGED)).map(|i| LIKELY[i].1), Some("serious head injury"));
        // A flu is not cut: after the fever, nothing is left to do.
        let s = parse(&window("add_smalltext|Patient has bird flu.|left|\nadd_smalltext|Pulse: `2Strong``    Status: `3Awake``|left|\nadd_smalltext|Temp: `298.6``    Operation site: `6Unclean``|left|\nadd_smalltext|Incisions: `20``|left|\n", ALL)).unwrap();
        assert_eq!(recommend(&s, false, true), None);
    }

    #[test]
    fn stitches_before_clamp_and_fix_it_again_after_a_fail() {
        // Bleeding with no incision: Stitches at once, fast or not.
        assert_eq!(tip("add_smalltext|Status: `2Unconscious``|left|\nadd_smalltext|Patient is losing blood `4very quickly!``|left|\n", false), "stitches");
        assert_eq!(tip("add_smalltext|Status: `4Awake``|left|\nadd_smalltext|Patient is `6losing blood!``|left|\n", false), "stitches");
        assert_eq!(tip("add_smalltext|Status: `2Unconscious``|left|\nadd_smalltext|Incisions: `61``|left|\nadd_smalltext|Patient is losing blood `4very quickly!``|left|\n", false), "stitches");
        let mut tools = ALL.to_vec();
        tools.push((1296, "Fix It!"));
        let s = parse(&window("add_smalltext|Status: `2Unconscious``|left|\nadd_smalltext|Incisions: `32``|left|\n", &tools)).unwrap();
        assert_eq!(recommend(&s, true, false).unwrap().0, "tool1296");
    }

    #[test]
    fn second_log() {
        // Broken leg, after the Pins: close before the Splint (no Fix It for broken bones).
        assert_eq!(
            tip("add_smalltext|Patient broke his leg.|left|\nadd_smalltext|Pulse: `2Strong``    Status: `2Unconscious``|left|\nadd_smalltext|Temp: `298.6``    Operation site: `3Not sanitized``|left|\nadd_smalltext|Incisions: `21``    Bones: `42 broken``|left|\nadd_smalltext|Patient is losing blood `3slowly.``|left|\n", false),
            "stitches"
        );
        // Trauma with a slowly rising fever, hard to see: mop first (no Antiseptic).
        assert_eq!(
            logged("add_smalltext|Patient suffered serious trauma with a punctured lung.|left|\nadd_smalltext|Pulse: `3Steady``    Status: `4Awake``|left|\nadd_smalltext|Temp: `3100.28``    Operation site: `4Unsanitary``|left|\nadd_smalltext|`6It is becoming hard to see your work.``|left|\nadd_smalltext|Incisions: `30``    Bones: `42 broken``, `61 shattered``|left|\nadd_smalltext|Patient's fever is `3slowly rising.``|left|\n", false),
            "sponge"
        );
        // Ran over by a truck: waking up with open incisions.
        assert_eq!(
            tip("add_smalltext|You've found gravel in the knees.|left|\nadd_smalltext|Pulse: `6Weak``    Status: `6Coming to``|left|\nadd_smalltext|Temp: `6103.28``    Operation site: `4Unsanitary``|left|\nadd_smalltext|Incisions: `22``    Bones: `42 broken``, `42 shattered``|left|\nadd_smalltext|Patient's fever is `3slowly rising.``|left|\n", false),
            "anesthetic"
        );
        assert_eq!(
            tip("add_smalltext|You've found gravel in the knees.|left|\nadd_smalltext|Pulse: `3Steady``    Status: `6Coming to``|left|\nadd_smalltext|Temp: `3101.28``    Operation site: `2Clean``|left|\nadd_smalltext|Incisions: `22``    Bones: `42 broken``, `42 shattered``|left|\n", true),
            "anesthetic"
        );
        // Flu, fever almost gone: finish with Antibiotics, never cut.
        assert_eq!(
            tip("add_smalltext|Patient is showing signs of the bird flu.|left|\nadd_smalltext|Pulse: `2Strong``    Status: `3Awake``|left|\nadd_smalltext|Temp: `3101.48``    Operation site: `6Unclean``|left|\nadd_smalltext|Incisions: `20``    |left|\n", false),
            "antibiotics"
        );
    }

    #[test]
    fn automatic_answers_hidden_and_stops_on_failure() {
        use crate::config::Config;
        use std::sync::Arc;
        let mut cfg = Config::default();
        cfg.log.text = false;
        cfg.log.variant = false;
        let mut ic = Interceptor::new(Arc::new(cfg));
        ic.features.surgery_tips = true;
        ic.features.surgery_auto = true;
        let m = window("add_smalltext|Pulse: `2Strong``    Status: `3Awake``|left|\n", ALL);
        assert!(matches!(on_server_dialog(&mut ic, &m), Some(Window::Hide)));
        let sent: Vec<String> = ic
            .take_effects()
            .into_iter()
            .filter_map(|e| match e {
                Effect::Later(_, inner) => match *inner {
                    Effect::ToServer(b) => Some(String::from_utf8_lossy(&b[4..]).trim_end_matches('\0').to_string()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(sent, vec!["action|dialog_return\ndialog_name|surgery\nbuttonClicked|tool1262\n"]);
        // No answer from the server: the window is handed over and auto stops.
        tick(&mut ic, Instant::now() + Duration::from_secs(30));
        assert!(!ic.features.surgery_auto);
        // A failed surgery does not stop it.
        ic.features.surgery_auto = true;
        on_message(&mut ic, "`4The patient died!``");
        assert!(ic.features.surgery_auto);
        // Assisted only marks.
        ic.features.surgery_auto = false;
        assert!(matches!(on_server_dialog(&mut ic, &m), Some(Window::Show(_))));
    }

    #[test]
    fn third_log_failures() {
        // Appendicitis: woke up with 3 incisions and bled: hard to see: a Sponge first, else it goes blind.
        assert_eq!(
            tip("add_smalltext|Pulse: `3Steady``    Status: `4Awake``|left|\nadd_smalltext|Temp: `298.78``    Operation site: `6Unclean``|left|\nadd_smalltext|`6It is becoming hard to see your work.``|left|\nadd_smalltext|Incisions: `63``    |left|\nadd_smalltext|Patient is `6losing blood!``|left|\n", true),
            "sponge"
        );
        // Head injury: weak and bleeding fast, no time for the cut trick.
        assert_eq!(
            tip("add_smalltext|`4The patient has not been diagnosed.``|left|\nadd_smalltext|Pulse: `6Weak``    Status: `4Awake``|left|\nadd_smalltext|Temp: `298.6``    Operation site: `6Unclean``|left|\nadd_smalltext|`6It is becoming hard to see your work.``|left|\nadd_smalltext|Incisions: `30``    |left|\nadd_smalltext|Patient is losing blood `4very quickly!``|left|\n", false),
            "stitches"
        );
        // Extremely weak, one incision, bleeding: closing it stops all bleeding.
        assert_eq!(
            tip("add_smalltext|`4The patient has not been diagnosed.``|left|\nadd_smalltext|Pulse: `4Extremely Weak``    Status: `2Unconscious``|left|\nadd_smalltext|Temp: `299.2``    Operation site: `4Unsanitary``|left|\nadd_smalltext|Incisions: `61``    |left|\nadd_smalltext|Patient is losing blood `4very quickly!``|left|\n", false),
            "stitches"
        );
        // "Not sanitized" is not worth a move before cutting.
        assert_eq!(
            tip("add_smalltext|Patient had a brain tumor.|left|\nadd_smalltext|Pulse: `2Strong``    Status: `2Unconscious``|left|\nadd_smalltext|Temp: `3100.4``    Operation site: `3Not sanitized``|left|\nadd_smalltext|Incisions: `63``    |left|\n", false),
            "scalpel"
        );
    }

    #[test]
    fn fourth_log() {
        // Truck victim, slowly bleeding, asleep, hard to see: mop, but after two
        // sponges in a row cut.
        let truck = "add_smalltext|Patient was run over by a truck.|left|\nadd_smalltext|Pulse: `3Steady``    Status: `2Unconscious``|left|\nadd_smalltext|Temp: `3101.84``    Operation site: `4Unsanitary``|left|\nadd_smalltext|`6It is becoming hard to see your work.``|left|\nadd_smalltext|Incisions: `30``    Bones: `44 shattered``|left|\nadd_smalltext|Patient is losing blood `3slowly.``|left|\n";
        let s = parse(&window(truck, ALL)).unwrap();
        assert_eq!(recommend_with(&s, false, false, true).unwrap().1, "sponge");
        assert_eq!(recommend_with(&s, false, false, false).unwrap().1, "scalpel");
        // "Unclean" alone is not cleaned before a cut.
        assert_eq!(
            tip("add_smalltext|Patient was run over by a truck.|left|\nadd_smalltext|Pulse: `3Steady``    Status: `2Unconscious``|left|\nadd_smalltext|Temp: `3100.58``    Operation site: `6Unclean``|left|\nadd_smalltext|Incisions: `30``    Bones: `44 shattered``|left|\n", false),
            "scalpel"
        );
    }

    #[test]
    fn fifth_log() {
        // Brain tumor found, hard to see, Unsanitary, mild fever: a Sponge first, then Fix It.
        let mut tools = ALL.to_vec();
        tools.push((1296, "Fix It!"));
        let s = parse(&window("add_smalltext|You've finally found the tumor!|left|\nadd_smalltext|Pulse: `3Steady``    Status: `2Unconscious``|left|\nadd_smalltext|Temp: `399.92``    Operation site: `4Unsanitary``|left|\nadd_smalltext|`6It is becoming hard to see your work.``|left|\nadd_smalltext|Incisions: `65``    |left|\nadd_smalltext|Patient is losing blood `3slowly.``|left|\nadd_smalltext|`6Patient's fever is slowly rising.``|left|\n", &tools)).unwrap();
        assert_eq!(recommend(&s, false, false).unwrap().0, "tool1258");
        assert_eq!(recommend_with(&s, false, false, false).unwrap().0, "tool1296");
        // Massive trauma, 2 incisions, Unsanitary, mild fever, weak: never Antiseptic.
        for site in ["Unsanitary", "Unclean", "Not sanitized"] {
            let w = format!("add_smalltext|Patient suffered massive trauma with internal bleeding.|left|\nadd_smalltext|Pulse: `3Steady``    Status: `2Unconscious``|left|\nadd_smalltext|Temp: `3100.64``    Operation site: `4{site}``|left|\nadd_smalltext|Incisions: `32``    Bones: `42 broken``, `42 shattered``|left|\nadd_smalltext|Patient is losing blood `3slowly.``|left|\n");
            let pick = tip(&w, false);
            assert!(pick != "antiseptic" && pick != "clamp", "{site}: {pick}");
        }
    }

    #[test]
    fn infection_death_is_a_failure() {
        use crate::config::Config;
        use std::sync::Arc;
        let mut ic = Interceptor::new(Arc::new(Config::default()));
        ic.features.surgery_auto = true;
        ic.surgery.last_seen = Some(Instant::now());
        on_message(&mut ic, "`4The patient has succumbed to infection.`` `4YOUR MEDICAL LICENSE IS REVOKED!``");
        // Logged as a failure, the next Surg-E comes; auto goes on.
        assert!(ic.surgery.ended && ic.surgery.next_at.is_some());
        assert!(ic.features.surgery_auto);
    }

    #[test]
    fn penalty_end_is_not_a_failure() {
        use crate::config::Config;
        use std::sync::Arc;
        let mut ic = Interceptor::new(Arc::new(Config::default()));
        ic.features.surgery_auto = true;
        ic.surgery.last_seen = Some(Instant::now());
        on_message(&mut ic, "You've paid your debt to society! (`$Malpractice`` mod removed)");
        assert!(ic.features.surgery_auto && !ic.surgery.ended);
        on_message(&mut ic, "`4The patient has bled out!`` `4YOUR MEDICAL LICENSE IS REVOKED!``");
        assert!(ic.features.surgery_auto && ic.surgery.ended);
    }

    /// Packets and texts sent to the server, now or later.
    fn sent(ic: &mut Interceptor) -> (Vec<crate::protocol::GameUpdatePacket>, Vec<String>) {
        let (mut pkts, mut texts) = (Vec::new(), Vec::new());
        for e in ic.take_effects() {
            let e = match e {
                Effect::Later(_, inner) => *inner,
                e => e,
            };
            if let Effect::ToServer(b) = e {
                if b[0] == 4 {
                    pkts.push(crate::protocol::GameUpdatePacket::from_bytes(&b[4..]).unwrap());
                } else {
                    texts.push(String::from_utf8_lossy(&b[4..]).trim_end_matches('\0').to_string());
                }
            }
        }
        (pkts, texts)
    }

    fn surge_world() -> Interceptor {
        use super::super::world::{Tile, WorldMap};
        use crate::config::Config;
        use std::sync::Arc;
        let mut cfg = Config::default();
        cfg.log.text = false;
        cfg.log.variant = false;
        let mut ic = Interceptor::new(Arc::new(cfg));
        // 10x3, Surg-Es at 2,1 4,1 and 9,1 (too far); the player at 1,1.
        let mut tiles = vec![Tile::default(); 30];
        for x in [2, 4, 9] {
            tiles[10 + x].fg = SURGE_ITEM;
        }
        ic.world = Some(WorldMap { name: "T".into(), width: 10, height: 3, tiles, last_object_uid: Some(0), objects: Vec::new() });
        ic.session.enter_world("T".into());
        ic.session.pos = Some((32.0, 32.0));
        ic.features.surgery_tips = true;
        ic.features.surgery_auto = true;
        ic
    }

    fn popup(x: i32, y: i32) -> String {
        format!("set_default_color|`o\nadd_label_with_icon|big|`9Surg-E Anatomical Dummy``|left|4296|\nadd_spacer|small|\nembed_data|tilex|{x}\nembed_data|tiley|{y}\nadd_popup_name|SurgeryPrePopup|\nend_dialog|surge|Cancel|Okay!|\n")
    }

    #[test]
    fn goes_to_the_next_surg_e() {
        use crate::protocol::GamePacketType;
        let mut ic = surge_world();
        // The player started on 2,1 themself.
        on_client_surge(&mut ic, Some("2"), Some("1"));
        ic.surgery.last_seen = Some(Instant::now());
        on_message(&mut ic, "`2YOU SAVED YOUR PATIENT!``");
        sent(&mut ic);
        tick(&mut ic, Instant::now() + Duration::from_secs(3));
        let (pkts, _) = sent(&mut ic);
        let w = pkts.iter().find(|p| p.packet_type == GamePacketType::TileChangeRequest).expect("wrench");
        assert_eq!((w.value, w.int_x, w.int_y, w.vec_x), (WRENCH, 4, 1, 32.0));
        // Someone else's Surg-E popup is left alone; ours is answered like Okay.
        assert!(!on_surge_popup(&mut ic, &popup(9, 1)));
        assert!(on_surge_popup(&mut ic, &popup(4, 1)));
        let (_, texts) = sent(&mut ic);
        assert_eq!(texts, vec!["action|dialog_return\ndialog_name|surge\ntilex|4|\ntiley|1|\n"]);
        // Its surgery starts.
        let m = window("add_smalltext|`3The patient is prepped for surgery.``|left|\nadd_smalltext|Pulse: `2Strong``    Status: `3Awake``|left|\n", ALL);
        assert!(matches!(on_server_dialog(&mut ic, &m), Some(Window::Hide)));
        assert!(ic.surgery.pending.is_none() && !ic.surgery.ended);
        // Failed: going on, but Malpractice comes; Legal Briefs are worn.
        ic.inventory.add(LEGAL_BRIEFS, 2);
        on_message(&mut ic, "`4The patient has bled out!`` `4YOUR MEDICAL LICENSE IS REVOKED!``");
        on_message(&mut ic, "You are not allowed to perform surgery for a while! (`$Malpractice`` mod added, `$45 mins`` left)");
        assert!(ic.features.surgery_auto && ic.surgery.next_at.is_none());
        let (pkts, _) = sent(&mut ic);
        assert!(pkts.iter().any(|p| p.packet_type == GamePacketType::ItemActivateRequest && p.value == LEGAL_BRIEFS));
        // The mod goes: the next Surg-E, but 9,1 is too far and 2,1, 4,1 are used.
        on_message(&mut ic, "You've paid your debt to society! (`$Malpractice`` mod removed)");
        tick(&mut ic, Instant::now() + Duration::from_secs(3));
        assert!(!ic.features.surgery_auto);
    }

    fn activations(ic: &mut Interceptor) -> usize {
        sent(ic).0.iter().filter(|p| p.packet_type == crate::protocol::GamePacketType::ItemActivateRequest && p.value == LEGAL_BRIEFS).count()
    }

    #[test]
    fn legal_briefs_on_and_off() {
        use crate::protocol::{Variant, VariantList};
        let clothing = |hat: f32| {
            VariantList::new(vec![
                Variant::String("OnSetClothing".into()),
                Variant::Vec3(1242.0, 0.0, hat),
                Variant::Vec3(0.0, 1244.0, 0.0),
                Variant::Vec3(0.0, 0.0, 8954.0),
                Variant::Unsigned(2190853119),
            ])
        };
        let penalty = "You are not allowed to perform surgery for a while! (`$Malpractice`` mod added, `$45 mins`` left)";
        let removed = "You've paid your debt to society! (`$Malpractice`` mod removed)";
        let mut ic = surge_world();
        ic.inventory.add(LEGAL_BRIEFS, 3);
        // Not worn: on, and off again once the mod is gone.
        on_message(&mut ic, penalty);
        assert_eq!(activations(&mut ic), 1);
        on_clothing(&mut ic, &clothing(3172.0));
        assert!(ic.surgery.briefs_worn);
        on_message(&mut ic, removed);
        assert_eq!(activations(&mut ic), 1);
        // Worn (taken off by hand never happened): off, then on.
        ic.surgery.briefs_at = None;
        on_message(&mut ic, penalty);
        assert_eq!(activations(&mut ic), 2);
        // The same penalty line twice is answered once.
        on_message(&mut ic, penalty);
        assert_eq!(activations(&mut ic), 0);
        on_clothing(&mut ic, &clothing(0.0));
        assert!(!ic.surgery.briefs_worn);
        assert!(ic.features.surgery_auto);
        // "You couldn't save them!" is a failure.
        ic.surgery.last_seen = Some(Instant::now());
        ic.surgery.ended = false;
        on_message(&mut ic, "You couldn't save them!");
        assert!(ic.surgery.ended);
    }

    #[test]
    fn off_finishes_the_surgery_first() {
        let mut ic = surge_world();
        let m = window("add_smalltext|`3The patient is prepped for surgery.``|left|\nadd_smalltext|Pulse: `2Strong``    Status: `3Awake``|left|\n", ALL);
        assert!(matches!(on_server_dialog(&mut ic, &m), Some(Window::Hide)));
        autosurg(&mut ic, &["off"]);
        // Still automatic: the next window is answered too.
        assert!(ic.features.surgery_auto && ic.surgery.off_after);
        assert!(matches!(on_server_dialog(&mut ic, &m), Some(Window::Hide)));
        // Saved: off now, no next Surg-E.
        on_message(&mut ic, "`2YOU SAVED YOUR PATIENT!``");
        assert!(!ic.features.surgery_auto && !ic.features.surgery_tips);
        assert!(ic.surgery.next_at.is_none() && !ic.surgery.off_after);
        // Between surgeries: off at once.
        let mut ic = surge_world();
        autosurg(&mut ic, &["off"]);
        assert!(!ic.features.surgery_auto);
        // A failed one also ends it.
        let mut ic = surge_world();
        on_server_dialog(&mut ic, &m);
        autosurg(&mut ic, &["off"]);
        on_message(&mut ic, "`4The patient has bled out!`` `4YOUR MEDICAL LICENSE IS REVOKED!``");
        assert!(!ic.features.surgery_auto);
    }

    #[test]
    fn eats_tea_and_gemonade() {
        use crate::protocol::GamePacketType;
        let mut ic = surge_world();
        ic.inventory.add(TEA, 3);
        ic.inventory.add(GEMONADE, 2);
        // Before the next Surg-E: eat both, the wrench waits.
        start_next(&mut ic, Instant::now());
        let (pkts, _) = sent(&mut ic);
        let eaten: Vec<_> = pkts.iter().filter(|p| p.packet_type == GamePacketType::TileChangeRequest).map(|p| (p.value, p.int_x, p.int_y)).collect();
        assert_eq!(eaten, vec![(TEA, 1, 1), (GEMONADE, 1, 1)]);
        assert!(ic.surgery.next_at.is_some());
        // The mods come; next time only the wrench.
        ic.session.note_mod("Steady hands of a surgeon. (`$Calm nerves.`` mod added, `$2 mins`` left)");
        ic.session.note_mod("Shiny! (`$Food: Sparkling Gems`` mod added, `$30 mins`` left)");
        start_next(&mut ic, Instant::now());
        let (pkts, _) = sent(&mut ic);
        assert_eq!(pkts.iter().map(|p| p.value).collect::<Vec<_>>(), vec![WRENCH]);
        // Another food (Skill Spice) is on: no Gemonade; Tea is out of time again.
        let mut ic = surge_world();
        ic.inventory.add(GEMONADE, 2);
        ic.session.note_mod("Reduce your skill fails by half. (`$Food: Spicey Skills`` mod added, `$30 mins`` left)");
        assert!(eat(&mut ic).is_zero());
        // Off: nothing.
        ic.session.mods.clear();
        ic.features.surgery_food = false;
        assert!(eat(&mut ic).is_zero());
    }

    #[test]
    fn stats_follow_the_surgery() {
        let mut ic = surge_world();
        let m = window("add_smalltext|`3The patient is prepped for surgery.``|left|\nadd_smalltext|Patient has a serious head injury.|left|\nadd_smalltext|`3[`4Skill Fail (10%)``] oops|left|\nadd_smalltext|Pulse: `2Strong``    Status: `3Awake``|left|\n", ALL);
        on_server_dialog(&mut ic, &m);
        on_message(&mut ic, "`2YOU SAVED YOUR PATIENT!``");
        super::super::surgstats::on_message(&mut ic, "You got 5 `2Hospital Window`` and a `3Caduceus``!");
        let s = ic.surg_stats.clone().unwrap();
        assert_eq!((s.saved, s.failed, s.skill_fails, s.moves, s.session), (1, 0, 1, 1, [1, 0]));
        assert_eq!(s.illnesses.get("serious head injury"), Some(&[1, 0]));
        assert_eq!(s.rewards.get("Caduceus"), Some(&1));
        // A failure, and its cause.
        on_server_dialog(&mut ic, &m);
        on_message(&mut ic, "`4The patient has bled out!`` `4YOUR MEDICAL LICENSE IS REVOKED!``");
        let s = ic.surg_stats.clone().unwrap();
        assert_eq!((s.saved, s.failed), (1, 1));
        assert_eq!(s.causes.get("bled out"), Some(&1));
    }

    #[test]
    fn malpractice_without_briefs_stops() {
        let mut ic = surge_world();
        on_message(&mut ic, "You are not allowed to perform surgery for a while! (`$Malpractice`` mod added, `$45 mins`` left)");
        assert!(!ic.features.surgery_auto);
        // Next Surg-E off: a saved patient leaves it to the player.
        let mut ic = surge_world();
        ic.features.surgery_next = false;
        ic.surgery.last_seen = Some(Instant::now());
        on_message(&mut ic, "`2YOU SAVED YOUR PATIENT!``");
        assert!(ic.surgery.next_at.is_none() && ic.features.surgery_auto);
    }

    #[test]
    fn low_supply_warning() {
        let m = "add_button_with_icon|tool1258|`$Sponge``|noflags|1258|119|\nadd_button_with_icon|tool1270|`$Stitches``|noflags|1270|12|\nadd_button_with_icon|nulltool||noflags|4320||\n";
        assert_eq!(tool_counts(m), vec![("Sponge".to_string(), 119), ("Stitches".to_string(), 12)]);
        let mut ic = surge_world();
        low_supply(&mut ic, m);
        assert_eq!(ic.take_effects().len(), 1);
        low_supply(&mut ic, "add_button_with_icon|tool1258|`$Sponge``|noflags|1258|119|\n");
        assert!(ic.take_effects().is_empty());
    }

    #[test]
    fn short_delays_are_allowed() {
        let mut ic = surge_world();
        set_delay(&mut ic, "10");
        assert_eq!(ic.features.surgery_delay_ms, 10);
        set_delay(&mut ic, "5");
        assert_eq!(ic.features.surgery_delay_ms, 10);
    }

    #[test]
    fn fix_it_when_offered() {
        let mut tools = ALL.to_vec();
        tools.push((1296, "Fix It!"));
        let s = parse(&window("add_smalltext|Status: `2Unconscious``|left|\nadd_smalltext|Incisions: `33``|left|\n", &tools)).unwrap();
        assert_eq!(recommend(&s, false, false).unwrap().0, "tool1296");
    }

    #[test]
    fn only_marks_the_button() {
        let m = window("add_smalltext|Status: `3Awake``|left|\n", ALL);
        let s = parse(&m).unwrap();
        let (id, _, _) = recommend(&s, false, false).unwrap();
        let out = mark_button(&m, &id);
        assert!(out.contains("add_button_with_icon|tool1262|`$Anesthetic``|staticYellowFrame|1262|"));
        // Nothing else changes.
        assert_eq!(out.lines().count(), m.lines().count());
        assert!(!out.contains("Tip"));
        assert!(out.ends_with("end_dialog|surgery|||\n"));
    }
}

/// `cargo test replay_log -- --ignored --nocapture`: what the tutor says for
/// every window of a saved surgery log (`SURGERY_LOG`, default surgery_log.txt).
#[cfg(test)]
#[test]
#[ignore]
fn replay_log() {
    let path = std::env::var("SURGERY_LOG").unwrap_or_else(|_| LOG_FILE.into());
    let text = std::fs::read_to_string(&path).expect("log file");
    let (mut fixed, mut likely_i) = (false, None);
    for (n, block) in text.split("--- ").filter(|b| b.starts_with("window")).enumerate() {
        let markup = block.split_once('\n').map_or("", |x| x.1);
        let Some(state) = parse(markup) else { continue };
        if strip_colors(markup).contains("prepped for surgery") {
            fixed = false;
            likely_i = likely(markup);
        }
        let flu = likely_i.is_some_and(|i| LIKELY[i].1.ends_with("flu"));
        let tip = recommend(&state, fixed, flu).map(|r| format!("{} ({})", r.1, r.2)).unwrap_or("-".into());
        println!("{n:2}: temp {:?} {:?} bleed {:?} -> {tip}", state.temp, state.status, state.bleeding);
        let pressed = text.split("--- ").skip_while(|b| *b != block).nth(1).unwrap_or("");
        if pressed.contains("pressed tool1296") {
            fixed = true;
        }
    }
}
