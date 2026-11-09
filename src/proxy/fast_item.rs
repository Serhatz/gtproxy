//! Fast drop / fast trash: skip the "How many?" dialog and use the whole stack.
//!
//! Normal flow:
//! 1. client -> server  `action|drop\n|itemID|242`
//! 2. server -> client  `OnDialogRequest` ending in `end_dialog|drop_item|...`,
//!    with a `count` text input pre-filled with the amount you own
//! 3. client -> server  `action|dialog_return\ndialog_name|drop_item\nitemID|242|\ncount|200\n`
//!
//! With the feature on, the proxy answers step 2 itself using the pre-filled
//! count and never shows the dialog. Trash is the same with `trash_item`.

use std::time::{Duration, Instant};

use super::session::strip_colors;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemAction {
    Drop,
    Trash,
}

impl ItemAction {
    pub fn from_action(action: &str) -> Option<Self> {
        match action {
            "drop" => Some(Self::Drop),
            "trash" => Some(Self::Trash),
            _ => None,
        }
    }

    pub fn dialog_name(self) -> &'static str {
        match self {
            Self::Drop => "drop_item",
            Self::Trash => "trash_item",
        }
    }

    pub fn verb(self) -> &'static str {
        match self {
            Self::Drop => "dropped",
            Self::Trash => "trashed",
        }
    }
}

/// What the server's "How many?" dialog tells us.
#[derive(Debug, PartialEq)]
pub struct ItemDialog {
    pub action: ItemAction,
    pub item_id: Option<u32>,
    pub count: Option<u32>,
    /// From the title, e.g. `` `wDrop Dirt`` `` -> `Dirt`.
    pub item_name: Option<String>,
}

/// Recognises a drop/trash count dialog by its `end_dialog` name.
pub fn parse_dialog(markup: &str) -> Option<ItemDialog> {
    let mut action = None;
    let mut item_id = None;
    let mut count = None;
    let mut item_name = None;
    for line in markup.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        match parts.as_slice() {
            ["end_dialog", name, ..] => {
                action = [ItemAction::Drop, ItemAction::Trash].into_iter().find(|a| a.dialog_name() == *name);
            }
            ["embed_data", "itemID", v, ..] => item_id = v.trim().parse().ok(),
            // add_text_input|count|<label>|<default>|<max length>|
            // Drop pre-fills the amount you own; trash/recycle pre-fills 0.
            ["add_text_input", "count", _, v, ..] => {
                if let Some(c) = v.trim().parse().ok().filter(|&c: &u32| c > 0) {
                    count = count.or(Some(c));
                }
            }
            // Trash/recycle: "How many to `4destroy``? (you have 3)"
            ["add_textbox", text, ..] => {
                if let Some(owned) = you_have(text) {
                    count = Some(owned);
                }
            }
            ["add_label_with_icon", _, title, ..] if item_name.is_none() => {
                let title = strip_colors(title);
                let name = title.trim().trim_start_matches("Drop ").trim_start_matches("Trash ").trim_start_matches("Recycle ").trim();
                item_name = (!name.is_empty()).then(|| name.to_string());
            }
            _ => {}
        }
    }
    Some(ItemDialog { action: action?, item_id, count, item_name })
}

/// The number in "(you have 3)".
fn you_have(text: &str) -> Option<u32> {
    let rest = &text[text.find("you have ")? + "you have ".len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

pub fn dialog_return(action: ItemAction, item_id: u32, count: u32) -> String {
    format!(
        "action|dialog_return\ndialog_name|{}\nitemID|{item_id}|\ncount|{count}\n",
        action.dialog_name()
    )
}

/// Locks that `/dw`, `/dd`, `/db` and `/daw` drop: (item ID, name).
pub const LOCKS: [(u32, &str); 3] = [(242, "World Lock"), (1796, "Diamond Lock"), (7188, "Blue Gem Lock")];

/// How long a proxy-sent drop waits for its dialog. The server sends none
/// when you do not have the item, so the plan must not linger and catch a
/// later manual drop.
const PLAN_TIMEOUT: Duration = Duration::from_secs(3);

/// A drop the proxy asked for itself (`/dw 10`): answer its dialog with
/// `amount`, or the whole stack when `None`.
#[derive(Debug, Clone)]
pub struct PlannedDrop {
    pub item_id: u32,
    pub name: String,
    pub amount: Option<u32>,
    pub expires: Instant,
    /// `/dropall`: no chat line per item, no warning when the server stays silent.
    pub quiet: bool,
}

impl PlannedDrop {
    pub fn new(item_id: u32, name: impl Into<String>, amount: Option<u32>, send_delay: Duration) -> Self {
        Self { item_id, name: name.into(), amount, expires: Instant::now() + send_delay + PLAN_TIMEOUT, quiet: false }
    }

    pub fn request(&self) -> String {
        format!("action|drop
|itemID|{}
", self.item_id)
    }
}

/// Removes and returns the plan this drop dialog belongs to: by item ID when
/// the dialog has one, otherwise by name, otherwise the oldest.
/// Removes plans whose dialog never came (the server stays silent when you
/// do not have the item) and returns them.
pub fn take_expired(plans: &mut Vec<PlannedDrop>) -> Vec<PlannedDrop> {
    let now = Instant::now();
    let (expired, live): (Vec<_>, Vec<_>) = plans.drain(..).partition(|p| p.expires <= now);
    *plans = live;
    expired
}

pub fn take_plan(plans: &mut Vec<PlannedDrop>, dialog: &ItemDialog) -> Option<PlannedDrop> {
    let now = Instant::now();
    plans.retain(|p| p.expires > now);
    if dialog.action != ItemAction::Drop || plans.is_empty() {
        return None;
    }
    let index = match (dialog.item_id, dialog.item_name.as_deref()) {
        (Some(id), _) => plans.iter().position(|p| p.item_id == id)?,
        (None, Some(name)) => plans.iter().position(|p| p.name.eq_ignore_ascii_case(name)).unwrap_or(0),
        (None, None) => 0,
    };
    Some(plans.remove(index))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DROP: &str = "set_default_color|`o\n\
        add_label_with_icon|big|`wDrop Dirt``|left|2|\n\
        add_textbox|How many to drop?|left|\n\
        add_text_input|count||200|5|\n\
        embed_data|itemID|2\n\
        end_dialog|drop_item|Cancel|OK|\n";

    #[test]
    fn parses_drop_dialog() {
        assert_eq!(
            parse_dialog(DROP),
            Some(ItemDialog {
                action: ItemAction::Drop,
                item_id: Some(2),
                count: Some(200),
                item_name: Some("Dirt".into())
            })
        );
    }

    #[test]
    fn parses_trash_dialog_without_item_id() {
        let trash = "add_label_with_icon|big|`4Trash`` `wDirt``|left|2|\n\
            add_textbox|How many to `4destroy``? (you have 37)|left|\n\
            add_text_input|count||0|5|\n\
            end_dialog|trash_item|Cancel|OK|\n";
        assert_eq!(
            parse_dialog(trash),
            Some(ItemDialog {
                action: ItemAction::Trash,
                item_id: None,
                count: Some(37),
                item_name: Some("Dirt".into())
            })
        );
    }

    #[test]
    fn parses_real_recycle_dialog() {
        // Captured from the game (Garbage, recycle variant of trash).
        let recycle = "set_default_color|`o\n\
            add_label_with_icon|big|`4Recycle`` `wGarbage``|left|856|\n\
            add_label|small|You will get 0-1 gems per item.|left|\n\
            add_textbox|How many to `4destroy``? (you have 3)|left|\n\
            add_text_input|count||0|5|\n\
            embed_data|itemID|856\n\
            end_dialog|trash_item|Cancel|OK|\n";
        let d = parse_dialog(recycle).unwrap();
        assert_eq!((d.action, d.item_id, d.count), (ItemAction::Trash, Some(856), Some(3)));
        assert_eq!(d.item_name.as_deref(), Some("Garbage"));
    }

    #[test]
    fn ignores_other_dialogs() {
        assert!(parse_dialog("add_textbox|hi|left|\nend_dialog|sign_edit|Cancel|OK|\n").is_none());
    }

    #[test]
    fn plans_match_by_id_then_name() {
        let mut plans = vec![
            PlannedDrop::new(242, "World Lock", Some(10), Duration::ZERO),
            PlannedDrop::new(1796, "Diamond Lock", None, Duration::ZERO),
        ];
        let dl = ItemDialog { action: ItemAction::Drop, item_id: None, count: Some(3), item_name: Some("Diamond Lock".into()) };
        assert_eq!(take_plan(&mut plans, &dl).map(|p| p.item_id), Some(1796));
        let wl = ItemDialog { action: ItemAction::Drop, item_id: Some(242), count: Some(50), item_name: None };
        assert_eq!(take_plan(&mut plans, &wl).map(|p| p.amount), Some(Some(10)));
        assert!(take_plan(&mut plans, &wl).is_none());
    }

    #[test]
    fn expired_plans_are_reported() {
        let mut plans = vec![
            PlannedDrop::new(1796, "Diamond Lock", Some(10), Duration::ZERO),
            PlannedDrop::new(242, "World Lock", None, Duration::from_secs(60)),
        ];
        plans[0].expires = Instant::now() - Duration::from_millis(1);
        let expired = take_expired(&mut plans);
        assert_eq!(expired.iter().map(|p| p.item_id).collect::<Vec<_>>(), vec![1796]);
        assert_eq!(plans.len(), 1);
    }

    #[test]
    fn builds_return() {
        assert_eq!(
            dialog_return(ItemAction::Trash, 242, 5),
            "action|dialog_return\ndialog_name|trash_item\nitemID|242|\ncount|5\n"
        );
    }
}
