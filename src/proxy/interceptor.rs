//! Looks at every packet passing through and decides what happens to it.
//!
//! Pure logic: no sockets. The bridge feeds packets in and applies the
//! returned [`Verdict`] plus any queued [`Effect`]s.

use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::items::ItemNames;
use crate::protocol::game_packet::PacketFlags;
use crate::protocol::{GamePacketType, GameUpdatePacket, Message, MessageType, TextPacket, Variant, VariantList};

use super::Direction;
use super::commands;
use super::dialog::DialogReturn;
use super::effect::Effect;
use super::fast_item::{self, ItemAction, PlannedDrop};
use super::features::Features;
use super::inventory::{self, Inventory};
use super::tools;
use super::{chat_tools, settings, social};
use super::stats::SpinStats;
use super::fake_drop::{self, FakeDrops};
use super::teleport::{self, Teleport};
use super::world::{Special, WorldMap};
use super::log::PacketLog;
use super::roulette::{self, RouletteSettings};
use super::session::{Session, strip_colors};

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Forward,
    Drop,
    Replace(Vec<u8>),
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LinkStatus {
    pub client_rtt: Option<Duration>,
    pub server_rtt: Option<Duration>,
}

pub struct Interceptor {
    pub cfg: Arc<Config>,
    pub session: Session,
    pub log: PacketLog,
    pub link: LinkStatus,
    /// Real sub-server from the last `OnSendToServer`; used for the client's next connect.
    pub redirect: Option<SocketAddr>,
    pub started: Instant,
    pub roulette: RouletteSettings,
    pub fast_drop: bool,
    pub fast_trash: bool,
    /// Last `action|drop` / `action|trash` from the client, for dialogs that omit the item ID.
    pending_item: Option<(ItemAction, u32)>,
    /// Drops requested by proxy commands (`/dw`, `/daw`, ...), waiting for their dialog.
    pub planned_drops: Vec<PlannedDrop>,
    pub inventory: Inventory,
    pub items: Arc<ItemNames>,
    pub features: Features,
    pub store: settings::Store,
    pub vends: super::vends::VendDb,
    pub warp: super::warp::Warp,
    pub history: super::history::History,
    pub drop_all: super::dropall::DropAll,
    pub collect: super::collect::Collect,
    pub roulette_stats: SpinStats,
    /// Tiles of the current world, `None` outside a world or if the map could not be read.
    pub world: Option<WorldMap>,
    pub teleport: Teleport,
    pub fake_drops: FakeDrops,
    effects: Vec<Effect>,
}

impl Interceptor {
    pub fn new(cfg: Arc<Config>) -> Self {
        Self {
            log: PacketLog::from(&cfg.log),
            cfg,
            session: Session::default(),
            link: LinkStatus::default(),
            redirect: None,
            started: Instant::now(),
            roulette: RouletteSettings::default(),
            fast_drop: false,
            fast_trash: false,
            pending_item: None,
            planned_drops: Vec::new(),
            inventory: Inventory::default(),
            items: Arc::new(ItemNames::default()),
            features: Features::default(),
            store: settings::Store::default(),
            vends: Default::default(),
            warp: Default::default(),
            history: Default::default(),
            drop_all: Default::default(),
            collect: Default::default(),
            roulette_stats: SpinStats::default(),
            world: None,
            teleport: Teleport::default(),
            fake_drops: FakeDrops::default(),
            effects: Vec::new(),
        }
    }

    pub fn with_items(cfg: Arc<Config>, items: Arc<ItemNames>) -> Self {
        Self { items, ..Self::new(cfg) }
    }

    pub fn emit(&mut self, e: Effect) {
        self.effects.push(e);
    }

    /// Time-based work, called every bridge loop.
    pub fn tick(&mut self) {
        let now = Instant::now();
        if let Some(text) = self.features.spam.due(now) {
            self.emit(Effect::chat(&text));
        }
        chat_tools::tick(self, now);
        teleport::tick(self);
        super::collect::tick(self, now);
        super::dropall::tick(self, now);
        settings::autosave_tick(self, now);
        for plan in fast_item::take_expired(&mut self.planned_drops) {
            tracing::info!("drop {}: no dialog from the server, item not owned", plan.name);
            if !plan.quiet {
                commands::warn(self, format!("You have no {}", plan.name));
            }
        }
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    pub fn handle(&mut self, dir: Direction, data: &[u8]) -> Verdict {
        let Some(msg) = Message::decode(data) else {
            tracing::warn!("{} undecodable packet ({} bytes), forwarding as-is", dir.arrow(), data.len());
            return Verdict::Forward;
        };
        match msg {
            Message::ServerHello => {
                tracing::debug!("{} [hello]", dir.arrow());
                Verdict::Forward
            }
            Message::Text { kind, text } => {
                self.log.text(dir, kind_name(kind), &text);
                match dir {
                    Direction::ToServer => self.on_client_text(&text),
                    Direction::ToClient => {
                        if kind == MessageType::Track {
                            self.on_track(&text);
                        }
                        Verdict::Forward
                    }
                }
            }
            Message::Game(pkt) => {
                self.log.game(dir, &pkt);
                match dir {
                    Direction::ToClient => self.on_server_game(pkt),
                    Direction::ToServer => self.on_client_game(pkt),
                }
            }
            Message::Other { kind, body } => {
                tracing::debug!("{} [{kind:?}] {} bytes", dir.arrow(), body.len());
                Verdict::Forward
            }
        }
    }

    fn on_client_text(&mut self, text: &str) -> Verdict {
        let tp = TextPacket::parse(text);
        match tp.get("action") {
            Some("input") => {
                let input = tp.get("text").unwrap_or_default();
                if let Some(line) = input.strip_prefix(self.cfg.command.prefix) {
                    if commands::run(self, line) {
                        return Verdict::Drop;
                    }
                }
                social::note_typed_command(self, input);
                if let Some(new) = chat_tools::rewrite_chat(self, input) {
                    return Verdict::Replace(
                        crate::protocol::message::encode_text(MessageType::GenericText, &format!("action|input\n|text|{new}")),
                    );
                }
            }
            // Answers to our own dialogs mean nothing to the server.
            Some("dialog_return") => {
                if let Some(ret) = DialogReturn::parse(&tp) {
                    commands::on_dialog(self, ret);
                    return Verdict::Drop;
                }
            }
            Some("quit_to_exit") => self.session.leave_world(),
            Some("wrench") => {
                if let Some(v) = tools::auto_wrench(self, &tp) {
                    return v;
                }
            }
            Some(action @ ("drop" | "trash")) => {
                let item = tp.get("itemID").and_then(|v| v.parse().ok());
                self.pending_item = ItemAction::from_action(action).zip(item);
            }
            _ => {}
        }
        Verdict::Forward
    }

    fn on_client_game(&mut self, pkt: GameUpdatePacket) -> Verdict {
        match pkt.packet_type {
            GamePacketType::State => {
                if teleport::hold_client_movement(self) {
                    return Verdict::Drop;
                }
                self.session.pos = Some((pkt.vec_x, pkt.vec_y));
                if pkt.flags.intersects(PacketFlags::PUNCH | PacketFlags::PLACE) {
                    return teleport::on_click(self, pkt.int_x, pkt.int_y, pkt.flags.contains(PacketFlags::PLACE));
                }
                Verdict::Forward
            }
            // 18 = fist; anything else is the held item being placed/used.
            GamePacketType::TileChangeRequest => teleport::on_click(self, pkt.int_x, pkt.int_y, pkt.value != 18),
            GamePacketType::ItemActivateObjectRequest => fake_drop::on_client_pickup(self, pkt),
            _ => Verdict::Forward,
        }
    }

    fn on_server_game(&mut self, pkt: GameUpdatePacket) -> Verdict {
        match pkt.packet_type {
            GamePacketType::CallFunction => {
                let Some(vl) = VariantList::parse(&pkt.extra) else {
                    tracing::debug!("unparsed CallFunction ({} bytes)", pkt.extra.len());
                    return Verdict::Forward;
                };
                self.log.variant(Direction::ToClient, &vl);
                self.on_call(pkt, vl)
            }
            GamePacketType::SetCharacterState => teleport::on_character_state(self, pkt),
            GamePacketType::State => {
                if Some(pkt.net_id) != self.session.net_id && self.session.players.contains_key(&pkt.net_id) {
                    self.session.player_pos.insert(pkt.net_id, (pkt.vec_x, pkt.vec_y));
                }
                Verdict::Forward
            }
            GamePacketType::ItemChangeObject => fake_drop::on_server_object(self, pkt),
            GamePacketType::TileChangeRequest => {
                if let Some(map) = self.world.as_mut() {
                    map.change_tile(pkt.int_x, pkt.int_y, pkt.value);
                }
                Verdict::Forward
            }
            GamePacketType::SendTileUpdateData => {
                let mut vend_changed = false;
                if let Some(map) = self.world.as_mut() {
                    if let Err(e) = map.update_tile(pkt.int_x, pkt.int_y, &pkt.extra) {
                        tracing::debug!("tile update ({},{}) not applied: {e}", pkt.int_x, pkt.int_y);
                    }
                    vend_changed = matches!(map.tile(pkt.int_x, pkt.int_y).map(|t| &t.special), Some(Special::Vend { .. }));
                }
                if vend_changed {
                    super::vends::record_world(self);
                }
                Verdict::Forward
            }
            GamePacketType::SendMapData => {
                self.load_map(&pkt.extra);
                self.session.players.clear();
                self.session.player_pos.clear();
                self.session.player_user.clear();
                if let Some(name) = world_name_from_map(&pkt.extra) {
                    tracing::info!("entered world {name}");
                    self.session.enter_world(name);
                }
                Verdict::Forward
            }
            GamePacketType::SendInventoryState => {
                match Inventory::parse(&pkt.extra) {
                    Some(inv) => {
                        tracing::info!("inventory: {} items, {} slots", inv.len(), inv.slots);
                        self.inventory = inv;
                    }
                    None => tracing::warn!("could not parse inventory ({} bytes)", pkt.extra.len()),
                }
                Verdict::Forward
            }
            GamePacketType::ModifyItemInventory => {
                if pkt.jump_count != 0 {
                    self.inventory.remove(pkt.value, pkt.jump_count as u32);
                } else if pkt.animation_type != 0 {
                    self.inventory.add(pkt.value, pkt.animation_type as u32);
                }
                tracing::debug!(
                    "inventory change: item {} -{} +{}",
                    pkt.value,
                    pkt.jump_count,
                    pkt.animation_type
                );
                Verdict::Forward
            }
            _ => Verdict::Forward,
        }
    }

    fn on_call(&mut self, pkt: GameUpdatePacket, vl: VariantList) -> Verdict {
        match vl.function() {
            Some("OnSendToServer") => match self.rewrite_send_to_server(pkt, vl) {
                Some(bytes) => Verdict::Replace(bytes),
                None => {
                    tracing::error!("could not rewrite OnSendToServer; client will bypass the proxy");
                    Verdict::Forward
                }
            },
            Some("OnSpawn") => self.on_spawn(pkt, vl),
            Some("OnRemove") => {
                let removed = TextPacket::parse(vl.get(1).and_then(Variant::as_str).unwrap_or_default());
                if let Some(id) = removed.get("netID").and_then(|v| v.parse().ok()) {
                    self.session.players.remove(&id);
                    self.session.player_raw.remove(&id);
                    self.session.player_pos.remove(&id);
                    self.session.player_user.remove(&id);
                }
                Verdict::Forward
            }
            Some("OnCountryState") => tools::rewrite_country_state(self, pkt, vl),
            // Remember our real name/skin so /nick and /skin can undo.
            Some("OnNameChanged") if Some(pkt.net_id) == self.session.net_id => {
                if let Some(name) = vl.get(1).and_then(Variant::as_str) {
                    self.session.raw_name = Some(name.to_string());
                }
                Verdict::Forward
            }
            Some("OnChangeSkin") if Some(pkt.net_id) == self.session.net_id => {
                if let Some(Variant::Unsigned(code)) = vl.get(1) {
                    self.session.skin = Some(*code);
                }
                Verdict::Forward
            }
            // [4] is the skin colour.
            Some("OnSetClothing") if Some(pkt.net_id) == self.session.net_id => {
                if let Some(Variant::Unsigned(code)) = vl.get(4) {
                    self.session.skin = Some(*code);
                }
                Verdict::Forward
            }
            Some("OnSetClothing") => social::strip_clothing(self, pkt, vl),
            Some("OnRequestWorldSelectMenu") => {
                self.session.leave_world();
                self.world = None;
                super::warp::on_world_menu(self);
                Verdict::Forward
            }
            Some("OnSetPos") if Some(pkt.net_id) == self.session.net_id => {
                if let Some(Variant::Vec2(x, y)) = vl.get(1) {
                    self.session.pos = Some((*x, *y));
                }
                Verdict::Forward
            }
            Some("OnTalkBubble") => {
                if let Some(v) = super::info::hide_level_text(self, &vl, 2) {
                    return v;
                }
                if let (Some(net_id), Some(text)) = (vl.get(1).and_then(Variant::as_i64), vl.get(2).and_then(Variant::as_str)) {
                    if let Some(chat) = social::chat_text(text) {
                        let chat = chat.to_string();
                        social::on_player_chat(self, net_id as i32, &chat);
                    }
                }
                self.check_spin(pkt, vl, 2)
            }
            Some("OnConsoleMessage") => {
                if let Some(v) = super::info::hide_level_text(self, &vl, 1) {
                    return v;
                }
                if let Some(text) = vl.get(1).and_then(Variant::as_str) {
                    super::info::note_world_owner(self, text);
                }
                self.track_collected(&vl);
                self.check_spin(pkt, vl, 1)
            }
            Some("OnDialogRequest") => self.on_server_dialog(pkt, vl),
            _ => Verdict::Forward,
        }
    }

    fn load_map(&mut self, data: &[u8]) {
        match WorldMap::parse(data) {
            Ok(map) => {
                tracing::info!("world map {} {}x{} read", map.name, map.width, map.height);
                self.fake_drops.reset(map.last_object_uid);
                super::collect::reset(self);
                self.world = Some(map);
                super::vends::record_world(self);
            }
            Err(e) => {
                // Keep the raw map so the parser can be fixed against it.
                tracing::warn!("world map not readable ({e:#}); saved to last_world.dat");
                let _ = std::fs::write("last_world.dat", data);
                self.fake_drops.reset(None);
                self.world = None;
            }
        }
    }

    fn on_spawn(&mut self, pkt: GameUpdatePacket, vl: VariantList) -> Verdict {
        let spawn = TextPacket::parse(vl.get(1).and_then(Variant::as_str).unwrap_or_default());
        let net_id: Option<i32> = spawn.get("netID").and_then(|v| v.parse().ok());
        let name = spawn.get("name").map(strip_colors);
        if spawn.get("type") != Some("local") {
            let (Some(id), Some(name)) = (net_id, name) else { return Verdict::Forward };
            self.session.players.insert(id, name.clone());
            if let Some(user) = spawn.get("userID").and_then(|v| v.parse::<u32>().ok()) {
                self.session.player_user.insert(id, user);
                self.session.known_users.insert(user, name.clone());
            }
            if let Some((x, y)) = spawn.get("posXY").and_then(|v| v.split_once('|')) {
                if let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
                    self.session.player_pos.insert(id, (x, y));
                }
            }
            if let Some(raw) = spawn.get("name") {
                self.session.player_raw.insert(id, raw.to_string());
            }
            social::on_player_joined(self, &name);
            return social::hide_spawn_name(self, pkt, vl, &name).unwrap_or(Verdict::Forward);
        }
        self.session.net_id = net_id;
        self.session.user_id = spawn.get("userID").and_then(|v| v.parse().ok());
        if let (Some(user), Some(n)) = (self.session.user_id, name.as_ref()) {
            self.session.known_users.insert(user as u32, n.clone());
        }
        self.session.name = name;
        self.session.raw_name = spawn.get("name").map(String::from);
        if let Some(c) = spawn.get("country") {
            self.session.country = c.split('|').next().map(String::from);
        }
        tools::rewrite_local_spawn(self, pkt, vl)
    }

    /// "Collected `w5 Dirt``" -> inventory.
    fn track_collected(&mut self, vl: &VariantList) {
        let Some((amount, name)) = vl.get(1).and_then(Variant::as_str).and_then(inventory::parse_collected) else {
            return;
        };
        match self.items.find(&name) {
            Some(id) => self.inventory.add(id, amount),
            None => tracing::debug!("collected unknown item {name:?}"),
        }
    }

    /// Account numbers from the analytics events the server sends.
    fn on_track(&mut self, text: &str) {
        let tp = TextPacket::parse(text);
        if let Some(level) = tp.get("Level").and_then(|v| v.parse().ok()) {
            self.session.level = Some(level);
        }
        if let Some(gems) = tp.get("Gems_balance").and_then(|v| v.parse().ok()) {
            self.session.gems = Some(gems);
        }
    }

    fn on_server_dialog(&mut self, pkt: GameUpdatePacket, mut vl: VariantList) -> Verdict {
        let markup = vl.get(1).and_then(Variant::as_str).unwrap_or_default().to_string();
        if markup.contains("Growtopia Gazette") {
            self.features.news = Some(markup.clone());
        }
        if let Some(v) = tools::auto_change(self, &markup) {
            return v;
        }
        match self.skip_item_dialog(&vl) {
            Verdict::Forward => {}
            other => return other,
        }
        // `/ui`: restyle every dialog the server opens.
        if self.features.ui_color.is_some() {
            vl.0[1] = Variant::String(self.style_dialog(markup));
            let mut pkt = pkt;
            pkt.extra = vl.serialize();
            return Verdict::Replace(Message::Game(pkt).encode());
        }
        Verdict::Forward
    }

    /// Appends the `/ui` colours to dialog markup.
    pub fn style_dialog(&self, mut markup: String) -> String {
        if let Some([r, g, b, a]) = self.features.ui_color {
            if !markup.ends_with('\n') {
                markup.push('\n');
            }
            markup.push_str(&format!("set_bg_color|{r},{g},{b},{a}|\nset_border_color|{r},{g},{b},255|\n"));
        }
        markup
    }

    /// Fast drop / trash: answer the "How many?" dialog with the full stack instead of showing it.
    fn skip_item_dialog(&mut self, vl: &VariantList) -> Verdict {
        let Some(markup) = vl.get(1).and_then(Variant::as_str) else { return Verdict::Forward };
        let Some(dialog) = fast_item::parse_dialog(markup) else { return Verdict::Forward };
        if let Some(plan) = fast_item::take_plan(&mut self.planned_drops, &dialog) {
            return self.answer_planned_drop(plan, dialog.count);
        }
        let pending = self.pending_item.take();
        let enabled = match dialog.action {
            ItemAction::Drop => self.fast_drop,
            ItemAction::Trash => self.fast_trash,
        };
        if !enabled {
            return Verdict::Forward;
        }
        tracing::debug!("{:?} dialog:
{markup}", dialog.action);

        let item_id = dialog
            .item_id
            .or_else(|| pending.filter(|(a, _)| *a == dialog.action).map(|(_, id)| id));
        let (Some(item_id), Some(count)) = (item_id, dialog.count.filter(|&c| c > 0)) else {
            tracing::warn!("fast {:?}: could not read item/count from the dialog, showing it", dialog.action);
            return Verdict::Forward;
        };
        tracing::info!("fast {:?}: {count} x item {item_id}", dialog.action);
        self.emit(Effect::text(fast_item::dialog_return(dialog.action, item_id, count)));
        let name = dialog.item_name.unwrap_or_else(|| format!("item #{item_id}"));
        commands::say(self, format!("{} `w{count} {name}``", dialog.action.verb()));
        Verdict::Drop
    }

    /// Roulette result in argument `text_index`: show it now and mark it `[REAL]`.
    fn check_spin(&mut self, mut pkt: GameUpdatePacket, mut vl: VariantList, text_index: usize) -> Verdict {
        if !self.roulette.enabled {
            return Verdict::Forward;
        }
        let Some(spin) = vl.get(text_index).and_then(Variant::as_str).and_then(roulette::parse_spin) else {
            return Verdict::Forward;
        };
        tracing::info!(
            "spin: {} got {} (delay {}ms, net_id {}, {})",
            spin.player,
            spin.number,
            pkt.value,
            pkt.net_id,
            vl
        );
        // The bubble and the console line carry the same spin; count it once.
        if text_index == 2 {
            self.roulette_stats.record(&spin);
            super::history::record_spin(self, &spin);
        }
        let text = vl.get(text_index).and_then(Variant::as_str).unwrap_or_default();
        vl.0[text_index] = Variant::String(roulette::annotate(text, &spin, self.roulette.mode));
        pkt.extra = vl.serialize();
        if self.roulette.instant {
            pkt.value = 0; // CallFunction delay in ms
        }
        Verdict::Replace(Message::Game(pkt).encode())
    }

    /// Answers the dialog of a proxy-requested drop; `owned` is the dialog's pre-filled count.
    fn answer_planned_drop(&mut self, plan: PlannedDrop, owned: Option<u32>) -> Verdict {
        let Some(owned) = owned.filter(|&c| c > 0) else {
            tracing::warn!("drop {}: no count in the dialog, showing it", plan.name);
            return Verdict::Forward;
        };
        let amount = plan.amount.unwrap_or(owned);
        if amount > owned {
            commands::warn(self, format!("You only have {owned} {}", plan.name));
        } else {
            tracing::info!("drop {amount} x {}", plan.name);
            self.emit(Effect::text(fast_item::dialog_return(ItemAction::Drop, plan.item_id, amount)));
            if !plan.quiet {
                commands::say(self, format!("Dropped `w{amount} {}``", plan.name));
            }
        }
        Verdict::Drop
    }

    /// Points the client's sub-server hop at the proxy and remembers the real target.
    ///
    /// Layout: `[1] port`, `[4] "ip|doorID|UUIDToken"`, other entries untouched.
    fn rewrite_send_to_server(&mut self, mut pkt: GameUpdatePacket, mut vl: VariantList) -> Option<Vec<u8>> {
        let port = u16::try_from(vl.get(1)?.as_i64()?).ok()?;
        let target = vl.get(4)?.as_str()?.to_string();
        let (ip, rest) = match target.split_once('|') {
            Some((ip, rest)) => (ip, Some(rest)),
            None => (target.as_str(), None),
        };
        let real = (ip, port).to_socket_addrs().ok()?.find(SocketAddr::is_ipv4)?;
        tracing::info!("OnSendToServer: sub-server {real}, rerouting through proxy");
        self.redirect = Some(real);

        let local_port = self.cfg.proxy.enet_port;
        vl.0[1] = match vl.0[1] {
            Variant::Unsigned(_) => Variant::Unsigned(local_port as u32),
            _ => Variant::Signed(local_port as i32),
        };
        let local_ip = &self.cfg.proxy.local_ip;
        vl.0[4] = Variant::String(match rest {
            Some(rest) => format!("{local_ip}|{rest}"),
            None => local_ip.clone(),
        });
        pkt.extra = vl.serialize();
        Some(Message::Game(pkt).encode())
    }
}

/// Map data starts with `u16 version, u32 flags, u16 name_len, name`.
fn world_name_from_map(d: &[u8]) -> Option<String> {
    let len = u16::from_le_bytes(d.get(6..8)?.try_into().ok()?) as usize;
    let name = d.get(8..8 + len)?;
    (!name.is_empty()).then(|| String::from_utf8_lossy(name).into_owned())
}

fn kind_name(kind: MessageType) -> &'static str {
    match kind {
        MessageType::GenericText => "text",
        MessageType::GameMessage => "action",
        MessageType::Track => "track",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::message::encode_text;

    fn interceptor() -> Interceptor {
        let mut cfg = Config::default();
        cfg.log.text = false;
        cfg.log.variant = false;
        Interceptor::new(Arc::new(cfg))
    }

    fn call(items: Vec<Variant>) -> Vec<u8> {
        Message::Game(GameUpdatePacket::call_function(-1, VariantList::new(items).serialize())).encode()
    }

    #[test]
    fn rewrites_send_to_server() {
        let mut ic = interceptor();
        let raw = call(vec![
            Variant::String("OnSendToServer".into()),
            Variant::Signed(17040),
            Variant::Signed(4744778),
            Variant::Signed(184573909),
            Variant::String("213.179.209.175|0|02560A89C0D794A9".into()),
            Variant::Signed(1),
            Variant::String("Player".into()),
        ]);
        let Verdict::Replace(out) = ic.handle(Direction::ToClient, &raw) else { panic!("not rewritten") };
        assert_eq!(ic.redirect, Some("213.179.209.175:17040".parse().unwrap()));

        let Some(Message::Game(pkt)) = Message::decode(&out) else { panic!() };
        let vl = VariantList::parse(&pkt.extra).unwrap();
        assert_eq!(vl.get(1), Some(&Variant::Signed(17091)));
        assert_eq!(vl.get(2), Some(&Variant::Signed(4744778)));
        assert_eq!(vl.get(4).and_then(Variant::as_str), Some("127.0.0.1|0|02560A89C0D794A9"));
        assert_eq!(vl.get(6).and_then(Variant::as_str), Some("Player"));
    }

    #[test]
    fn commands_are_swallowed_but_chat_is_not() {
        let mut ic = interceptor();
        let cmd = encode_text(MessageType::GenericText, "action|input\n|text|/proxyhelp");
        assert_eq!(ic.handle(Direction::ToServer, &cmd), Verdict::Drop);
        assert_eq!(opened_dialogs(ic.take_effects()).len(), 1);

        // /help is the game's own command now.
        let game_help = encode_text(MessageType::GenericText, "action|input\n|text|/help");
        assert_eq!(ic.handle(Direction::ToServer, &game_help), Verdict::Forward);

        let chat = encode_text(MessageType::GenericText, "action|input\n|text|hello");
        assert_eq!(ic.handle(Direction::ToServer, &chat), Verdict::Forward);

        // Unknown commands belong to the game (/msg, /sb, ...).
        let game_cmd = encode_text(MessageType::GenericText, "action|input\n|text|/msg bob hi");
        assert_eq!(ic.handle(Direction::ToServer, &game_cmd), Verdict::Forward);
    }

    fn opened_dialogs(effects: Vec<Effect>) -> Vec<String> {
        effects
            .into_iter()
            .filter_map(|e| match e {
                Effect::ToClient(b) => match Message::decode(&b) {
                    Some(Message::Game(p)) => {
                        let vl = VariantList::parse(&p.extra)?;
                        (vl.function() == Some("OnDialogRequest")).then(|| vl.get(1)?.as_str().map(String::from))?
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn proxy_command_opens_status_dialog() {
        let mut ic = interceptor();
        ic.session.enter_world("START".into());
        let cmd = encode_text(MessageType::GenericText, "action|input\n|text|/proxy");
        assert_eq!(ic.handle(Direction::ToServer, &cmd), Verdict::Drop);
        let dialogs = opened_dialogs(ic.take_effects());
        assert_eq!(dialogs.len(), 1);
        assert!(dialogs[0].contains("end_dialog|gtproxy_status|Close||"));
        assert!(dialogs[0].contains("`wSTART``"));
    }

    #[test]
    fn own_dialog_returns_stay_in_proxy() {
        let mut ic = interceptor();
        let refresh = encode_text(
            MessageType::GenericText,
            "action|dialog_return\ndialog_name|gtproxy_status\nbuttonClicked|commands\n",
        );
        assert_eq!(ic.handle(Direction::ToServer, &refresh), Verdict::Drop);
        let dialogs = opened_dialogs(ic.take_effects());
        assert!(dialogs[0].contains("end_dialog|gtproxy_commands|"));

        let before = ic.log.variant;
        let toggle = encode_text(
            MessageType::GenericText,
            "action|dialog_return\ndialog_name|gtproxy_status\nbuttonClicked|log_variant\n",
        );
        assert_eq!(ic.handle(Direction::ToServer, &toggle), Verdict::Drop);
        assert_eq!(ic.log.variant, !before);

        // The game's own dialogs still go to the server.
        let sign = encode_text(MessageType::GenericText, "action|dialog_return\ndialog_name|sign_edit\ntilex|1|\n");
        assert_eq!(ic.handle(Direction::ToServer, &sign), Verdict::Forward);
    }

    #[test]
    fn fast_drop_answers_dialog_with_full_stack() {
        let mut ic = interceptor();
        let drop_dialog = call(vec![
            Variant::String("OnDialogRequest".into()),
            Variant::String(
                "add_label_with_icon|big|`wDrop Dirt``|left|2|\nadd_text_input|count||200|5|\nend_dialog|drop_item|Cancel|OK|\n"
                    .into(),
            ),
        ]);

        // Off: the dialog reaches the client as usual.
        assert_eq!(ic.handle(Direction::ToClient, &drop_dialog), Verdict::Forward);

        ic.fast_drop = true;
        let request = encode_text(MessageType::GenericText, "action|drop\n|itemID|2");
        assert_eq!(ic.handle(Direction::ToServer, &request), Verdict::Forward);
        assert_eq!(ic.handle(Direction::ToClient, &drop_dialog), Verdict::Drop);
        let sent: Vec<String> = ic
            .take_effects()
            .into_iter()
            .filter_map(|e| match e {
                Effect::ToServer(b) => Some(String::from_utf8_lossy(&b[4..b.len() - 1]).into_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(sent, vec!["action|dialog_return\ndialog_name|drop_item\nitemID|2|\ncount|200\n"]);

        // Trash stays untouched while only fast drop is on.
        let trash_dialog = call(vec![
            Variant::String("OnDialogRequest".into()),
            Variant::String("add_text_input|count||5|5|\nembed_data|itemID|2\nend_dialog|trash_item|Cancel|OK|\n".into()),
        ]);
        assert_eq!(ic.handle(Direction::ToClient, &trash_dialog), Verdict::Forward);
    }

    fn server_texts(effects: Vec<Effect>) -> Vec<String> {
        effects
            .into_iter()
            .filter_map(|e| match e {
                Effect::ToServer(b) => Some(String::from_utf8_lossy(&b[4..b.len() - 1]).into_owned()),
                _ => None,
            })
            .collect()
    }

    fn wl_dialog(owned: u32) -> Vec<u8> {
        call(vec![
            Variant::String("OnDialogRequest".into()),
            Variant::String(format!(
                "add_label_with_icon|big|`wDrop World Lock``|left|242|\nadd_text_input|count||{owned}|5|\nembed_data|itemID|242\nend_dialog|drop_item|Cancel|OK|\n"
            )),
        ])
    }

    #[test]
    fn drop_lock_command_checks_what_you_own() {
        let mut ic = interceptor();
        let cmd = encode_text(MessageType::GenericText, "action|input\n|text|/dw 10");
        assert_eq!(ic.handle(Direction::ToServer, &cmd), Verdict::Drop);
        assert_eq!(server_texts(ic.take_effects()), vec!["action|drop\n|itemID|242\n"]);

        // 23 owned: 10 are dropped, dialog hidden.
        assert_eq!(ic.handle(Direction::ToClient, &wl_dialog(23)), Verdict::Drop);
        assert_eq!(
            server_texts(ic.take_effects()),
            vec!["action|dialog_return\ndialog_name|drop_item\nitemID|242|\ncount|10\n"]
        );

        // Only 5 owned: nothing is dropped.
        ic.handle(Direction::ToServer, &cmd);
        ic.take_effects();
        assert_eq!(ic.handle(Direction::ToClient, &wl_dialog(5)), Verdict::Drop);
        assert!(server_texts(ic.take_effects()).is_empty());

        // No plan left: a manual drop dialog is shown normally.
        assert_eq!(ic.handle(Direction::ToClient, &wl_dialog(5)), Verdict::Forward);
    }

    #[test]
    fn drop_all_locks_uses_whole_stacks() {
        let mut ic = interceptor();
        let cmd = encode_text(MessageType::GenericText, "action|input\n|text|/daw");
        ic.handle(Direction::ToServer, &cmd);
        assert_eq!(ic.planned_drops.len(), 3);
        ic.take_effects();
        assert_eq!(ic.handle(Direction::ToClient, &wl_dialog(37)), Verdict::Drop);
        assert_eq!(
            server_texts(ic.take_effects()),
            vec!["action|dialog_return\ndialog_name|drop_item\nitemID|242|\ncount|37\n"]
        );
        assert_eq!(ic.planned_drops.len(), 2);
    }

    #[test]
    fn spin_shown_instantly_and_marked() {
        let mut ic = interceptor();
        ic.roulette.mode = super::roulette::SpinMode::Reme;
        let mut pkt = GameUpdatePacket::call_function(
            2,
            VariantList::new(vec![
                Variant::String("OnTalkBubble".into()),
                Variant::Unsigned(2),
                Variant::String("`7[```wBob`` spun the wheel and got `429``!`7]``".into()),
                Variant::Unsigned(0),
            ])
            .serialize(),
        );
        pkt.value = 2500;
        let Verdict::Replace(out) = ic.handle(Direction::ToClient, &Message::Game(pkt).encode()) else {
            panic!("spin not rewritten")
        };
        let Some(Message::Game(out)) = Message::decode(&out) else { panic!() };
        assert_eq!(out.value, 0);
        assert_eq!(out.net_id, 2);
        let vl = VariantList::parse(&out.extra).unwrap();
        assert_eq!(
            vl.get(2).and_then(Variant::as_str),
            Some("[`2REAL``] `7[```wBob`` spun the wheel and got `429``!`7]`` [`2REME`` 1]")
        );
        assert_eq!(vl.get(3), Some(&Variant::Unsigned(0)));

        // A chat line that only looks similar is left alone.
        let chat = call(vec![
            Variant::String("OnTalkBubble".into()),
            Variant::Unsigned(3),
            Variant::String("Bob spun the wheel and got 29!".into()),
        ]);
        assert_eq!(ic.handle(Direction::ToClient, &chat), Verdict::Forward);
    }

    #[test]
    fn tracks_world_and_local_player() {
        let mut ic = interceptor();
        let mut map = vec![0x19, 0, 0, 0, 0, 0, 5, 0];
        map.extend_from_slice(b"START");
        map.extend_from_slice(&[0; 16]);
        let pkt = GameUpdatePacket {
            packet_type: GamePacketType::SendMapData,
            flags: crate::protocol::game_packet::PacketFlags::EXTENDED,
            extra: map,
            ..Default::default()
        };
        ic.handle(Direction::ToClient, &Message::Game(pkt).encode());
        assert_eq!(ic.session.world.as_deref(), Some("START"));

        let spawn = call(vec![
            Variant::String("OnSpawn".into()),
            Variant::String("spawn|avatar\nnetID|7\nuserID|42\nname|`#@Serhat``\ntype|local\n".into()),
        ]);
        ic.handle(Direction::ToClient, &spawn);
        assert_eq!(ic.session.net_id, Some(7));
        assert_eq!(ic.session.name.as_deref(), Some("@Serhat"));
    }
}
