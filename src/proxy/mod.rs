//! ENet bridge: game client <-> proxy <-> real server.
//!
//! Two ENet hosts run on one thread:
//! - `downstream` listens for the game client (`using_new_packet_server`)
//! - `upstream` connects to the real server (`using_new_packet`)
//!
//! Each client connection gets its own upstream connection. The target is the
//! sub-server from the last `OnSendToServer`, otherwise the main server from
//! the last server_data.php answer.

mod commands;
mod dialog;
mod effect;
mod fast_item;
mod features;
mod inventory;
mod locks;
mod tools;
mod chat_tools;
mod settings;
mod social;
mod stats;
mod worlds;
mod world;
mod teleport;
mod zoom;
mod fake_drop;
mod scan;
mod vends;
mod warp;
mod history;
mod info;
mod pathing;
mod collect;
mod dropall;
#[cfg(test)]
mod feature_tests;
mod interceptor;
mod log;
mod roulette;
mod session;

use anyhow::{Context, Result};
use rusty_enet as enet;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::items::ItemNames;
use crate::state::Shared;
use effect::{Effect, Scheduler};
use interceptor::{Interceptor, Verdict};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Game client -> server.
    ToServer,
    /// Server -> game client.
    ToClient,
}

impl Direction {
    pub fn arrow(self) -> &'static str {
        match self {
            Self::ToServer => "client -> server",
            Self::ToClient => "server -> client",
        }
    }
}

type Host = enet::Host<UdpSocket>;

struct Pending {
    channel: u8,
    data: Vec<u8>,
    kind: enet::PacketKind,
}

struct Bridge {
    shared: Arc<Shared>,
    downstream: Host,
    upstream: Host,
    game: Option<enet::PeerID>,
    server: Option<enet::PeerID>,
    server_ready: bool,
    /// Client packets that arrived before the upstream connection was up.
    backlog: Vec<Pending>,
    /// Fallback when a client connects without a fresh server_data / redirect.
    last_target: Option<SocketAddr>,
    ic: Interceptor,
    scheduler: Scheduler,
}

pub fn run(cfg: Arc<Config>, shared: Arc<Shared>) -> Result<()> {
    let bind = format!("{}:{}", cfg.proxy.local_ip, cfg.proxy.enet_port);
    let down_sock = UdpSocket::bind(&bind).with_context(|| format!("cannot bind ENet on {bind}"))?;
    let downstream = Host::new(down_sock, host_settings(4, true)).context("failed to create ENet server")?;
    let up_sock = UdpSocket::bind("0.0.0.0:0")?;
    // One slot: the integrity bytes always use slot 0, so it must be the live server peer.
    let upstream = Host::new(up_sock, host_settings(1, false)).context("failed to create ENet client")?;
    tracing::info!("ENet proxy listening on {bind}");

    let mut ic = Interceptor::with_items(cfg.clone(), Arc::new(ItemNames::load()));
    ic.store = settings::Store::new(cfg.system.settings_path.clone().into());
    settings::load(&mut ic);
    ic.vends = vends::VendDb::load("vends.json".into());

    let mut bridge = Bridge {
        shared,
        downstream,
        upstream,
        game: None,
        server: None,
        server_ready: false,
        backlog: Vec::new(),
        last_target: None,
        ic,
        scheduler: Scheduler::default(),
    };
    bridge.run();
    Ok(())
}

fn host_settings(peers: usize, server_side: bool) -> enet::HostSettings {
    enet::HostSettings {
        peer_limit: peers,
        channel_limit: 2,
        compressor: Some(Box::new(enet::RangeCoder::new())),
        checksum: Some(Box::new(enet::crc32)),
        using_new_packet: !server_side,
        using_new_packet_server: server_side,
        ..Default::default()
    }
}

/// How often the window's snapshot is refreshed.
const PUBLISH_EVERY: Duration = Duration::from_millis(250);

impl Bridge {
    fn run(&mut self) {
        let mut published = std::time::Instant::now() - PUBLISH_EVERY;
        while !self.shared.stopped() {
            for line in self.shared.take_commands() {
                if !commands::run(&mut self.ic, &line) {
                    tracing::warn!("unknown command from the window: {line}");
                }
            }
            if published.elapsed() >= PUBLISH_EVERY {
                published = std::time::Instant::now();
                self.shared.publish(self.snapshot());
            }
            self.pump_downstream();
            self.pump_upstream();
            for e in self.scheduler.due() {
                self.apply(e);
            }
            self.ic.tick();
            self.apply_effects();
            self.update_link();
            self.flush();
            std::thread::sleep(Duration::from_millis(1));
        }

        tracing::info!("stopping ENet bridge");
        if let Some(id) = self.game.take() {
            self.downstream.peer_mut(id).disconnect_now(0);
        }
        if let Some(id) = self.server.take() {
            self.upstream.peer_mut(id).disconnect_now(0);
        }
        self.flush();
    }

    fn snapshot(&self) -> crate::state::Snapshot {
        let ic = &self.ic;
        let f = &ic.features;
        let s = &ic.session;
        let ms = |d: Option<Duration>| d.map(|d| d.as_millis() as u32);
        let me = s.net_id;
        crate::state::Snapshot {
            game_connected: self.game.is_some(),
            server_connected: self.server_ready,
            server: s.server.map(|a| a.to_string()),
            client_ping_ms: ms(ic.link.client_rtt),
            server_ping_ms: ms(ic.link.server_rtt),
            name: s.name.clone(),
            world: s.world.clone(),
            level: s.level,
            gems: s.gems,
            locks: (ic.inventory.count(locks::WL), ic.inventory.count(locks::DL), ic.inventory.count(locks::BGL)),
            inventory_items: ic.inventory.len(),
            players: s.players.iter().filter(|(id, _)| Some(**id) != me).map(|(k, v)| (*k, v.clone())).collect(),
            saved_worlds: f.saved_worlds.iter().map(|w| (w.target(), w.description.clone())).collect(),
            fast_drop: ic.fast_drop,
            fast_trash: ic.fast_trash,
            teleport: ic.teleport.enabled,
            teleport_step_ms: ic.teleport.step.as_millis() as u64,
            wheel_zoom: zoom::enabled(),
            hide_clothes: f.hide_clothes,
            me: f.me,
            formalize: f.formalize,
            auto_change: f.auto_change,
            autosave: f.autosave,
            wrench: f.wrench.map(|w| w.command().to_string()),
            ping_alert_ms: f.ping_alert_ms,
            spam_running: f.spam.running,
            auto_pull: f.auto_pull.enabled,
            auto_ban: f.auto_ban.enabled,
            ban_words: f.ban_words.enabled,
            auto_collect: ic.collect.auto,
            collect_range: ic.collect.range,
            collect_only: ic.collect.only.map(|id| ic.items.name(id)),
            minimal: f.minimal,
            hide_level: f.hide_level,
            track_drops: ic.history.track,
            vends_known: ic.vends.worlds.values().map(Vec::len).sum(),
            spin_history: ic.history.spins.iter().take(12).map(|e| (e.player.clone(), e.number, e.colour)).collect(),
            roulette_enabled: ic.roulette.enabled,
            roulette_instant: ic.roulette.instant,
            roulette_mode: ic.roulette.mode.name().to_string(),
            spins: crate::state::SpinSummary {
                total: ic.roulette_stats.total,
                red: ic.roulette_stats.red,
                black: ic.roulette_stats.black,
                green: ic.roulette_stats.green,
                recent: ic.roulette_stats.recent(),
            },
        }
    }

    fn flush(&mut self) {
        self.downstream.flush();
        // See `pump_upstream`: the upstream host must not send before its first connect.
        if self.last_target.is_some() {
            self.upstream.flush();
        }
    }

    fn pump_downstream(&mut self) {
        loop {
            let event = match self.downstream.service() {
                Ok(Some(e)) => e.no_ref(),
                Ok(None) => break,
                Err(e) => {
                    log_socket_error("server", &e);
                    break;
                }
            };
            match event {
                enet::EventNoRef::Connect { peer, .. } => self.on_game_connect(peer),
                enet::EventNoRef::Disconnect { peer, .. } => self.on_game_disconnect(peer),
                enet::EventNoRef::Receive { peer, channel_id, packet } => {
                    if self.game != Some(peer) {
                        continue;
                    }
                    let verdict = self.ic.handle(Direction::ToServer, packet.data());
                    self.apply_effects();
                    match verdict {
                        Verdict::Forward => self.to_server(channel_id, packet.data().to_vec(), packet.kind()),
                        Verdict::Replace(data) => self.to_server(channel_id, data, packet.kind()),
                        Verdict::Drop => {}
                    }
                }
            }
        }
    }

    fn pump_upstream(&mut self) {
        // With `using_new_packet`, rusty_enet derives the header integrity bytes
        // from peer slot 0's address and panics while that slot has never been used.
        if self.last_target.is_none() {
            return;
        }
        loop {
            let event = match self.upstream.service() {
                Ok(Some(e)) => e.no_ref(),
                Ok(None) => break,
                Err(e) => {
                    log_socket_error("client", &e);
                    break;
                }
            };
            match event {
                enet::EventNoRef::Connect { peer, .. } => {
                    if self.server != Some(peer) {
                        continue;
                    }
                    tracing::info!("connected to server {}", fmt_addr(self.ic.session.server));
                    self.server_ready = true;
                    for p in std::mem::take(&mut self.backlog) {
                        self.to_server(p.channel, p.data, p.kind);
                    }
                }
                enet::EventNoRef::Disconnect { peer, .. } => {
                    if self.server != Some(peer) {
                        continue;
                    }
                    tracing::info!("server {} closed the connection", fmt_addr(self.ic.session.server));
                    self.server = None;
                    self.server_ready = false;
                    // `later` so queued packets (e.g. OnSendToServer) still reach the client.
                    if let Some(id) = self.game {
                        self.downstream.peer_mut(id).disconnect_later(0);
                    }
                }
                enet::EventNoRef::Receive { peer, channel_id, packet } => {
                    if self.server != Some(peer) {
                        continue;
                    }
                    let verdict = self.ic.handle(Direction::ToClient, packet.data());
                    self.apply_effects();
                    match verdict {
                        Verdict::Forward => self.to_client(channel_id, packet.data().to_vec(), packet.kind()),
                        Verdict::Replace(data) => self.to_client(channel_id, data, packet.kind()),
                        Verdict::Drop => {}
                    }
                }
            }
        }
    }

    fn on_game_connect(&mut self, peer: enet::PeerID) {
        let addr = self.downstream.peer_mut(peer).address();
        tracing::info!("game client connected from {}", fmt_addr(addr));

        if let Some(old) = self.game.replace(peer) {
            if old != peer {
                self.downstream.peer_mut(old).disconnect_now(0);
            }
        }
        if let Some(old) = self.server.take() {
            self.upstream.peer_mut(old).disconnect_now(0);
        }
        self.server_ready = false;
        self.backlog.clear();
        self.scheduler.clear();

        let target = self
            .ic
            .redirect
            .take()
            .or_else(|| self.shared.take_server_data_upstream())
            .or_else(|| {
                let last = self.last_target;
                if let Some(a) = last {
                    tracing::warn!("no new server_data / redirect, reusing last server {a}");
                }
                last
            });
        let Some(target) = target else {
            tracing::error!("client connected but no game server is known yet (server_data.php did not go through the proxy)");
            self.downstream.peer_mut(peer).disconnect(0);
            self.game = None;
            return;
        };

        match self.upstream.connect(target, 2, 0) {
            Ok(p) => {
                self.server = Some(p.id());
                self.last_target = Some(target);
                self.ic.session.server = Some(target);
                tracing::info!("connecting to server {target}");
            }
            Err(e) => {
                tracing::error!("cannot connect to {target}: {e}");
                self.downstream.peer_mut(peer).disconnect(0);
                self.game = None;
            }
        }
    }

    fn on_game_disconnect(&mut self, peer: enet::PeerID) {
        if self.game != Some(peer) {
            return;
        }
        tracing::info!("game client disconnected");
        self.game = None;
        self.scheduler.clear();
        // `_now` frees the single upstream slot at once; the client usually
        // reconnects (sub-server hop) within milliseconds.
        if let Some(id) = self.server.take() {
            self.upstream.peer_mut(id).disconnect_now(0);
        }
        self.server_ready = false;
        self.backlog.clear();
    }

    fn to_server(&mut self, channel: u8, data: Vec<u8>, kind: enet::PacketKind) {
        match self.server {
            Some(id) if self.server_ready => {
                let _ = self.upstream.peer_mut(id).send(channel, &enet::Packet::new(data, kind));
            }
            Some(_) => self.backlog.push(Pending { channel, data, kind }),
            None => tracing::debug!("dropping client packet: no server connection"),
        }
    }

    fn to_client(&mut self, channel: u8, data: Vec<u8>, kind: enet::PacketKind) {
        if let Some(id) = self.game {
            let _ = self.downstream.peer_mut(id).send(channel, &enet::Packet::new(data, kind));
        }
    }

    fn apply_effects(&mut self) {
        for e in self.ic.take_effects() {
            self.apply(e);
        }
    }

    fn apply(&mut self, e: Effect) {
        match e {
            Effect::ToClient(data) => self.to_client(0, data, enet::PacketKind::Reliable),
            Effect::ToServer(data) => self.to_server(0, data, enet::PacketKind::Reliable),
            Effect::Later(delay, inner) => self.scheduler.push(delay, *inner),
        }
    }

    fn update_link(&mut self) {
        let rtt = |host: &mut Host, id: Option<enet::PeerID>| {
            id.map(|id| host.peer_mut(id).round_trip_time())
        };
        self.ic.link.client_rtt = rtt(&mut self.downstream, self.game);
        self.ic.link.server_rtt = if self.server_ready { rtt(&mut self.upstream, self.server) } else { None };
    }
}

/// On Windows a UDP socket reports WSAECONNRESET (10054) when an earlier send
/// hit a closed port, e.g. the game's old socket after a sub-server hop. It
/// carries no data and the socket keeps working, so it is not worth an error.
fn log_socket_error(side: &str, e: &std::io::Error) {
    if e.kind() == std::io::ErrorKind::ConnectionReset {
        tracing::debug!("ENet {side} socket: peer port closed (ignored)");
    } else {
        tracing::error!("ENet {side} socket error: {e}");
    }
}

fn fmt_addr(a: Option<SocketAddr>) -> String {
    a.map_or_else(|| "?".to_string(), |a| a.to_string())
}

#[cfg(test)]
mod tests {
    //! End-to-end: fake game client <-> real Bridge <-> fake game servers, all on localhost.

    use super::*;
    use crate::protocol::message::encode_text;
    use crate::protocol::{GameUpdatePacket, Message, MessageType, Variant, VariantList};
    use std::time::Instant;

    const PROXY_PORT: u16 = 27_391;

    fn host(server_side: bool, bind: &str) -> Host {
        Host::new(UdpSocket::bind(bind).unwrap(), host_settings(4, server_side)).unwrap()
    }

    /// Services `hosts` until `f` returns Some or the deadline passes.
    fn wait<T>(hosts: &mut [&mut Host], mut f: impl FnMut(usize, enet::EventNoRef) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            for (i, h) in hosts.iter_mut().enumerate() {
                while let Ok(Some(e)) = h.service() {
                    if let Some(v) = f(i, e.no_ref()) {
                        return v;
                    }
                }
                h.flush();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("timed out");
    }

    fn text_of(data: &[u8]) -> String {
        match Message::decode(data) {
            Some(Message::Text { text, .. }) => text,
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn bridges_and_follows_sub_server_redirect() {
        let mut cfg = Config::default();
        cfg.proxy.enet_port = PROXY_PORT;
        cfg.log.text = false;
        cfg.log.variant = false;
        let cfg = Arc::new(cfg);
        let shared = Arc::new(Shared::default());

        let mut main_srv = host(true, "127.0.0.1:0");
        let mut sub_srv = host(true, "127.0.0.1:0");
        let main_addr = main_srv.socket().local_addr().unwrap();
        let sub_addr = sub_srv.socket().local_addr().unwrap();
        shared.set_server_data_upstream(main_addr);

        let bridge = std::thread::spawn({
            let (cfg, shared) = (cfg.clone(), shared.clone());
            move || run(cfg, shared).unwrap()
        });
        std::thread::sleep(Duration::from_millis(200));

        // 1. client connects -> proxy connects to main server -> hello reaches client.
        let mut client = host(false, "127.0.0.1:0");
        let proxy_addr: SocketAddr = format!("127.0.0.1:{PROXY_PORT}").parse().unwrap();
        client.connect(proxy_addr, 2, 0).unwrap();
        let srv_peer = wait(&mut [&mut client, &mut main_srv], |i, e| match (i, e) {
            (1, enet::EventNoRef::Connect { peer, .. }) => Some(peer),
            _ => None,
        });
        main_srv
            .peer_mut(srv_peer)
            .send(0, &enet::Packet::reliable(Message::ServerHello.encode()))
            .unwrap();
        let cli_peer = wait(&mut [&mut client, &mut main_srv], |i, e| match (i, e) {
            (0, enet::EventNoRef::Receive { peer, packet, .. }) => {
                assert!(matches!(Message::decode(packet.data()), Some(Message::ServerHello)));
                Some(peer)
            }
            _ => None,
        });

        // 2. client -> server forwarding; proxy commands are swallowed and answered locally.
        let send = |c: &mut Host, text: &str| {
            c.peer_mut(cli_peer)
                .send(0, &enet::Packet::reliable(encode_text(MessageType::GenericText, text)))
                .unwrap();
        };
        send(&mut client, "action|input\n|text|/proxy");
        send(&mut client, "action|input\n|text|hello");
        let mut got_console = false;
        let forwarded = wait(&mut [&mut client, &mut main_srv], |i, e| match (i, e) {
            (0, enet::EventNoRef::Receive { packet, .. }) => {
                if let Some(Message::Game(p)) = Message::decode(packet.data()) {
                    let vl = VariantList::parse(&p.extra).unwrap();
                    got_console |= vl.function() == Some("OnDialogRequest");
                }
                None
            }
            (1, enet::EventNoRef::Receive { packet, .. }) => Some(text_of(packet.data())),
            _ => None,
        });
        assert_eq!(forwarded, "action|input\n|text|hello");

        // 3. OnSendToServer: client sees the proxy address, proxy remembers the sub-server.
        let redirect = VariantList::new(vec![
            Variant::String("OnSendToServer".into()),
            Variant::Signed(sub_addr.port() as i32),
            Variant::Signed(1234),
            Variant::Signed(5678),
            Variant::String(format!("{}|0|UUID", sub_addr.ip())),
            Variant::Signed(1),
        ]);
        let pkt = Message::Game(GameUpdatePacket::call_function(-1, redirect.serialize())).encode();
        main_srv.peer_mut(srv_peer).send(0, &enet::Packet::reliable(pkt)).unwrap();
        let seen = wait(&mut [&mut client, &mut main_srv], |i, e| match (i, e) {
            (0, enet::EventNoRef::Receive { packet, .. }) => match Message::decode(packet.data()) {
                Some(Message::Game(p)) => {
                    let vl = VariantList::parse(&p.extra).unwrap();
                    got_console |= vl.function() == Some("OnDialogRequest");
                    (vl.function() == Some("OnSendToServer")).then_some(vl)
                }
                _ => None,
            },
            _ => None,
        });
        assert!(got_console, "/proxy should open a dialog");
        assert_eq!(seen.get(1), Some(&Variant::Signed(PROXY_PORT as i32)));
        assert_eq!(seen.get(4).and_then(Variant::as_str), Some("127.0.0.1|0|UUID"));

        // 4. client reconnects to the proxy -> proxy now goes to the sub-server.
        client.peer_mut(cli_peer).disconnect_now(0);
        client.flush();
        client.connect(proxy_addr, 2, 0).unwrap();
        wait(&mut [&mut client, &mut main_srv, &mut sub_srv], |i, e| match (i, e) {
            (2, enet::EventNoRef::Connect { .. }) => Some(()),
            _ => None,
        });

        shared.request_stop();
        bridge.join().unwrap();
    }
}
