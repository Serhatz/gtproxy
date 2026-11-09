//! Desktop window: start/stop the proxy, launch Growtopia, see what is
//! happening and flip features. Every button runs an existing proxy command,
//! so the window and the in-game commands always do the same thing.

use std::time::Duration;

use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Sense, Stroke, Vec2};

use crate::config::Config;
use crate::engine::{self, Engine, StartError};
use crate::state::Snapshot;
use crate::{cert, hosts, ui_log};

// ── Palette ──────────────────────────────────────────────────────────────────
// Measured from the game's own screens: teal panels with a light rim, darker
// inset sections, chunky buttons (light blue, green, yellow, purple) whose
// darker bottom edge makes them look raised, white text with a drop shadow.
// The colours people can change live in `Theme` (Settings > Görünüm).

const PANEL_OUTER: Color32 = Color32::from_rgb(11, 54, 69);
const FIELD: Color32 = Color32::from_rgb(11, 74, 91);
const YELLOW: Color32 = Color32::from_rgb(253, 204, 4);
const PURPLE: Color32 = Color32::from_rgb(176, 8, 201);
const RED: Color32 = Color32::from_rgb(214, 62, 52);
const WARN: Color32 = YELLOW;
const DANGER: Color32 = Color32::from_rgb(255, 90, 80);
const SHADOW: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 120);
const BORDER: Color32 = Color32::from_rgb(70, 140, 168);
const CONTROL: Color32 = FIELD;

/// The customisable part of the look, from `config.toml` `[ui]`.
#[derive(Clone, Copy)]
struct Theme {
    bg: Color32,
    sidebar: Color32,
    panel: Color32,
    rim: Color32,
    inset: Color32,
    button: Color32,
    accent: Color32,
    text: Color32,
    muted: Color32,
    opacity: f32,
    font_scale: f32,
    radius: u8,
}

impl Theme {
    fn from_config(ui: &crate::config::UiConfig) -> Self {
        let c = |[r, g, b]: [u8; 3]| Color32::from_rgb(r, g, b);
        Self {
            bg: c(ui.background),
            sidebar: c(ui.sidebar),
            panel: c(ui.panel),
            rim: c(ui.rim),
            inset: c(ui.inset),
            button: c(ui.button),
            accent: c(ui.accent),
            text: c(ui.text),
            muted: c(ui.muted),
            opacity: ui.opacity.clamp(0.3, 1.0),
            font_scale: ui.font_scale.clamp(0.8, 1.4),
            radius: ui.corner_radius.min(16),
        }
    }
}

static THEME: std::sync::RwLock<Option<Theme>> = std::sync::RwLock::new(None);

/// The current theme (defaults until the window applies the config).
fn th() -> Theme {
    THEME
        .read()
        .ok()
        .and_then(|t| *t)
        .unwrap_or_else(|| Theme::from_config(&crate::config::UiConfig::default()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Features,
    World,
    Roulette,
    Log,
    Settings,
}

impl Page {
    const ALL: [Page; 6] = [Page::Overview, Page::Features, Page::World, Page::Roulette, Page::Log, Page::Settings];

    fn title(self) -> &'static str {
        match self {
            Page::Overview => "Genel bakış",
            Page::Features => "Özellikler",
            Page::World => "Dünya",
            Page::Roulette => "Rulet",
            Page::Log => "Log",
            Page::Settings => "Ayarlar",
        }
    }

    fn subtitle(self) -> &'static str {
        match self {
            Page::Overview => "Bağlantı, oyuncu ve envanter özeti",
            Page::Features => "Açıp kapatabileceğin proxy özellikleri",
            Page::World => "Dünyalar arası geçiş ve oyuncular",
            Page::Roulette => "Spin kontrolü ve istatistik",
            Page::Log => "Proxy'nin canlı kaydı",
            Page::Settings => "Bağlantı ve uygulama ayarları",
        }
    }

    /// Pages that talk to the running proxy.
    fn needs_proxy(self) -> bool {
        matches!(self, Page::Features | Page::World | Page::Roulette)
    }
}

pub struct ProxyApp {
    cfg: Config,
    engine: Option<Engine>,
    page: Page,
    /// Last message for the user: (text, is_error).
    message: Option<(String, bool)>,
    autostart: bool,
    warp: String,
    command: String,
    ping_alert: String,
    /// Teleport step while its slider is being dragged.
    tp_drag: Option<u64>,
    /// Collect range while its slider is being dragged.
    collect_drag: Option<u32>,
    /// Door ID box, item box (vend / floating item search) and "only this item" box.
    door: String,
    find_item: String,
    collect_only: String,
    log_filter: String,
    /// The game's backdrops, if its textures were found.
    backdrop: Option<crate::gui_background::Backdrop>,
    /// `[ui]` settings the current style was built from.
    applied_ui: crate::config::UiConfig,
    /// Embedded mode: the game window inside ours.
    embed: crate::embed::Embed,
    embed_status: crate::embed::Status,
    /// Embedded mode was on last frame (to maximise once / release once).
    embed_was_on: bool,
    /// Embedded mode: the page panel over the game is open (closed at start).
    side_open: bool,
    /// Window the panel was made an owned window of (the game, else ours).
    side_owner: Option<isize>,
}

/// Width of the page panel over the game, in points.
const SIDE_WIDTH: f32 = 400.0;
/// Title of the panel window, used to find it for `own_window`.
const SIDE_TITLE: &str = "GTProxy Panel";

pub fn run(cfg: Config, autostart: bool) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("GTProxy")
            .with_inner_size(preview_size().unwrap_or([980.0, 660.0]))
            .with_min_inner_size([780.0, 520.0]),
        ..Default::default()
    };
    let app = ProxyApp {
        cfg,
        engine: None,
        page: Page::Overview,
        message: None,
        autostart,
        warp: String::new(),
        command: String::new(),
        ping_alert: "300".into(),
        tp_drag: None,
        collect_drag: None,
        door: String::new(),
        find_item: String::new(),
        collect_only: String::new(),
        log_filter: String::new(),
        backdrop: None,
        applied_ui: Default::default(),
        embed: Default::default(),
        embed_status: crate::embed::Status::Off,
        embed_was_on: false,
        side_open: false,
        side_owner: None,
    };
    eframe::run_native(
        "GTProxy",
        options,
        Box::new(|cc| {
            let mut app = app;
            apply_theme(&cc.egui_ctx, &app.cfg);
            app.applied_ui = app.cfg.ui.clone();
            app.backdrop = engine::game_path(&app.cfg)
                .and_then(|exe| exe.parent().map(|d| d.to_path_buf()))
                .and_then(|dir| crate::gui_background::Backdrop::load(&cc.egui_ctx, &dir));
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("window error: {e}"))
}

/// The game's own font (Century Gothic Bold), loaded from its install folder;
/// Windows' copy of Century Gothic as a fallback.
fn game_font(cfg: &Config) -> Option<Vec<u8>> {
    let mut candidates = Vec::new();
    if let Some(dir) = engine::game_path(cfg).and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        candidates.push(dir.join(r"GameData\UI\Font\Century-Gothic-Bold.ttf"));
    }
    if let Some(windir) = std::env::var_os("WINDIR") {
        candidates.push(std::path::PathBuf::from(windir).join(r"Fonts\GOTHICB.TTF"));
    }
    candidates.into_iter().find_map(|p| std::fs::read(p).ok())
}

fn apply_theme(ctx: &egui::Context, cfg: &Config) {
    use egui::FontFamily;
    use egui::epaint::text::{FontData, FontInsert, FontPriority, InsertFontFamily};
    if let Some(bytes) = game_font(cfg) {
        ctx.add_font(FontInsert::new(
            "growtopia",
            FontData::from_owned(bytes),
            vec![InsertFontFamily { family: FontFamily::Proportional, priority: FontPriority::Highest }],
        ));
    }
    apply_style(ctx, &cfg.ui);
}

/// Colours, sizes and widget looks from the `[ui]` settings.
fn apply_style(ctx: &egui::Context, ui_cfg: &crate::config::UiConfig) {
    use egui::{FontFamily, FontId, TextStyle};
    if let Ok(mut t) = THEME.write() {
        *t = Some(Theme::from_config(ui_cfg));
    }
    let scale = th().font_scale;
    ctx.set_visuals(egui::Visuals::dark());
    ctx.global_style_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::new(22.0 * scale, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(14.0 * scale, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.0 * scale, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(12.0 * scale, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.5 * scale, FontFamily::Monospace)),
        ]
        .into();
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.interact_size.y = 30.0;

        let v = &mut style.visuals;
        v.panel_fill = th().bg;
        v.window_fill = th().panel;
        v.extreme_bg_color = FIELD;
        v.faint_bg_color = th().panel;
        v.override_text_color = Some(th().text);
        v.selection.bg_fill = YELLOW.gamma_multiply(0.45);
        v.selection.stroke = Stroke::new(1.0, YELLOW);
        let radius = CornerRadius::same(th().radius);
        for (w, fill) in [
            (&mut v.widgets.noninteractive, th().panel),
            (&mut v.widgets.inactive, CONTROL),
            (&mut v.widgets.hovered, th().inset),
            (&mut v.widgets.active, th().inset),
            (&mut v.widgets.open, th().inset),
        ] {
            w.corner_radius = radius;
            w.weak_bg_fill = fill;
            w.bg_fill = fill;
            w.expansion = 0.0;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, th().rim);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, th().text);
        v.widgets.active.bg_stroke = Stroke::new(1.5, YELLOW);
        v.widgets.inactive.fg_stroke = Stroke::new(1.0, th().text);
        v.widgets.hovered.fg_stroke = Stroke::new(1.0, th().text);
    });
}

impl ProxyApp {
    fn running(&self) -> bool {
        self.engine.is_some()
    }

    fn info(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), false));
    }

    fn error(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), true));
    }

    /// Runs a proxy command (without the `/`), as if typed in game.
    fn cmd(&self, line: impl Into<String>) {
        if let Some(e) = &self.engine {
            e.shared.command(line);
        }
    }

    fn start(&mut self) {
        match Engine::start(self.cfg.clone()) {
            Ok(e) => {
                self.engine = Some(e);
                self.info("Proxy çalışıyor.");
                if self.cfg.system.launch_game {
                    self.launch_game();
                }
            }
            Err(StartError::NeedAdmin) => match relaunch_as_admin() {
                // The elevated copy starts the proxy itself.
                Ok(()) => std::process::exit(0),
                Err(e) => self.error(format!("Yönetici izni gerekiyor: {e}")),
            },
            Err(StartError::Other(e)) => self.error(format!("Başlatılamadı: {e:#}")),
        }
    }

    fn stop(&mut self) {
        if let Some(e) = self.engine.take() {
            match e.stop() {
                Ok(()) => self.info("Proxy durduruldu, hosts dosyası eski haline döndü."),
                Err(e) => self.error(format!("Proxy hatayla durdu: {e:#}")),
            }
        }
    }

    fn launch_game(&mut self) {
        match engine::launch_game(&self.cfg) {
            Ok(()) => self.info("Proxy çalışıyor, Growtopia açılıyor."),
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    fn install_cert(&mut self) {
        match cert::install(&self.cfg.system.cert_dir) {
            Ok(()) => self.info("Sertifika kuruldu. Proxy'yi durdurup yeniden başlat."),
            Err(e) => self.error(format!("{e:#}")),
        }
    }

    // ── Frame parts ──────────────────────────────────────────────────────────

    fn sidebar(&mut self, ui: &mut egui::Ui, snap: &Snapshot) {
        ui.add_space(4.0);
        shadowed(ui, "GTProxy", 22.0, th().text);
        ui.label(RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).small().color(th().muted));
        ui.add_space(10.0);
        let (text, colour) = connection_state(self.running(), snap);
        status_pill(ui, text, colour);
        ui.add_space(18.0);

        for page in Page::ALL {
            if nav_item(ui, page.title(), self.page == page).clicked() {
                self.page = page;
            }
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.add_space(4.0);
            let width = ui.available_width();
            if self.running() {
                if secondary_button(ui, "Durdur", width, DANGER).clicked() {
                    self.stop();
                }
                if secondary_button(ui, "Growtopia'yı aç", width, th().text).clicked() {
                    self.launch_game();
                }
            } else if primary_button(ui, "Başlat", width).clicked() {
                self.start();
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.engine.as_ref().is_some_and(|e| e.shared.tls_failed()) {
                ui.label(RichText::new("Oyun sertifikayı kabul etmedi.").color(DANGER));
                if gt_small(ui, "Sertifikayı kur", YELLOW).clicked() {
                    self.install_cert();
                }
                return;
            }
            match &self.message {
                Some((text, true)) => ui.label(RichText::new(text).color(DANGER)),
                Some((text, false)) => ui.label(RichText::new(text).color(th().muted)),
                None => ui.label(RichText::new("Hazır").color(th().muted)),
            };
        });
    }

    fn page_header(&self, ui: &mut egui::Ui) {
        shadowed(ui, self.page.title(), 26.0, th().text);
        shadowed(ui, self.page.subtitle(), 14.0, th().text);
        ui.add_space(12.0);
    }

    fn proxy_off(&mut self, ui: &mut egui::Ui) {
        card(ui, None, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("Proxy kapalı").size(17.0).strong());
                ui.label(RichText::new("Bu bölüm proxy çalışırken kullanılabilir.").color(th().muted));
                ui.add_space(10.0);
                if primary_button(ui, "Başlat", 160.0).clicked() {
                    self.start();
                }
                ui.add_space(20.0);
            });
        });
    }

    // ── Pages ────────────────────────────────────────────────────────────────

    fn overview(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        let (state, colour) = connection_state(self.running(), s);
        let (wl, dl, bgl) = s.locks;
        let ping = match (s.client_ping_ms, s.server_ping_ms) {
            (_, Some(server)) => format!("{server} ms"),
            (Some(client), None) => format!("{client} ms"),
            _ => "-".into(),
        };
        let locks = format!("{} WL", wl + dl * 100 + bgl * 10_000);
        let stats = [
            ("Bağlantı", state, colour),
            ("Ping", ping.as_str(), th().text),
            ("Dünya", s.world.as_deref().unwrap_or("-"), th().text),
            ("Kilitler", locks.as_str(), th().text),
        ];
        flow(ui, 4, 170.0, |ui, i| stat(ui, stats[i].0, stats[i].1, stats[i].2));
        flow(ui, 2, 300.0, |ui, i| match i {
            0 => card(ui, Some("Oyuncu"), |ui| {
                let dash = |v: Option<String>| v.unwrap_or_else(|| "-".into());
                info_row(ui, "İsim", &dash(s.name.clone()));
                info_row(ui, "Seviye", &dash(s.level.map(|v| v.to_string())));
                info_row(ui, "Gem", &dash(s.gems.map(|v| v.to_string())));
                info_row(ui, "Sunucu", &dash(s.server.clone()));
            }),
            _ => card(ui, Some("Envanter"), |ui| {
                info_row(ui, "Blue Gem Lock", &bgl.to_string());
                info_row(ui, "Diamond Lock", &dl.to_string());
                info_row(ui, "World Lock", &wl.to_string());
                info_row(ui, "Eşya çeşidi", &s.inventory_items.to_string());
            }),
        });
        card(ui, Some("Komut"), |ui| {
            ui.label(RichText::new("Oyun içi komutların hepsi burada da çalışır.").small().color(th().muted));
            ui.horizontal(|ui| {
                let running = self.running();
                let (run, enter) = ui
                    .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let run = ui.add_enabled_ui(running, |ui| gt_button(ui, "Çalıştır", th().accent)).inner;
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.command)
                                .hint_text("dw 5, tp 10 20, vendfind dirt ...")
                                .desired_width(ui.available_width()),
                        );
                        (run, edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    })
                    .inner;
                if (run.clicked() || enter) && !self.command.trim().is_empty() {
                    self.cmd(self.command.trim().trim_start_matches('/').to_string());
                    self.command.clear();
                }
            });
        });
    }

    fn features(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        flow(ui, 2, 340.0, |ui, i| if i == 0 { self.features_left(ui, s) } else { self.features_right(ui, s) });
    }

    fn features_left(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        {
            card(ui, Some("Eşyalar"), |ui| {
                if setting(ui, "Fast drop", "Drop'ta adet sorulmaz, hepsi bırakılır", s.fast_drop) {
                    self.cmd("fastdrop");
                }
                divider(ui);
                if setting(ui, "Fast trash", "Trash ve recycle'da adet sorulmaz", s.fast_trash) {
                    self.cmd("fasttrash");
                }
                divider(ui);
                if setting(ui, "DL -> BGL", "Telephone'a wrench atınca 100 DL çevrilir", s.auto_change) {
                    self.cmd("change");
                }
                divider(ui);
                labeled_row(ui, "Hepsini bırak", "Oyunda önce onay penceresi açılır", |ui| {
                    if gt_small(ui, "Her eşyadan 1", th().button).clicked() {
                        self.cmd("dropall1");
                    }
                    if gt_small(ui, "Hepsi", RED).clicked() {
                        self.cmd("dropall");
                    }
                });
            });
            card(ui, Some("Toplama"), |ui| {
                let what = s.collect_only.as_deref().map_or("Her şeyi".to_string(), |n| format!("Sadece {n}"));
                if setting(ui, "Oto toplama", &format!("{what}, {} kare içinden toplar", s.collect_range), s.auto_collect) {
                    self.cmd("ac");
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Menzil").small().color(th().muted));
                    let mut v = self.collect_drag.unwrap_or(s.collect_range);
                    ui.spacing_mut().slider_width = (ui.available_width() - 80.0).clamp(60.0, 220.0);
                    let r = ui.add(egui::Slider::new(&mut v, 1..=10).suffix(" kare"));
                    if r.dragged() {
                        self.collect_drag = Some(v);
                    }
                    if r.drag_stopped() || (r.changed() && !r.dragged()) {
                        self.cmd(format!("collect range {v}"));
                        self.collect_drag = None;
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.collect_only)
                        .hint_text("Sadece bu eşya (boş = hepsi)")
                        .desired_width(ui.available_width()),
                );
                ui.horizontal_wrapped(|ui| {
                    if gt_small(ui, "Sadece bunu", th().button).clicked() && !self.collect_only.trim().is_empty() {
                        self.cmd(format!("collect only {}", self.collect_only.trim()));
                    }
                    if gt_small(ui, "Hepsi", th().button).clicked() {
                        self.collect_only.clear();
                        self.cmd("collect only all");
                    }
                    if gt_small(ui, "Şimdi topla", th().accent).clicked() {
                        self.cmd("collect now");
                    }
                    if gt_small(ui, "Oyunda ayarla", th().button).clicked() {
                        self.cmd("collect");
                    }
                });
            });
            card(ui, Some("Sohbet"), |ui| {
                if setting(ui, "/me modu", "Her mesaj /me ile gider", s.me) {
                    self.cmd("me");
                }
                divider(ui);
                if setting(ui, "Formalize", "Büyük harfle başlar, noktayla biter", s.formalize) {
                    self.cmd("formalize");
                }
                divider(ui);
                labeled_row(ui, "Spam", if s.spam_running { "Çalışıyor" } else { "Kapalı" }, |ui| {
                    if s.spam_running && gt_button(ui, "Durdur", RED).clicked() {
                        self.cmd("spam stop");
                    }
                    if gt_button(ui, "Oyunda ayarla", th().button).clicked() {
                        self.cmd("spam");
                    }
                });
            });
        }
    }

    fn features_right(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        {
            card(ui, Some("Hareket ve görüntü"), |ui| {
                if setting(ui, "Işınlanma", "Ctrl basılıyken tıkladığın yere gidersin", s.teleport) {
                    self.cmd("tp");
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Adım hızı").small().color(th().muted));
                    let mut v = self.tp_drag.unwrap_or(s.teleport_step_ms);
                    ui.spacing_mut().slider_width = (ui.available_width() - 80.0).clamp(60.0, 220.0);
                    let r = ui.add(egui::Slider::new(&mut v, 10..=500).suffix(" ms"));
                    if r.dragged() {
                        self.tp_drag = Some(v);
                    }
                    if r.drag_stopped() || (r.changed() && !r.dragged()) {
                        self.cmd(format!("tp speed {v}"));
                        self.tp_drag = None;
                    }
                });
                divider(ui);
                if setting(ui, "Sınırsız zoom", "Oyunun kendi zoom'u sınırsız olur (sonraki dünyada)", s.wheel_zoom) {
                    self.cmd("zoom");
                }
                ui.horizontal_wrapped(|ui| {
                    for (label, level) in [("En uzak", "0.1"), ("Uzak", "0.5"), ("Normal", "1")] {
                        if gt_small(ui, label, th().button).clicked() {
                            self.cmd(format!("zoom {level}"));
                        }
                    }
                });
                divider(ui);
                if setting(ui, "Kıyafetleri gizle", "Kalabalık dünyalarda lag azalır", s.hide_clothes) {
                    self.cmd("hideclothes");
                }
            });
            card(ui, Some("Diğer"), |ui| {
                let desc = s.ping_alert_ms.map_or("Kapalı".to_string(), |v| format!("{v} ms üstünde uyarır"));
                labeled_row(ui, "Ping uyarısı", &desc, |ui| {
                    if gt_small(ui, "Kapat", th().button).clicked() {
                        self.cmd("pingalert off");
                    }
                    if gt_small(ui, "Ayarla", th().accent).clicked() {
                        self.cmd(format!("pingalert {}", self.ping_alert.trim()));
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.ping_alert).desired_width(52.0));
                });
                divider(ui);
                if setting(ui, "Oto kaydet", "Ayarlar kapatıp açınca korunur", s.autosave) {
                    self.cmd("autosave");
                }
                divider(ui);
                if setting(ui, "Sade mesajlar", "Proxy sohbete yazmaz, uyarılar ortada kalır", s.minimal) {
                    self.cmd("minimal");
                }
                divider(ui);
                if setting(ui, "Level yazısını gizle", "\"You need to be Level\" yazıları görünmez", s.hide_level) {
                    self.cmd("hidelevel");
                }
            });
        }
    }

    fn world(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        card(ui, Some("Dünya değiştir"), |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 70.0).clamp(80.0, 260.0);
                let edit = ui.add(egui::TextEdit::singleline(&mut self.warp).hint_text("Dünya adı").desired_width(width));
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (gt_button(ui, "Git", th().accent).clicked() || enter) && !self.warp.trim().is_empty() {
                    self.cmd(format!("warp {}", self.warp.trim()));
                }
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (label, c) in
                    [("Geri", "back"), ("Relog", "relog"), ("Çıkış", "exit"), ("Rastgele", "rndm"), ("Bu dünyayı kaydet", "sworld"), ("Log off", "logoff")]
                {
                    if gt_button(ui, label, th().button).clicked() {
                        self.cmd(c);
                    }
                }
            });
            ui.add_space(4.0);
            field_row(ui, "Kapı", |ui| {
                let width = (ui.available_width() - 60.0).clamp(60.0, 200.0);
                let edit = ui.add(egui::TextEdit::singleline(&mut self.door).hint_text("Kapı ID").desired_width(width));
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (gt_small(ui, "Git", th().accent).clicked() || enter) && !self.door.trim().is_empty() {
                    self.cmd(format!("door {}", self.door.trim()));
                }
            });
        });
        card(ui, Some("Bu dünya"), |ui| {
            ui.horizontal_wrapped(|ui| {
                for (label, c) in [
                    ("Growscan", "gs"),
                    ("Yerdeki eşyalar", "items"),
                    ("Kilitler", "admin"),
                    ("Kapılar", "locked"),
                    ("Vend fiyatlarını kopyala", "vendcopy"),
                    ("Drop geçmişi", "drophistory"),
                ] {
                    if gt_small(ui, label, th().button).clicked() {
                        self.cmd(c);
                    }
                }
            });
            ui.add_space(4.0);
            ui.add(egui::TextEdit::singleline(&mut self.find_item).hint_text("Eşya adı").desired_width(ui.available_width().min(360.0)));
            let item = self.find_item.trim().to_string();
            ui.horizontal_wrapped(|ui| {
                if gt_small(ui, "Vend bul", th().accent).clicked() {
                    self.cmd(format!("vendfind {item}"));
                }
                if gt_small(ui, "En ucuz vend'e git", th().button).clicked() && !item.is_empty() {
                    self.cmd(format!("pvend {item}"));
                }
                if gt_small(ui, "Yerdekine git", th().button).clicked() && !item.is_empty() {
                    self.cmd(format!("pfind {item}"));
                }
            });
            ui.label(RichText::new(format!("Kayıtlı vend: {}", s.vends_known)).small().color(th().muted));
            divider(ui);
            if setting(ui, "Canlı drop takibi", "Başkalarının attığı ve aldığı eşyalar sohbete yazılır", s.track_drops) {
                self.cmd("track");
            }
        });
        flow(ui, 2, 340.0, |ui, i| if i == 0 { self.world_players(ui, s) } else { self.world_lists(ui, s) });
    }

    fn world_players(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        {
            card(ui, Some(&format!("Oyuncular · {}", s.players.len())), |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Wrench").small().color(th().muted));
                    for (label, c) in [("Kapalı", "off"), ("Pull", "pull"), ("Kick", "kick"), ("Ban", "ban")] {
                        if chip(ui, label, s.wrench.as_deref().unwrap_or("off") == c).clicked() {
                            self.cmd(format!("wrench {c}"));
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if gt_small(ui, "Herkesi pull", th().accent).clicked() {
                        self.cmd("pullall");
                    }
                    if gt_small(ui, "Herkesi kick", PURPLE).clicked() {
                        self.cmd("kickall");
                    }
                    if gt_small(ui, "Herkesi ban", RED).clicked() {
                        self.cmd("banall");
                    }
                });
                divider(ui);
                if s.players.is_empty() {
                    ui.label(RichText::new("Dünyada başka kimse yok.").color(th().muted));
                }
                egui::ScrollArea::vertical().id_salt("players").max_height(260.0).show(ui, |ui| {
                    for (_, name) in &s.players {
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if gt_small(ui, "Gizle", th().button).clicked() {
                                    self.cmd(format!("hidename {name}"));
                                }
                                if gt_small(ui, "Git", th().button).clicked() {
                                    self.cmd(format!("tpto {name}"));
                                }
                                if gt_small(ui, "Ban", RED).clicked() {
                                    self.cmd(format!("chat /ban {name}"));
                                }
                                if gt_small(ui, "Kick", PURPLE).clicked() {
                                    self.cmd(format!("chat /kick {name}"));
                                }
                                if gt_small(ui, "Pull", th().accent).clicked() {
                                    self.cmd(format!("chat /pull {name}"));
                                }
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.add(egui::Label::new(RichText::new(name).strong()).truncate());
                                });
                            });
                        });
                    }
                });
            });
        }
    }

    fn world_lists(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        {
            card(ui, Some("Kayıtlı dünyalar"), |ui| {
                if s.saved_worlds.is_empty() {
                    ui.label(RichText::new("Henüz yok. Bir dünyadayken \"Bu dünyayı kaydet\"e bas.").color(th().muted));
                }
                for (i, (target, note)) in s.saved_worlds.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if gt_button(ui, target, th().button).clicked() {
                            self.cmd(format!("list {}", i + 1));
                        }
                        ui.add(egui::Label::new(RichText::new(note).small().color(th().muted)).truncate());
                    });
                }
            });
            card(ui, Some("Listeler"), |ui| {
                ui.label(RichText::new("Düzenleme penceresi oyunun içinde açılır.").small().color(th().muted));
                for (label, on, c) in [
                    ("Oto pull", s.auto_pull, "autopullpage"),
                    ("Oto ban", s.auto_ban, "autobanpage"),
                    ("Yasaklı kelimeler", s.ban_words, "banwords"),
                ] {
                    ui.horizontal(|ui| {
                        status_dot(ui, if on { th().accent } else { th().muted });
                        ui.label(label);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if gt_small(ui, "Düzenle", th().button).clicked() {
                                self.cmd(c);
                            }
                        });
                    });
                }
            });
        }
    }

    fn roulette(&mut self, ui: &mut egui::Ui, s: &Snapshot) {
        card(ui, Some("Kontrol"), |ui| {
            if setting(ui, "Rulet kontrolü", "Gerçek spinlerin başına [REAL] yazılır", s.roulette_enabled) {
                self.cmd("spin toggle");
            }
            divider(ui);
            if setting(ui, "Anında göster", "Sonuç çark dururken değil, hemen görünür", s.roulette_instant) {
                self.cmd("spin instant");
            }
            divider(ui);
            labeled_row(ui, "Oyun", "QQ ve Reme değeri spinin sonuna eklenir", |ui| {
                for mode in ["Reme", "QQ", "CSN"] {
                    if chip(ui, mode, s.roulette_mode == mode).clicked() {
                        self.cmd(format!("spin {}", mode.to_lowercase()));
                    }
                }
            });
        });
        let sp = &s.spins;
        let pct = |n: u32| if sp.total == 0 { "-".into() } else { format!("{:.0}%", n as f64 * 100.0 / sp.total as f64) };
        let stats = [
            ("Toplam spin", sp.total.to_string(), th().text),
            ("Kırmızı", format!("{} · {}", sp.red, pct(sp.red)), DANGER),
            ("Siyah", format!("{} · {}", sp.black, pct(sp.black)), th().text),
            ("Yeşil", format!("{} · {}", sp.green, pct(sp.green)), th().accent),
        ];
        flow(ui, 4, 150.0, |ui, i| stat(ui, stats[i].0, &stats[i].1, stats[i].2));
        card(ui, Some("Son spinler"), |ui| {
            if sp.recent.is_empty() {
                ui.label(RichText::new("Henüz spin yok.").color(th().muted));
            }
            ui.horizontal_wrapped(|ui| {
                for (n, c) in &sp.recent {
                    spin_ball(ui, *n, *c);
                }
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if gt_small(ui, "Sıfırla", RED).clicked() {
                    self.cmd("spinstats reset");
                }
                if gt_small(ui, "Oyunda geçmiş", th().button).clicked() {
                    self.cmd("roulette");
                }
            });
        });
        card(ui, Some("Kim attı"), |ui| {
            if s.spin_history.is_empty() {
                ui.label(RichText::new("Henüz spin yok.").color(th().muted));
            }
            for (player, n, c) in &s.spin_history {
                ui.horizontal(|ui| {
                    spin_ball(ui, *n, *c);
                    ui.add(egui::Label::new(RichText::new(player).strong()).truncate());
                });
            }
        });
    }

    fn log(&mut self, ui: &mut egui::Ui) {
        card(ui, None, |ui| {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 110.0).clamp(80.0, 320.0);
                ui.add(egui::TextEdit::singleline(&mut self.log_filter).hint_text("Filtrele").desired_width(width));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if gt_button(ui, "Temizle", th().button).clicked() {
                        ui_log::clear();
                    }
                });
            });
            divider(ui);
            let filter = self.log_filter.to_lowercase();
            let lines: Vec<String> =
                ui_log::lines().into_iter().filter(|l| filter.is_empty() || l.to_lowercase().contains(&filter)).collect();
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink(false).show(ui, |ui| {
                for line in lines {
                    let colour = if line.contains("ERROR") {
                        DANGER
                    } else if line.contains("WARN") {
                        WARN
                    } else {
                        th().muted
                    };
                    ui.label(RichText::new(line).monospace().color(colour));
                }
            });
        });
    }

    /// Colours, sizes and backdrop; applied live, kept with "Kaydet".
    fn appearance(&mut self, ui: &mut egui::Ui) {
        card(ui, Some("Görünüm"), |ui| {
            labeled_row(ui, "Arka plan", "Oyunun açılış ekranı, bağlantı ekranı ya da düz renk", |ui| {
                for (label, key) in [("Düz", "plain"), ("Bağlantı", "connecting"), ("Açılış", "title")] {
                    if chip(ui, label, self.cfg.ui.backdrop == key).clicked() {
                        self.cfg.ui.backdrop = key.into();
                    }
                }
            });
            divider(ui);
            let ui_cfg = &mut self.cfg.ui;
            let colours: [(&str, &mut [u8; 3]); 9] = [
                ("Kart", &mut ui_cfg.panel),
                ("Kenar", &mut ui_cfg.rim),
                ("İç bölüm", &mut ui_cfg.inset),
                ("Menü", &mut ui_cfg.sidebar),
                ("Buton", &mut ui_cfg.button),
                ("Vurgu", &mut ui_cfg.accent),
                ("Yazı", &mut ui_cfg.text),
                ("Açıklama", &mut ui_cfg.muted),
                ("Düz arka plan", &mut ui_cfg.background),
            ];
            // Swatch + label pairs that wrap onto as many rows as needed.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 10.0);
                for (label, value) in colours {
                    ui.allocate_ui_with_layout(Vec2::new(150.0, 26.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(150.0);
                        egui::color_picker::color_edit_button_srgb(ui, value);
                        ui.label(RichText::new(label).color(th().muted));
                    });
                }
            });
            divider(ui);
            let ui_cfg = &mut self.cfg.ui;
            let slider_width = |ui: &mut egui::Ui| ui.spacing_mut().slider_width = (ui.available_width() - 70.0).clamp(60.0, 260.0);
            field_row(ui, "Saydamlık", |ui| {
                slider_width(ui);
                let mut solid = (ui_cfg.opacity * 100.0).round();
                if ui.add(egui::Slider::new(&mut solid, 30.0..=100.0).suffix(" %")).changed() {
                    ui_cfg.opacity = solid / 100.0;
                }
            });
            field_row(ui, "Yazı boyutu", |ui| {
                slider_width(ui);
                let mut size = (ui_cfg.font_scale * 100.0).round();
                if ui.add(egui::Slider::new(&mut size, 80.0..=140.0).suffix(" %")).changed() {
                    ui_cfg.font_scale = size / 100.0;
                }
            });
            field_row(ui, "Köşe", |ui| {
                slider_width(ui);
                ui.add(egui::Slider::new(&mut ui_cfg.corner_radius, 0..=16));
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if gt_button(ui, "Varsayılana dön", th().button).clicked() {
                    self.cfg.ui = crate::config::UiConfig::default();
                }
                if gt_button(ui, "Görünümü kaydet", th().accent).clicked() {
                    match self.cfg.save() {
                        Ok(()) => self.info("Görünüm kaydedildi."),
                        Err(e) => self.error(format!("{e:#}")),
                    }
                }
            });
        });
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        self.appearance(ui);
        flow(ui, 2, 340.0, |ui, i| if i == 0 { self.settings_left(ui) } else { self.settings_right(ui) });
        ui.horizontal_wrapped(|ui| {
            if gt_button_sized(ui, "Kaydet", th().accent, Vec2::new(120.0, 38.0), 16.0).clicked() {
                match self.cfg.save() {
                    Ok(()) => self.info("Ayarlar kaydedildi; proxy bir sonraki başlatmada kullanır."),
                    Err(e) => self.error(format!("{e:#}")),
                }
            }
            ui.label(RichText::new("Bir sonraki başlatmada geçerli olur").small().color(th().muted));
        });
    }

    fn settings_left(&mut self, ui: &mut egui::Ui) {
        {
            card(ui, Some("Oyun"), |ui| {
                ui.label(RichText::new("Growtopia.exe yolu").small().color(th().muted));
                ui.add(
                    egui::TextEdit::singleline(&mut self.cfg.system.game_path)
                        .hint_text("Boş = varsayılan konum")
                        .desired_width(f32::INFINITY),
                );
                divider(ui);
                if setting(ui, "Oyunu otomatik aç", "Başlat'a basınca Growtopia da açılır", self.cfg.system.launch_game) {
                    self.cfg.system.launch_game = !self.cfg.system.launch_game;
                }
                divider(ui);
                if setting(ui, "Gömmeli mod", "Growtopia bu pencerede açılır, proxy üstte, sayfalar açılır panelde", self.cfg.ui.embed_game) {
                    self.cfg.ui.embed_game = !self.cfg.ui.embed_game;
                }
                divider(ui);
                if setting(ui, "Terminal modu", "Kaydedince sonraki açılışta pencere yerine terminal açılır", self.cfg.system.terminal) {
                    self.cfg.system.terminal = !self.cfg.system.terminal;
                }
            });
            card(ui, Some("Bakım"), |ui| {
                ui.horizontal_wrapped(|ui| {
                    if gt_button(ui, "Sertifikayı kur", th().accent).clicked() {
                        self.install_cert();
                    }
                    if gt_button(ui, "Sertifikayı kaldır", th().button).clicked() {
                        match cert::uninstall() {
                            Ok(()) => self.info("Sertifika kaldırıldı."),
                            Err(e) => self.error(format!("{e:#}")),
                        }
                    }
                    if ui.add_enabled_ui(!self.running(), |ui| gt_button(ui, "hosts'u temizle", th().button)).inner.clicked() {
                        match hosts::uninstall() {
                            Ok(true) => self.info("hosts dosyası temizlendi."),
                            Ok(false) => self.info("hosts dosyasında proxy satırı yok."),
                            Err(e) => self.error(format!("{e:#} (yönetici olarak açın)")),
                        }
                    }
                });
            });
        }
    }

    fn settings_right(&mut self, ui: &mut egui::Ui) {
        {
            card(ui, Some("Panel paylaşımı"), |ui| {
                let r = &mut self.cfg.report;
                ui.label(
                    RichText::new(
                        "Varsayılan açık; istersen kapatabilirsin. Vend fiyatları (dünyaya girince), spin sonuçları (oyuncu adı olmadan) ve proxy hataları web paneline gönderilir. GrowID, şifre, token, sohbet ve oyuncu isimleri asla gönderilmez.",
                    )
                    .small()
                    .color(th().muted),
                );
                divider(ui);
                if setting(ui, "Paylaş", "Kaydet, sonraki başlatmada geçerli", r.enabled) {
                    r.enabled = !r.enabled;
                }
                if r.enabled {
                    ui.label(RichText::new("Panel adresi (boş = projenin paneli)").small().color(th().muted));
                    ui.add(egui::TextEdit::singleline(&mut r.url).hint_text("https://...").desired_width(f32::INFINITY));
                    divider(ui);
                    if setting(ui, "Vendler", "Dünya, konum, eşya, fiyat", r.vends) {
                        r.vends = !r.vends;
                    }
                    if setting(ui, "Spinler", "Dünya, sayı, renk; isim yok", r.spins) {
                        r.spins = !r.spins;
                    }
                    if setting(ui, "Hatalar", "Proxy'nin uyarı ve hata satırları", r.errors) {
                        r.errors = !r.errors;
                    }
                }
            });
            card(ui, Some("Ağ"), |ui| {
                let text = |ui: &mut egui::Ui, value: &mut String| {
                    ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width()));
                };
                field_row(ui, "Yerel IP", |ui| text(ui, &mut self.cfg.proxy.local_ip));
                field_row(ui, "ENet portu", |ui| {
                    ui.add(egui::DragValue::new(&mut self.cfg.proxy.enet_port));
                });
                field_row(ui, "DNS sunucusu", |ui| text(ui, &mut self.cfg.upstream.dns_server));
                field_row(ui, "Log seviyesi", |ui| text(ui, &mut self.cfg.log.level));
                divider(ui);
                if setting(ui, "hosts dosyasını yönet", "Kapatırsan yönlendirmeyi elle yaparsın", self.cfg.system.manage_hosts) {
                    self.cfg.system.manage_hosts = !self.cfg.system.manage_hosts;
                }
                divider(ui);
                if setting(ui, "TLS doğrula", "Gerçek sunucunun sertifikası kontrol edilir", self.cfg.upstream.verify_tls) {
                    self.cfg.upstream.verify_tls = !self.cfg.upstream.verify_tls;
                }
            });
        }
    }
}

// ── Embedded mode ────────────────────────────────────────────────────────────

impl ProxyApp {
    /// Our window's handle, for parenting the game window.
    fn own_hwnd(frame: &eframe::Frame) -> Option<isize> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match frame.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
            _ => None,
        }
    }

    /// Top bar, page panel on the left, the game in the rest.
    fn embedded_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame, snap: &Snapshot, preview: bool) {
        let bar = th().sidebar.gamma_multiply(th().opacity.max(0.92));
        let mut refocus = false;
        egui::Panel::top("embed_top")
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(bar).inner_margin(Margin::symmetric(12, 6)))
            .show(ui, |ui| refocus = self.embed_bar(ui, snap));
        let mut game_rect = egui::Rect::NOTHING;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ui, |ui| {
            let r = ui.max_rect();
            game_rect = r;
            let ppp = ui.ctx().pixels_per_point();
            let area = [
                (r.min.x * ppp).round() as i32,
                (r.min.y * ppp).round() as i32,
                (r.width() * ppp).round() as i32,
                (r.height() * ppp).round() as i32,
            ];
            if !preview {
                if let Some(parent) = Self::own_hwnd(frame) {
                    self.embed_status = self.embed.update(parent, area);
                }
            }
            if refocus {
                self.embed.focus_game();
            }
            if let Some((gw, gh)) = self.embed.game_size() {
                if gw > area[2] || gh > area[3] {
                    self.message = Some((
                        format!(
                            "Oyun sığmıyor ({gw}x{gh}, yer {}x{}): oyunda daha küçük çözünürlük seç",
                            area[2], area[3]
                        ),
                        true,
                    ));
                } else if self.message.as_ref().is_some_and(|m| m.0.starts_with("Oyun sığmıyor")) {
                    self.message = None;
                }
            }
            if self.embed_status != crate::embed::Status::Embedded {
                ui.vertical_centered(|ui| {
                    ui.add_space((r.height() / 2.0 - 60.0).max(20.0));
                    shadowed(ui, "Growtopia bekleniyor", 22.0, th().text);
                    ui.label(RichText::new("Oyun açılınca bu alana yerleşir.").color(th().muted));
                    ui.add_space(10.0);
                    if self.running() && gt_button(ui, "Growtopia'yı aç", th().accent).clicked() {
                        self.launch_game();
                    }
                });
            }
        });
        if self.side_open {
            self.side_overlay(ui, frame, snap, preview, game_rect);
        } else {
            // The panel window is gone; a new one has to be owned again.
            self.side_owner = None;
        }
    }

    /// The page panel over the left of the game. The game is a separate
    /// window that our window cannot paint over, so outside the preview the
    /// panel is its own borderless window, owned by ours (stays above it and
    /// the game, minimises with it).
    fn side_overlay(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame, snap: &Snapshot, preview: bool, area: egui::Rect) {
        let width = SIDE_WIDTH.min(area.width());
        if preview {
            let rect = egui::Rect::from_min_size(area.min, Vec2::new(width, area.height()));
            egui::Area::new(egui::Id::unique("side_preview")).fixed_pos(rect.min).show(ui.ctx(), |ui| {
                egui::Frame::new().fill(th().sidebar).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.set_min_size(rect.size() - Vec2::splat(20.0));
                    ui.set_max_size(rect.size() - Vec2::splat(20.0));
                    self.side_contents(ui, snap, preview);
                });
            });
            return;
        }
        let Some(inner) = ui.ctx().input(|i| i.viewport().inner_rect) else { return };
        // Owned by the game window, so it always stays above the game.
        let owner = self.embed.game_hwnd().or_else(|| Self::own_hwnd(frame));
        let builder = egui::ViewportBuilder::default()
            .with_title(SIDE_TITLE)
            .with_decorations(false)
            .with_taskbar(false)
            .with_resizable(false)
            .with_position(inner.min + area.min.to_vec2())
            .with_inner_size(Vec2::new(width, area.height()));
        let ctx = ui.ctx().clone();
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("gtproxy_side"), builder, |ui, _| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                self.side_open = false;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(th().sidebar).inner_margin(Margin::same(10)))
                .show(ui, |ui| self.side_contents(ui, snap, false));
        });
        if owner.is_some() && self.side_owner != owner {
            if crate::embed::own_window(SIDE_TITLE, owner.unwrap_or_default()) {
                self.side_owner = owner;
            }
        }
    }

    fn side_contents(&mut self, ui: &mut egui::Ui, snap: &Snapshot, preview: bool) {
        ui.horizontal_wrapped(|ui| {
            for page in Page::ALL {
                if chip(ui, page.title(), self.page == page).clicked() {
                    self.page = page;
                }
            }
            if gt_small(ui, "Kapat", RED).clicked() {
                self.side_open = false;
            }
        });
        ui.add_space(6.0);
        if self.page.needs_proxy() && !self.running() && !preview {
            self.proxy_off(ui);
            return;
        }
        egui::ScrollArea::both().id_salt("embed_page").auto_shrink(false).show(ui, |ui| {
            // Narrower than this, labels break letter by letter: scroll sideways instead.
            ui.set_min_width(360.0);
            match self.page {
                Page::Overview => self.overview(ui, snap),
                Page::Features => self.features(ui, snap),
                Page::World => self.world(ui, snap),
                Page::Roulette => self.roulette(ui, snap),
                Page::Log => self.log(ui),
                Page::Settings => self.settings(ui),
            }
        });
    }

    /// The always-visible bar: state, world, quick switches, warp. Returns
    /// `true` when a button was used, so the keyboard can go back to the game.
    fn embed_bar(&mut self, ui: &mut egui::Ui, s: &Snapshot) -> bool {
        let mut used = false;
        ui.horizontal_wrapped(|ui| {
            if chip(ui, if self.side_open { "Paneli kapat" } else { "Panel" }, self.side_open).clicked() {
                self.side_open = !self.side_open;
                self.side_owner = None;
            }
            shadowed(ui, "GTProxy", 18.0, th().text);
            let (text, colour) = connection_state(self.running(), s);
            status_pill(ui, text, colour);
            let (wl, dl, bgl) = s.locks;
            let ping = s.server_ping_ms.map_or("-".to_string(), |p| format!("{p} ms"));
            ui.label(
                RichText::new(format!(
                    "{}  ·  {ping}  ·  {} WL",
                    s.world.as_deref().unwrap_or("-"),
                    wl + dl * 100 + bgl * 10_000
                ))
                .strong(),
            );
            ui.add_space(8.0);
            let edit = ui.add(egui::TextEdit::singleline(&mut self.warp).hint_text("Dünya").desired_width(110.0));
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (gt_small(ui, "Git", th().accent).clicked() || enter) && !self.warp.trim().is_empty() {
                self.cmd(format!("warp {}", self.warp.trim()));
                used = true;
            }
            ui.add_space(8.0);
            for (label, on, c) in [
                ("Işınlanma", s.teleport, "tp"),
                ("Oto toplama", s.auto_collect, "ac"),
                ("Fast drop", s.fast_drop, "fastdrop"),
            ] {
                if chip(ui, label, on).clicked() {
                    self.cmd(c);
                    used = true;
                }
            }
            ui.add_space(8.0);
            if self.running() {
                if gt_small(ui, "Durdur", RED).clicked() {
                    self.stop();
                }
            } else if gt_small(ui, "Başlat", th().accent).clicked() {
                self.start();
            }
            if gt_small(ui, "Normal görünüm", th().button).clicked() {
                self.cfg.ui.embed_game = false;
                if let Err(e) = self.cfg.save() {
                    self.error(format!("{e:#}"));
                }
            }
            // No status bar in this mode: the last message sits at the end of the bar.
            match &self.message {
                Some((text, true)) => {
                    ui.add(egui::Label::new(RichText::new(text).color(DANGER)).truncate());
                }
                Some((text, false)) => {
                    ui.add(egui::Label::new(RichText::new(text).color(th().muted)).truncate());
                }
                None => {}
            }
        });
        used
    }
}

impl eframe::App for ProxyApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Our window is about to go; the game window must not go with it.
        self.embed.release();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            self.embed.release();
        }
        if std::mem::take(&mut self.autostart) {
            self.start();
        }
        // The proxy stopped by itself (port in use, crash...).
        if self.engine.as_ref().is_some_and(Engine::is_finished) {
            self.stop();
            if !self.message.as_ref().is_some_and(|m| m.1) {
                self.error("Proxy beklenmedik şekilde durdu, ayrıntılar Log sayfasında.");
            }
        }
        let preview = preview_page();
        if let Some(page) = preview {
            self.page = page;
            if let Ok(scene) = std::env::var("GTPROXY_PREVIEW_SCENE") {
                self.cfg.ui.backdrop = scene;
            }
        }
        let snap = match preview {
            Some(_) => demo_snapshot(),
            None => self.engine.as_ref().map(|e| e.shared.snapshot()).unwrap_or_default(),
        };
        ui.ctx().request_repaint_after(Duration::from_millis(300));

        if self.cfg.ui != self.applied_ui {
            apply_style(ui.ctx(), &self.cfg.ui);
            self.applied_ui = self.cfg.ui.clone();
        }
        use crate::gui_background::Scene;
        let scene = match (self.cfg.ui.backdrop.as_str(), &self.backdrop) {
            (_, None) | ("plain", _) => None,
            ("connecting", Some(_)) => Some(Scene::Connecting),
            (_, Some(_)) => Some(Scene::Title),
        };
        if let (Some(scene), Some(b)) = (scene, &self.backdrop) {
            b.paint(ui.painter(), ui.max_rect(), scene);
        }
        let page_fill = if scene.is_some() { Color32::TRANSPARENT } else { th().bg };

        let embed_on = self.cfg.ui.embed_game || (preview.is_some() && std::env::var("GTPROXY_EMBED").is_ok());
        if preview.is_some() && std::env::var("GTPROXY_EMBED").as_deref() == Ok("panel") {
            self.side_open = true;
        }
        if embed_on != self.embed_was_on {
            self.embed_was_on = embed_on;
            // Full screen: the game needs every pixel of height it can get.
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(embed_on));
            if !embed_on {
                self.embed.release();
                self.embed_status = crate::embed::Status::Off;
            }
        }
        if embed_on {
            self.embedded_ui(ui, frame, &snap, preview.is_some());
            if preview.is_some() {
                preview_screenshot(ui.ctx());
            }
            return;
        }

        egui::Panel::left("sidebar")
            .exact_size(200.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(th().sidebar.gamma_multiply(th().opacity)).inner_margin(Margin::same(16)))
            .show(ui, |ui| self.sidebar(ui, &snap));
        egui::Panel::bottom("status")
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(th().sidebar.gamma_multiply(th().opacity)).inner_margin(Margin::symmetric(16, 6)))
            .show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(page_fill).inner_margin(Margin::same(24)))
            .show(ui, |ui| {
                self.page_header(ui);
                if self.page.needs_proxy() && !self.running() && preview.is_none() {
                    self.proxy_off(ui);
                    return;
                }
                egui::ScrollArea::vertical().id_salt("page").auto_shrink(false).show(ui, |ui| match self.page {
                    Page::Overview => self.overview(ui, &snap),
                    Page::Features => self.features(ui, &snap),
                    Page::World => self.world(ui, &snap),
                    Page::Roulette => self.roulette(ui, &snap),
                    Page::Log => self.log(ui),
                    Page::Settings => self.settings(ui),
                });
            });
        if preview.is_some() {
            preview_screenshot(ui.ctx());
        }
    }
}

/// Debug builds: with `GTPROXY_SHOT=<file.bmp>` the preview saves a screenshot
/// of itself after a few frames and closes (for checking layouts).
fn preview_screenshot(ctx: &egui::Context) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static FRAMES: AtomicU32 = AtomicU32::new(0);
    let Ok(path) = std::env::var("GTPROXY_SHOT") else { return };
    if FRAMES.fetch_add(1, Ordering::Relaxed) == 30 {
        let ctx2 = ctx.clone();
        ctx.request_screenshot(move |image| {
            let [w, h] = image.size;
            let row = (w * 3 + 3) & !3;
            let mut bmp = Vec::with_capacity(54 + row * h);
            bmp.extend_from_slice(b"BM");
            bmp.extend_from_slice(&((54 + row * h) as u32).to_le_bytes());
            bmp.extend_from_slice(&[0; 4]);
            bmp.extend_from_slice(&54u32.to_le_bytes());
            bmp.extend_from_slice(&40u32.to_le_bytes());
            bmp.extend_from_slice(&(w as i32).to_le_bytes());
            bmp.extend_from_slice(&(h as i32).to_le_bytes());
            bmp.extend_from_slice(&1u16.to_le_bytes());
            bmp.extend_from_slice(&24u16.to_le_bytes());
            bmp.extend_from_slice(&[0; 24]);
            for y in (0..h).rev() {
                for x in 0..w {
                    let c = image.pixels[y * w + x];
                    bmp.extend_from_slice(&[c.b(), c.g(), c.r()]);
                }
                bmp.resize(bmp.len() + (row - w * 3), 0);
            }
            let _ = std::fs::write(&path, bmp);
            ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
            ctx2.request_repaint();
        });
    }
    ctx.request_repaint();
}

impl Drop for ProxyApp {
    fn drop(&mut self) {
        self.embed.release();
        // Closing the window stops the proxy and restores the hosts file.
        if let Some(e) = self.engine.take() {
            let _ = e.stop();
        }
    }
}

/// Debug builds only: `GTPROXY_PREVIEW=<page number>` shows a page with
/// sample data, to check the layout without a running proxy.
/// Debug builds: `GTPROXY_SIZE=<w>x<h>` sets the starting window size.
fn preview_size() -> Option<[f32; 2]> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let v = std::env::var("GTPROXY_SIZE").ok()?;
    let (w, h) = v.split_once('x')?;
    Some([w.parse().ok()?, h.parse().ok()?])
}

fn preview_page() -> Option<Page> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let n: usize = std::env::var("GTPROXY_PREVIEW").ok()?.parse().ok()?;
    Page::ALL.get(n).copied()
}

fn demo_snapshot() -> Snapshot {
    use crate::state::SpinSummary;
    Snapshot {
        game_connected: true,
        server_connected: true,
        server: Some("213.179.209.178:17172".into()),
        client_ping_ms: Some(1),
        server_ping_ms: Some(62),
        name: Some("Dr.Serhatzzz".into()),
        world: Some("SERHATMAIN".into()),
        level: Some(106),
        gems: Some(1_080_548),
        locks: (23, 4, 1),
        inventory_items: 54,
        players: vec![(3, "Bob".into()), (5, "Alice".into()), (8, "Trader".into())],
        saved_worlds: vec![("BUYALL".into(), "kilit al".into()), ("SERHATMAIN|HOME".into(), "ev".into())],
        fast_drop: true,
        teleport: true,
        teleport_step_ms: 40,
        wheel_zoom: true,
        autosave: true,
        ping_alert_ms: Some(300),
        roulette_enabled: true,
        roulette_instant: true,
        roulette_mode: "Reme".into(),
        auto_pull: true,
        auto_collect: true,
        collect_range: 3,
        vends_known: 1920,
        spin_history: vec![("Bob".into(), 29, 'b'), ("Alice".into(), 7, '4'), ("Bob".into(), 0, '2')],
        spins: SpinSummary {
            total: 42,
            red: 20,
            black: 19,
            green: 3,
            recent: vec![(29, 'b'), (7, '4'), (0, '2'), (23, '4'), (16, '4'), (22, 'b'), (34, '4')],
        },
        ..Default::default()
    }
}

fn connection_state(running: bool, s: &Snapshot) -> (&'static str, Color32) {
    if !running {
        ("Kapalı", th().muted)
    } else if s.game_connected && s.server_connected {
        ("Bağlı", th().accent)
    } else if s.game_connected {
        ("Bağlanıyor", WARN)
    } else {
        ("Oyun bekleniyor", WARN)
    }
}

// ── Components ───────────────────────────────────────────────────────────────

/// A game-style panel: dark outer line, light rim, teal face, optional
/// shadowed title, content on a darker inset.
fn card<R>(ui: &mut egui::Ui, title: Option<&str>, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let r = egui::Frame::new()
        .fill(PANEL_OUTER.gamma_multiply(th().opacity * 0.83))
        .corner_radius(CornerRadius::same(th().radius + 1))
        .inner_margin(Margin::same(1))
        .show(ui, |ui| {
            egui::Frame::new()
                .fill(th().panel.gamma_multiply(th().opacity))
                .stroke(Stroke::new(3.0, th().rim))
                .corner_radius(CornerRadius::same(th().radius))
                .inner_margin(Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if let Some(t) = title {
                        shadowed(ui, t, 18.0, th().text);
                        ui.add_space(2.0);
                    }
                    egui::Frame::new()
                        .fill(th().inset.gamma_multiply((th().opacity * 0.93).min(1.0)))
                        .corner_radius(CornerRadius::same(th().radius.saturating_sub(2)))
                        .inner_margin(Margin::same(10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            add(ui)
                        })
                        .inner
                })
                .inner
        })
        .inner;
    ui.add_space(10.0);
    r
}

/// A small summary card: label above, big value below.
fn stat(ui: &mut egui::Ui, label: &str, value: &str, colour: Color32) {
    card(ui, None, |ui| {
        ui.label(RichText::new(label).small().color(th().muted));
        shadowed(ui, value, 20.0, colour);
    });
}

/// Lays out `count` blocks in as many columns as fit (each at least `min_width`
/// wide), so narrow windows get one column instead of overlapping cards.
/// Columns always divide `count`, so rows stay even.
fn flow(ui: &mut egui::Ui, count: usize, min_width: f32, mut add: impl FnMut(&mut egui::Ui, usize)) {
    let gap = ui.spacing().item_spacing.x;
    let mut cols = (((ui.available_width() + gap) / (min_width + gap)).floor() as usize).clamp(1, count.max(1));
    while count % cols != 0 {
        cols -= 1;
    }
    for start in (0..count).step_by(cols) {
        ui.columns(cols, |c| {
            for (j, col) in c.iter_mut().enumerate() {
                add(col, start + j);
            }
        });
    }
}

fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(th().muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(value).truncate());
        });
    });
}

fn two_line(ui: &mut egui::Ui, title: &str, desc: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add(egui::Label::new(title).wrap());
        ui.add(egui::Label::new(RichText::new(desc).small().color(th().muted)).wrap());
    });
}

/// Title and description on the left, `right` (buttons, a switch) on the
/// right. The right side is laid out first so it never gets pushed out;
/// the text wraps in what is left.
fn labeled_row<R>(ui: &mut egui::Ui, title: &str, desc: &str, right: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let r = right(ui);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| two_line(ui, title, desc));
            r
        })
        .inner
    })
    .inner
}

/// Title, description and a switch on the right. Returns true when clicked.
fn setting(ui: &mut egui::Ui, title: &str, desc: &str, on: bool) -> bool {
    labeled_row(ui, title, desc, |ui| switch(ui, on).clicked())
}

/// Fixed-width label column and a field that fills the rest of the row.
fn field_row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(110.0, 26.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(110.0);
            ui.label(RichText::new(label).color(th().muted));
        });
        add(ui);
    });
}

fn switch(ui: &mut egui::Ui, on: bool) -> egui::Response {
    let size = Vec2::new(38.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let t = ui.ctx().animate_bool(response.id, on);
    let off_colour = if response.hovered() { shade(FIELD, 1.3) } else { FIELD };
    let bg = lerp_colour(off_colour, th().accent, t);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(11), bg);
    let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
    painter.circle_filled(egui::pos2(x, rect.center().y), 8.0, Color32::WHITE);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn lerp_colour(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn divider(ui: &mut egui::Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, BORDER);
}

fn chip(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    gt_small(ui, label, if selected { YELLOW } else { th().button })
}

/// The big green "Play Online" button.
fn primary_button(ui: &mut egui::Ui, label: &str, width: f32) -> egui::Response {
    gt_button_sized(ui, label, th().accent, Vec2::new(width, 46.0), 19.0)
}

fn secondary_button(ui: &mut egui::Ui, label: &str, width: f32, colour: Color32) -> egui::Response {
    let fill = if colour == DANGER { RED } else { th().button };
    gt_button_sized(ui, label, fill, Vec2::new(width, 38.0), 16.0)
}

/// White text with a soft drop shadow, the way the game draws titles.
fn shadowed(ui: &mut egui::Ui, text: &str, size: f32, colour: Color32) {
    let mut font = egui::FontId::proportional(size * th().font_scale);
    let mut galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), colour);
    // Too wide for the space (a long value in a small card): shrink, never overflow.
    let room = ui.available_width() - 4.0;
    if galley.size().x > room && room > 20.0 {
        font.size = (font.size * room / galley.size().x).max(10.0);
        galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), colour);
    }
    let (rect, _) = ui.allocate_exact_size(galley.size() + Vec2::splat(3.0), Sense::hover());
    shadow_text(ui.painter(), rect.left_top(), egui::Align2::LEFT_TOP, text, font, colour);
}

fn shadow_text(painter: &egui::Painter, pos: egui::Pos2, align: egui::Align2, text: &str, font: egui::FontId, colour: Color32) {
    let off = (font.size / 14.0).clamp(1.0, 3.0);
    painter.text(pos + Vec2::splat(off), align, text, font.clone(), SHADOW);
    painter.text(pos, align, text, font, colour);
}

fn shade(c: Color32, f: f32) -> Color32 {
    let m = |v: u8| (v as f32 * f).clamp(0.0, 255.0) as u8;
    Color32::from_rgb(m(c.r()), m(c.g()), m(c.b()))
}

/// The game's raised button: coloured face, darker bottom edge, shadowed white label.
fn gt_button_sized(ui: &mut egui::Ui, label: &str, colour: Color32, min: Vec2, font_size: f32) -> egui::Response {
    let font = egui::FontId::proportional(font_size * th().font_scale);
    let text_size = ui.painter().layout_no_wrap(label.to_string(), font.clone(), th().text).size();
    let size = Vec2::new((text_size.x + 26.0).max(min.x), (text_size.y + 14.0).max(min.y));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let pressed = response.is_pointer_button_down_on();
        let face_colour = if !enabled {
            shade(colour, 0.55)
        } else if response.hovered() {
            shade(colour, 1.1)
        } else {
            colour
        };
        let edge = 4.0;
        let sink = if pressed { edge - 1.0 } else { 0.0 };
        let painter = ui.painter();
        let body = egui::Rect::from_min_max(rect.min + Vec2::new(0.0, sink), rect.max);
        painter.rect_filled(body, CornerRadius::same(6), shade(face_colour, 0.62));
        let face = egui::Rect::from_min_max(body.min, body.max - Vec2::new(0.0, edge - sink));
        painter.rect_filled(face, CornerRadius::same(6), face_colour);
        let label_colour = if enabled { th().text } else { Color32::from_gray(210) };
        shadow_text(painter, face.center(), egui::Align2::CENTER_CENTER, label, font, label_colour);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn gt_button(ui: &mut egui::Ui, label: &str, colour: Color32) -> egui::Response {
    gt_button_sized(ui, label, colour, Vec2::new(0.0, 34.0), 15.0)
}

fn gt_small(ui: &mut egui::Ui, label: &str, colour: Color32) -> egui::Response {
    gt_button_sized(ui, label, colour, Vec2::new(0.0, 28.0), 13.0)
}

fn nav_item(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let width = ui.available_width();
    if selected {
        return gt_button_sized(ui, label, th().button, Vec2::new(width, 38.0), 16.0);
    }
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 38.0), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), th().inset);
    }
    shadow_text(
        painter,
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(16.0 * th().font_scale),
        if response.hovered() { th().text } else { th().muted },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn status_dot(ui: &mut egui::Ui, colour: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, colour);
}

fn status_pill(ui: &mut egui::Ui, text: &str, colour: Color32) {
    egui::Frame::new()
        .fill(colour.gamma_multiply(0.15))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                status_dot(ui, colour);
                ui.label(RichText::new(text).small().color(colour));
            });
        });
}

/// A roulette number in its wheel colour.
fn spin_ball(ui: &mut egui::Ui, number: u8, colour: char) {
    let fill = match colour {
        '4' => DANGER,
        '2' => th().accent,
        _ => Color32::from_rgb(34, 34, 38),
    };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(30.0, 30.0), Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), 14.0, fill);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        number.to_string(),
        egui::FontId::proportional(12.5),
        Color32::WHITE,
    );
}

// ── Administrator restart ────────────────────────────────────────────────────

#[cfg(windows)]
fn relaunch_as_admin() -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(hwnd: isize, op: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
    }
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let exe = std::env::current_exe()?;
    let dir = std::env::current_dir()?;
    let (op, file, params, dir) = (wide("runas".as_ref()), wide(exe.as_os_str()), wide("--start".as_ref()), wide(dir.as_os_str()));
    const SW_SHOWNORMAL: i32 = 1;
    // SAFETY: all strings are NUL-terminated UTF-16 buffers alive for the call.
    let r = unsafe { ShellExecuteW(0, op.as_ptr(), file.as_ptr(), params.as_ptr(), dir.as_ptr(), SW_SHOWNORMAL) };
    if r > 32 { Ok(()) } else { anyhow::bail!("Windows izin vermedi (kod {r})") }
}

#[cfg(not(windows))]
fn relaunch_as_admin() -> anyhow::Result<()> {
    anyhow::bail!("run as root")
}
