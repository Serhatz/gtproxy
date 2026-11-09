//! Growtopia wire format.
//!
//! Every ENet payload starts with a little-endian `u32` message type:
//! - 1 `ServerHello`  — empty, server -> client, starts the login
//! - 2 `GenericText`  — `key|value\n` text (login, actions, chat input)
//! - 3 `GameMessage`  — `action|...` text (join_request, quit_to_exit, ...)
//! - 4 `GamePacket`   — 56-byte [`GameUpdatePacket`] + optional extra data
//! - 6 `Track`        — analytics text
//! - 7 `ClientLogRequest`

pub mod game_packet;
pub mod message;
pub mod text;
pub mod variant;

pub use game_packet::{GamePacketType, GameUpdatePacket};
pub use message::{Message, MessageType};
pub use text::TextPacket;
pub use variant::{Variant, VariantList};
