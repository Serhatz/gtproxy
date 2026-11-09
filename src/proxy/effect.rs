use std::time::{Duration, Instant};

use crate::protocol::message::encode_text;
use crate::protocol::{GameUpdatePacket, Message, MessageType, Variant, VariantList};

/// Something the interceptor wants the bridge to do, besides forwarding.
#[derive(Debug, Clone)]
pub enum Effect {
    ToClient(Vec<u8>),
    ToServer(Vec<u8>),
    Later(Duration, Box<Effect>),
}

impl Effect {
    /// A console line shown in the game's chat log.
    pub fn console(text: impl Into<String>) -> Self {
        let vl = VariantList::new(vec![
            Variant::String("OnConsoleMessage".into()),
            Variant::String(text.into()),
        ]);
        Self::ToClient(Message::Game(GameUpdatePacket::call_function(-1, vl.serialize())).encode())
    }

    /// Big text in the middle of the screen, for warnings.
    pub fn overlay(text: impl Into<String>) -> Self {
        let vl = VariantList::new(vec![Variant::String("OnTextOverlay".into()), Variant::String(text.into())]);
        Self::ToClient(Message::Game(GameUpdatePacket::call_function(-1, vl.serialize())).encode())
    }

    /// Opens a dialog; `markup` comes from [`super::dialog::Dialog`].
    pub fn dialog(markup: String) -> Self {
        let vl = VariantList::new(vec![Variant::String("OnDialogRequest".into()), Variant::String(markup)]);
        Self::ToClient(Message::Game(GameUpdatePacket::call_function(-1, vl.serialize())).encode())
    }

    /// Function call to the game client, e.g. `OnNameChanged` for the local player.
    pub fn client_call(net_id: i32, items: Vec<Variant>) -> Self {
        let vl = VariantList::new(items);
        Self::ToClient(Message::Game(GameUpdatePacket::call_function(net_id, vl.serialize())).encode())
    }

    /// A chat line (or game command like `/pull name`) sent as if typed.
    pub fn chat(text: &str) -> Self {
        Self::text(format!("action|input
|text|{text}
"))
    }

    /// "Use" an item from the inventory (converts locks, wears clothes).
    pub fn activate_item(item_id: u32) -> Self {
        let pkt = GameUpdatePacket {
            packet_type: crate::protocol::GamePacketType::ItemActivateRequest,
            value: item_id,
            ..Default::default()
        };
        Self::ToServer(Message::Game(pkt).encode())
    }

    /// Generic text to the server (type 2), e.g. a `dialog_return`.
    pub fn text(text: impl AsRef<str>) -> Self {
        Self::ToServer(encode_text(MessageType::GenericText, text.as_ref()))
    }

    /// `action|...` game message to the server (type 3).
    pub fn action(text: impl AsRef<str>) -> Self {
        Self::ToServer(encode_text(MessageType::GameMessage, text.as_ref()))
    }

    pub fn join_world(name: &str) -> Self {
        Self::action(format!("action|join_request\nname|{name}\ninvitedWorld|0\n"))
    }

    pub fn quit_to_exit() -> Self {
        Self::action("action|quit_to_exit\n")
    }

    pub fn later(self, delay: Duration) -> Self {
        Self::Later(delay, Box::new(self))
    }
}

/// Delayed effects, fired from the bridge loop.
#[derive(Default)]
pub struct Scheduler {
    queue: Vec<(Instant, Effect)>,
}

impl Scheduler {
    pub fn push(&mut self, delay: Duration, effect: Effect) {
        self.queue.push((Instant::now() + delay, effect));
    }

    pub fn due(&mut self) -> Vec<Effect> {
        let now = Instant::now();
        let (ready, pending): (Vec<_>, Vec<_>) = self.queue.drain(..).partition(|(at, _)| *at <= now);
        self.queue = pending;
        ready.into_iter().map(|(_, e)| e).collect()
    }

    pub fn clear(&mut self) {
        self.queue.clear();
    }
}
