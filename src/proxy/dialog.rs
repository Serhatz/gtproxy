//! Builder for in-game dialogs (`OnDialogRequest`).
//!
//! The client renders a line-based markup (`add_textbox|...|left|`) and, when
//! a button is pressed, answers with
//! `action|dialog_return\ndialog_name|<name>\nbuttonClicked|<id>\n`.
//! Every proxy dialog name starts with [`PREFIX`], so those answers can be
//! recognised and kept away from the server.


pub const PREFIX: &str = "gtproxy_";

/// Item shown next to dialog titles (Wrench).
pub const DEFAULT_ICON: u32 = 32;

pub struct Dialog {
    name: String,
    body: String,
}

impl Dialog {
    /// `name` without the prefix, e.g. `"status"` -> `gtproxy_status`.
    pub fn new(name: &str) -> Self {
        Self {
            name: format!("{PREFIX}{name}"),
            body: String::from("set_default_color|`o\n"),
        }
    }

    fn line(mut self, parts: &[&str]) -> Self {
        for p in parts {
            self.body.push_str(p);
            self.body.push('|');
        }
        self.body.push('\n');
        self
    }

    pub fn title(self, text: &str, icon: u32) -> Self {
        self.line(&["add_label_with_icon", "big", &format!("`w{}``", clean(text)), "left", &icon.to_string()])
    }

    pub fn heading(self, text: &str) -> Self {
        self.line(&["add_custom_textbox", &format!("`w{}``", clean(text)), "size:medium"])
    }

    pub fn text(self, text: &str) -> Self {
        self.line(&["add_textbox", &clean(text), "left"])
    }

    pub fn small(self, text: &str) -> Self {
        self.line(&["add_smalltext", &clean(text), "left"])
    }

    /// `label: value` row with a dimmed label.
    pub fn field(self, label: &str, value: &str) -> Self {
        self.text(&format!("`s{}:`` `w{}``", clean(label), clean(value)))
    }

    pub fn spacer(self) -> Self {
        self.line(&["add_spacer", "small"])
    }

    pub fn text_input(self, id: &str, label: &str, value: &str, max_len: u32) -> Self {
        self.line(&["add_text_input", id, &clean(label), &clean(value), &max_len.to_string()])
    }

    pub fn checkbox(self, id: &str, label: &str, checked: bool) -> Self {
        self.line(&["add_checkbox", id, &clean(label), if checked { "1" } else { "0" }])
    }

    /// Item icon with a label, e.g. one inventory row.
    pub fn item(self, icon: u32, text: &str) -> Self {
        self.line(&["add_label_with_icon", "small", &clean(text), "left", &icon.to_string()])
    }

    /// A clickable one-line row with an item icon; pressing it returns `id`.
    pub fn item_button(self, icon: u32, text: &str, id: &str) -> Self {
        self.line(&["add_label_with_icon_button", "small", &clean(text), "left", &icon.to_string(), id])
    }

    pub fn button(self, id: &str, label: &str) -> Self {
        self.line(&["add_button", id, &clean(label), "noflags", "0", "0"])
    }

    /// Finishes the dialog. Empty labels hide the bottom buttons; the
    /// quick-exit `X` in the corner is always there.
    pub fn build(self, cancel: &str, ok: &str) -> String {
        let name = self.name.clone();
        self.line(&["add_quick_exit"]).line(&["end_dialog", &name, cancel, ok]).body
    }
}

/// `|` and newlines would break the markup; dynamic values (world names,
/// player names from the server) go through here.
fn clean(s: &str) -> String {
    s.replace(['|', '\n', '\r'], " ")
}

/// A button press (or the OK button) on one of our dialogs.
pub struct DialogReturn<'a> {
    /// Name without [`PREFIX`].
    pub name: &'a str,
    /// `None` when the bottom OK button was used instead of an `add_button`.
    pub button: Option<&'a str>,
    fields: &'a crate::protocol::TextPacket,
}

impl<'a> DialogReturn<'a> {
    pub fn parse(tp: &'a crate::protocol::TextPacket) -> Option<Self> {
        let name = tp.get("dialog_name")?.strip_prefix(PREFIX)?;
        Some(Self {
            name,
            button: tp.get("buttonClicked").filter(|b| !b.is_empty()),
            fields: tp,
        })
    }

    /// Value of a text input or checkbox (`"1"`/`"0"`).
    pub fn field(&self, id: &str) -> Option<&'a str> {
        self.fields.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::TextPacket;

    #[test]
    fn builds_markup() {
        let text = Dialog::new("status")
            .title("GTProxy", DEFAULT_ICON)
            .field("World", "START|evil\nline")
            .button("refresh", "Refresh")
            .build("Close", "");
        assert_eq!(
            text,
            "set_default_color|`o\n\
             add_label_with_icon|big|`wGTProxy``|left|32|\n\
             add_textbox|`sWorld:`` `wSTART evil line``|left|\n\
             add_button|refresh|Refresh|noflags|0|0|\n\
             add_quick_exit|\n\
             end_dialog|gtproxy_status|Close||\n"
        );
    }

    #[test]
    fn parses_return() {
        let tp = TextPacket::parse("action|dialog_return\ndialog_name|gtproxy_status\nbuttonClicked|refresh\n");
        let r = DialogReturn::parse(&tp).unwrap();
        assert_eq!((r.name, r.button), ("status", Some("refresh")));

        let other = TextPacket::parse("action|dialog_return\ndialog_name|sign_edit\n");
        assert!(DialogReturn::parse(&other).is_none());
    }
}
