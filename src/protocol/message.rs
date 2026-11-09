use super::game_packet::GameUpdatePacket;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    ServerHello,
    GenericText,
    GameMessage,
    GamePacket,
    Error,
    Track,
    ClientLogRequest,
    ClientLogResponse,
    Unknown(u32),
}

impl MessageType {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::ServerHello,
            2 => Self::GenericText,
            3 => Self::GameMessage,
            4 => Self::GamePacket,
            5 => Self::Error,
            6 => Self::Track,
            7 => Self::ClientLogRequest,
            8 => Self::ClientLogResponse,
            other => Self::Unknown(other),
        }
    }

    pub fn as_u32(self) -> u32 {
        match self {
            Self::ServerHello => 1,
            Self::GenericText => 2,
            Self::GameMessage => 3,
            Self::GamePacket => 4,
            Self::Error => 5,
            Self::Track => 6,
            Self::ClientLogRequest => 7,
            Self::ClientLogResponse => 8,
            Self::Unknown(v) => v,
        }
    }
}

/// A decoded ENet payload.
#[derive(Debug, Clone)]
pub enum Message {
    ServerHello,
    /// Type 2 or 3 text; `kind` keeps which one so it can be re-encoded unchanged.
    Text { kind: MessageType, text: String },
    Game(GameUpdatePacket),
    Other { kind: MessageType, body: Vec<u8> },
}

impl Message {
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }
        let kind = MessageType::from_u32(u32::from_le_bytes(data[0..4].try_into().ok()?));
        let body = &data[4..];
        Some(match kind {
            MessageType::ServerHello => Self::ServerHello,
            MessageType::GenericText | MessageType::GameMessage | MessageType::Track => {
                // The last byte is the terminator slot. The client does not
                // always zero it, so it can hold any leftover byte (`*`, 0x0F,
                // ...) that would otherwise stick to the text: always drop it.
                let body = &body[..body.len().saturating_sub(1)];
                let end = body.iter().position(|&b| b == 0).unwrap_or(body.len());
                Self::Text {
                    kind,
                    text: String::from_utf8_lossy(&body[..end]).into_owned(),
                }
            }
            MessageType::GamePacket => Self::Game(GameUpdatePacket::from_bytes(body)?),
            _ => Self::Other { kind, body: body.to_vec() },
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::ServerHello => MessageType::ServerHello.as_u32().to_le_bytes().to_vec(),
            Self::Text { kind, text } => encode_text(*kind, text),
            Self::Game(pkt) => {
                let mut buf = MessageType::GamePacket.as_u32().to_le_bytes().to_vec();
                buf.extend_from_slice(&pkt.to_bytes());
                buf
            }
            Self::Other { kind, body } => {
                let mut buf = kind.as_u32().to_le_bytes().to_vec();
                buf.extend_from_slice(body);
                buf
            }
        }
    }
}

pub fn encode_text(kind: MessageType, text: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + text.len() + 1);
    buf.extend_from_slice(&kind.as_u32().to_le_bytes());
    buf.extend_from_slice(text.as_bytes());
    buf.push(0);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrip() {
        let raw = encode_text(MessageType::GameMessage, "action|join_request\nname|START\n");
        let Some(Message::Text { kind, text }) = Message::decode(&raw) else { panic!() };
        assert_eq!(kind, MessageType::GameMessage);
        assert_eq!(text, "action|join_request\nname|START\n");
        assert_eq!(Message::Text { kind, text }.encode(), raw);
    }

    #[test]
    fn stray_terminator_byte() {
        // Seen in game: the terminator slot held 0xB4, 0x0F and '*'.
        for junk in [0xb4u8, 0x0f, b'*'] {
            let mut raw = MessageType::GenericText.as_u32().to_le_bytes().to_vec();
            raw.extend_from_slice(b"action|input\n|text|/fastdrop");
            raw.push(junk);
            let Some(Message::Text { text, .. }) = Message::decode(&raw) else { panic!() };
            assert_eq!(text, "action|input\n|text|/fastdrop");
        }
    }
}
