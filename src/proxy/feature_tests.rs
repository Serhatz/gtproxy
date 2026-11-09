//! Packet-level tests for the social, chat, world-list and settings features.

use std::sync::Arc;

use crate::config::Config;
use crate::protocol::message::encode_text;
use crate::protocol::{GameUpdatePacket, Message, MessageType, TextPacket, Variant, VariantList};

use super::Direction;
use super::effect::Effect;
use super::interceptor::{Interceptor, Verdict};
use super::settings;

fn ic() -> Interceptor {
    let mut cfg = Config::default();
    cfg.log.text = false;
    cfg.log.variant = false;
    Interceptor::new(Arc::new(cfg))
}

fn input(ic: &mut Interceptor, text: &str) -> Verdict {
    ic.handle(Direction::ToServer, &encode_text(MessageType::GenericText, &format!("action|input\n|text|{text}")))
}

fn call(ic: &mut Interceptor, net_id: i32, items: Vec<Variant>) -> Verdict {
    let pkt = GameUpdatePacket::call_function(net_id, VariantList::new(items).serialize());
    ic.handle(Direction::ToClient, &Message::Game(pkt).encode())
}

fn spawn(ic: &mut Interceptor, net_id: i32, name: &str, local: bool) -> Verdict {
    let kind = if local { "type|local\n" } else { "" };
    call(
        ic,
        -1,
        vec![
            Variant::String("OnSpawn".into()),
            Variant::String(format!("spawn|avatar\nnetID|{net_id}\nname|`w{name}``\ncountry|us\n{kind}")),
        ],
    )
}

/// Chat lines sent to the server, delayed ones included.
fn chats(effects: Vec<Effect>) -> Vec<String> {
    effects
        .into_iter()
        .filter_map(|e| {
            let e = match e {
                Effect::Later(_, inner) => *inner,
                e => e,
            };
            match e {
                Effect::ToServer(b) => {
                    let t = String::from_utf8_lossy(&b[4..b.len() - 1]).into_owned();
                    t.strip_prefix("action|input\n|text|").map(|c| c.trim_end().to_string())
                }
                _ => None,
            }
        })
        .collect()
}

fn replaced_text(v: Verdict) -> String {
    let Verdict::Replace(b) = v else { panic!("not replaced: {v:?}") };
    match Message::decode(&b).unwrap() {
        Message::Text { text, .. } => text,
        Message::Game(p) => VariantList::parse(&p.extra).unwrap().to_string(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn auto_pull_and_ban_on_join() {
    let mut ic = ic();
    spawn(&mut ic, 1, "Me", true);
    input(&mut ic, "/addpull Bob");
    input(&mut ic, "/addban Eve");
    ic.take_effects();
    spawn(&mut ic, 5, "Bob", false);
    spawn(&mut ic, 6, "Eve", false);
    spawn(&mut ic, 7, "Ann", false);
    assert_eq!(chats(ic.take_effects()), vec!["/pull Bob", "/ban Eve"]);

    // /lp repeats the last pull.
    input(&mut ic, "/lp");
    assert_eq!(chats(ic.take_effects()), vec!["/pull Bob"]);
}

#[test]
fn ban_words_ban_the_speaker() {
    let mut ic = ic();
    spawn(&mut ic, 1, "Me", true);
    spawn(&mut ic, 5, "Bob", false);
    ic.features.ban_words.enabled = true;
    ic.features.ban_words.entries.insert("scam".into());
    ic.take_effects();
    let bubble = |net: u32, text: &str| {
        vec![
            Variant::String("OnTalkBubble".into()),
            Variant::Unsigned(net),
            Variant::String(format!("CP:0_PL:0_OID:_player_chat={text}")),
        ]
    };
    call(&mut ic, -1, bubble(5, "free SCAM here"));
    assert_eq!(chats(ic.take_effects()), vec!["/ban Bob"]);
    // Your own words never ban anyone.
    call(&mut ic, -1, bubble(1, "scam"));
    assert!(chats(ic.take_effects()).is_empty());
}

#[test]
fn hidden_names_and_clothes() {
    let mut ic = ic();
    spawn(&mut ic, 1, "Me", true);
    ic.features.hidden_names.insert("bob".into());
    let out = replaced_text(spawn(&mut ic, 5, "Bob", false));
    assert!(out.contains("name|``\\n"), "{out}");
    assert_eq!(ic.session.players.get(&5).map(String::as_str), Some("Bob"));

    ic.features.hide_clothes = true;
    let clothing = || {
        vec![
            Variant::String("OnSetClothing".into()),
            Variant::Vec3(48.0, 1.0, 2.0),
            Variant::Vec3(3.0, 4.0, 5.0),
            Variant::Vec3(6.0, 7.0, 8.0),
            Variant::Unsigned(1234),
            Variant::Vec3(9.0, 0.0, 0.0),
        ]
    };
    let out = replaced_text(call(&mut ic, 5, clothing()));
    assert!(out.contains("[1] (0, 0, 0)") && out.contains("[4] 1234u"), "{out}");
    // Your own clothes stay.
    assert_eq!(call(&mut ic, 1, clothing()), Verdict::Forward);
}

#[test]
fn me_and_formalize_rewrite_chat() {
    let mut ic = ic();
    input(&mut ic, "/me");
    input(&mut ic, "/formalize");
    ic.take_effects();
    let out = replaced_text(input(&mut ic, "hello world"));
    assert_eq!(TextPacket::parse(&out).get("text"), Some("/me Hello world."));
    // Game commands pass untouched.
    assert_eq!(input(&mut ic, "/msg bob hi"), Verdict::Forward);
}

#[test]
fn saved_worlds_warp_with_door() {
    let mut ic = ic();
    ic.session.enter_world("START".into());
    input(&mut ic, "/sworld :abc trade spot");
    assert_eq!(ic.features.saved_worlds[0].target(), "START|ABC");
    assert_eq!(ic.features.saved_worlds[0].description, "trade spot");
    ic.session.leave_world();
    ic.take_effects();
    input(&mut ic, "/list 1");
    let sent: Vec<String> = ic
        .take_effects()
        .into_iter()
        .filter_map(|e| match e {
            Effect::ToServer(b) => Some(String::from_utf8_lossy(&b[4..]).into_owned()),
            _ => None,
        })
        .collect();
    assert!(sent[0].starts_with("action|join_request\nname|START|ABC\n"), "{sent:?}");
}

#[test]
fn settings_file_roundtrip() {
    let dir = std::env::temp_dir().join(format!("gtproxy-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("settings.json");

    let mut a = ic();
    a.store = settings::Store::new(path.clone());
    a.fast_trash = true;
    a.features.auto_ban.entries.insert("eve".into());
    settings::save(&mut a).unwrap();

    let mut b = ic();
    b.store = settings::Store::new(path.clone());
    settings::load(&mut b);
    assert!(b.fast_trash);
    assert!(b.features.auto_ban.entries.contains("eve"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn spin_stats_count_bubbles_once() {
    let mut ic = ic();
    let spin = "`7[```wBob`` spun the wheel and got `423``!`7]``";
    call(&mut ic, -1, vec![Variant::String("OnTalkBubble".into()), Variant::Unsigned(2), Variant::String(spin.into())]);
    call(&mut ic, -1, vec![Variant::String("OnConsoleMessage".into()), Variant::String(spin.into())]);
    assert_eq!((ic.roulette_stats.total, ic.roulette_stats.red), (1, 1));
}

#[test]
fn messages_start_with_a_capital() {
    assert_eq!(super::commands::sentence("dropped 5"), "Dropped 5");
    assert_eq!(super::commands::sentence("`wbob`` added"), "`wbob`` added");
    assert_eq!(super::commands::sentence("/me mode on"), "/me mode on");
}

#[test]
fn lock_drop_answers_from_inventory_at_once() {
    let mut ic = ic();
    ic.inventory.slots = 20; // inventory received
    ic.inventory.add(super::locks::WL, 3);
    input(&mut ic, "/dd 5");
    input(&mut ic, "/dw 10");
    assert!(ic.planned_drops.is_empty());
    let overlays = ic
        .take_effects()
        .into_iter()
        .filter_map(|e| match e {
            Effect::ToClient(b) => match Message::decode(&b)? {
                Message::Game(p) => VariantList::parse(&p.extra)?.get(1)?.as_str().map(String::from),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(overlays, vec!["`4You have no Diamond Lock``", "`4You only have 3 World Lock``"]);

    input(&mut ic, "/dw 2");
    assert_eq!(ic.planned_drops.len(), 1);
    // /daw only asks for locks you have.
    ic.planned_drops.clear();
    input(&mut ic, "/daw");
    assert_eq!(ic.planned_drops.len(), 1);
}

#[test]
fn local_dirt_drop_keeps_real_pickups_working() {
    use crate::protocol::GamePacketType;
    let mut ic = ic();
    ic.fake_drops.reset(Some(9)); // next uid 10
    ic.session.pos = Some((64.0, 64.0));
    input(&mut ic, "/dlocal 5");
    let drops: Vec<GameUpdatePacket> = ic
        .take_effects()
        .into_iter()
        .filter_map(|e| match e {
            Effect::ToClient(b) => match Message::decode(&b)? {
                Message::Game(p) if p.packet_type == GamePacketType::ItemChangeObject => Some(p),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(drops.len(), 1);
    assert_eq!((drops[0].net_id, drops[0].value, drops[0].float_var), (-1, 2, 5.0));

    let pickup = |uid: u32| {
        Message::Game(GameUpdatePacket { packet_type: GamePacketType::ItemActivateObjectRequest, value: uid, ..Default::default() })
            .encode()
    };
    // Walking over the fake (client uid 10) never reaches the server.
    assert_eq!(ic.handle(Direction::ToServer, &pickup(10)), Verdict::Drop);

    // The server drops a real item: server uid 10, client uid 11.
    let real = GameUpdatePacket { packet_type: GamePacketType::ItemChangeObject, net_id: -1, value: 242, ..Default::default() };
    assert_eq!(ic.handle(Direction::ToClient, &Message::Game(real).encode()), Verdict::Forward);
    let Verdict::Replace(b) = ic.handle(Direction::ToServer, &pickup(11)) else { panic!("not translated") };
    let Some(Message::Game(p)) = Message::decode(&b) else { panic!() };
    assert_eq!(p.value, 10);

    // The server confirms the pickup of uid 10: the client knows it as 11.
    let collected = GameUpdatePacket { packet_type: GamePacketType::ItemChangeObject, net_id: 2, value: 10, ..Default::default() };
    let Verdict::Replace(b) = ic.handle(Direction::ToClient, &Message::Game(collected).encode()) else { panic!() };
    let Some(Message::Game(p)) = Message::decode(&b) else { panic!() };
    assert_eq!(p.value, 11);
}
