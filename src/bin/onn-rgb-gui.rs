#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use eframe::egui;
use egui::{
    pos2, vec2, Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, Painter, Pos2, Rect, Response, RichText, Sense,
    Stroke, StrokeKind, Ui, Vec2,
};
use onn_rgb::fnkeys::mode_key_conflict;
use onn_rgb::hotkeys::{key_name, NAMES};
use onn_rgb::instance;
use std::io::Read;
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use onn_rgb::{config, Keyboard, Mode, Settings, MAX_BRIGHTNESS};

// ---- Look and feel (visual only; no keyboard I/O below this line until `App`) ----

const ACCENT: Color32 = Color32::from_rgb(56, 189, 248);
const ACCENT_BG: Color32 = Color32::from_rgb(14, 58, 82);
const ACCENT_BAND: Color32 = Color32::from_rgb(9, 42, 60);
const BG: Color32 = Color32::from_rgb(22, 22, 25);
const CARD: Color32 = Color32::from_rgb(33, 33, 37);
const CARD_HOVER: Color32 = Color32::from_rgb(43, 43, 49);
const BORDER: Color32 = Color32::from_rgb(56, 56, 62);
const BORDER_HI: Color32 = Color32::from_rgb(84, 84, 92);
const TEXT: Color32 = Color32::from_rgb(232, 232, 236);
const MUTED: Color32 = Color32::from_rgb(150, 150, 158);
const OK_GREEN: Color32 = Color32::from_rgb(120, 220, 160);
const OK_BG: Color32 = Color32::from_rgb(22, 48, 34);
const ERR_RED: Color32 = Color32::from_rgb(248, 113, 113);
const ERR_BG: Color32 = Color32::from_rgb(56, 24, 26);
/// Keycap tint, left to right: warm white to amber (the LEDs are one fixed color).
const KEY_WARM: Color32 = Color32::from_rgb(246, 240, 228);
const KEY_AMBER: Color32 = Color32::from_rgb(242, 150, 58);
const KEY_UNLIT: Color32 = Color32::from_rgb(70, 68, 66);

const LABEL_W: f32 = 96.0;
const VALUE_W: f32 = 34.0;

fn setup_style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = Color32::from_rgb(17, 17, 19);
    v.faint_bg_color = CARD;
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.hyperlink_color = ACCENT;
    v.override_text_color = None;
    let r = CornerRadius::same(6);
    let w = &mut v.widgets;
    for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        s.corner_radius = r;
    }
    w.noninteractive.bg_fill = CARD;
    w.noninteractive.weak_bg_fill = CARD;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    w.inactive.bg_fill = Color32::from_rgb(70, 70, 78);
    w.inactive.weak_bg_fill = Color32::from_rgb(42, 42, 48);
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.bg_fill = Color32::from_rgb(96, 96, 106);
    w.hovered.weak_bg_fill = Color32::from_rgb(52, 52, 60);
    w.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.active.bg_fill = ACCENT;
    w.active.weak_bg_fill = ACCENT_BG;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    w.open.weak_bg_fill = Color32::from_rgb(42, 42, 48);
    w.open.bg_stroke = Stroke::new(1.0, ACCENT);
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 8.0);
        s.spacing.button_padding = vec2(16.0, 6.0);
        s.spacing.interact_size.y = 26.0;
        use egui::TextStyle as T;
        s.text_styles.insert(T::Heading, FontId::proportional(18.0));
        s.text_styles.insert(T::Body, FontId::proportional(14.5));
        s.text_styles.insert(T::Button, FontId::proportional(14.5));
        s.text_styles.insert(T::Small, FontId::proportional(12.0));
    });
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
}

/// Small keyboard glyph (header and Steady tile).
fn paint_keyboard(p: &Painter, r: Rect, c: Color32, width: f32) {
    p.rect_stroke(r, CornerRadius::same(4), Stroke::new(width, c), StrokeKind::Inside);
    let inner = r.shrink2(vec2(r.width() * 0.14, r.height() * 0.2));
    let cols = 5;
    let kw = inner.width() / (cols as f32 * 1.6 - 0.6);
    let kh = inner.height() / 4.2;
    for row in 0..2 {
        for col in 0..cols {
            let x = inner.left() + col as f32 * kw * 1.6;
            let y = inner.top() + row as f32 * kh * 1.6;
            p.rect_filled(Rect::from_min_size(pos2(x, y), vec2(kw, kh)), CornerRadius::same(1), c);
        }
    }
    let y = inner.top() + 2.0 * kh * 1.6;
    let bar = Rect::from_min_max(pos2(inner.left() + kw * 1.6, y), pos2(inner.right() - kw * 1.6, y + kh));
    p.rect_filled(bar, CornerRadius::same(1), c);
}

fn paint_power(p: &Painter, r: Rect, c: Color32) {
    let center = r.center() + vec2(0.0, 2.0);
    let rad = r.height().min(r.width()) * 0.46;
    let gap = 38f32.to_radians();
    let top = -std::f32::consts::FRAC_PI_2;
    let (a0, a1) = (top + gap, top + std::f32::consts::TAU - gap);
    let n = 40;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            center + vec2(a.cos(), a.sin()) * rad
        })
        .collect();
    let stroke = Stroke::new(3.0, c);
    p.add(egui::Shape::line(pts, stroke));
    p.line_segment([center + vec2(0.0, -rad - 3.0), center + vec2(0.0, -rad * 0.25)], stroke);
    p.circle_filled(center + vec2(0.0, -rad - 3.0), 1.5, c);
    p.circle_filled(center + vec2(0.0, -rad * 0.25), 1.5, c);
}

/// Dot fields: Pulse fades in evenly, Flash is a streak that grows to the right.
fn paint_dots(p: &Painter, r: Rect, c: Color32, flash: bool) {
    let cols = 11;
    let rows = 3;
    let dx = r.width() / (cols - 1) as f32;
    let dy = r.height() / 4.0;
    for row in 0..rows {
        for col in 0..cols {
            let t = col as f32 / (cols - 1) as f32;
            let (alpha, size) = if flash {
                let rise = t.powf(1.6);
                (0.15 + 0.85 * rise, 0.8 + 2.0 * rise)
            } else {
                let bell = 1.0 - (t - 0.62).abs() * 1.5;
                (bell.clamp(0.12, 1.0), 1.0 + 1.3 * bell.clamp(0.0, 1.0))
            };
            let mut y = r.top() + dy * (row as f32 + 1.0);
            if flash {
                y += (row as f32 - 1.0) * (1.0 - t) * 2.5;
            }
            let pos = pos2(r.left() + dx * col as f32, y);
            p.circle_filled(pos, size, c.gamma_multiply(alpha));
        }
    }
}

fn mode_tile(ui: &mut Ui, mode: Mode, selected: bool, size: Vec2) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let (fill, stroke) = if selected {
            (ACCENT_BG, Stroke::new(1.5, ACCENT))
        } else if resp.hovered() {
            (CARD_HOVER, Stroke::new(1.0, BORDER_HI))
        } else {
            (CARD, Stroke::new(1.0, BORDER))
        };
        p.rect_filled(rect, CornerRadius::same(10), fill);
        let split = rect.top() + rect.height() * 0.64;
        if selected {
            let band = Rect::from_min_max(pos2(rect.left(), split), rect.max);
            p.rect_filled(band, CornerRadius { nw: 0, ne: 0, sw: 10, se: 10 }, ACCENT_BAND);
        }
        p.rect_stroke(rect, CornerRadius::same(10), stroke, StrokeKind::Inside);
        let icon_c = if selected { ACCENT } else { Color32::from_gray(205) };
        let ic = pos2(rect.center().x, rect.top() + rect.height() * 0.34);
        match mode {
            Mode::Steady => paint_keyboard(p, Rect::from_center_size(ic, vec2(42.0, 26.0)), icon_c, 2.0),
            Mode::Off => paint_power(p, Rect::from_center_size(ic, vec2(32.0, 32.0)), icon_c),
            Mode::Pulse => paint_dots(p, Rect::from_center_size(ic, vec2(50.0, 22.0)), icon_c, false),
            Mode::Flash => paint_dots(p, Rect::from_center_size(ic, vec2(50.0, 22.0)), icon_c, true),
        }
        p.text(
            pos2(rect.center().x, rect.top() + rect.height() * 0.82),
            Align2::CENTER_CENTER,
            mode.label(),
            FontId::proportional(15.0),
            if selected { ACCENT } else { TEXT },
        );
    }
    resp
}

/// Rounded status pill, right-aligned in the header.
fn pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(13.0), fg);
    let size = galley.size() + vec2(24.0, 10.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(255), bg);
    p.rect_stroke(rect, CornerRadius::same(255), Stroke::new(1.0, fg.gamma_multiply(0.35)), StrokeKind::Inside);
    p.galley(rect.center() - galley.size() / 2.0, galley, fg);
}

fn header(ui: &mut Ui, connected: bool) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(34.0, 26.0), Sense::hover());
        paint_keyboard(ui.painter(), r, Color32::from_gray(190), 1.5);
        ui.add_space(6.0);
        ui.label(RichText::new("onn Keyboard Lighting").size(17.0).color(TEXT).strong());
        ui.label(
            RichText::new(format!("v{}.{}", env!("CARGO_PKG_VERSION_MAJOR"), env!("CARGO_PKG_VERSION_MINOR")))
                .size(12.5)
                .color(MUTED),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if connected {
                pill(ui, "Connected", OK_GREEN, OK_BG);
            } else {
                pill(ui, "Not connected", ERR_RED, ERR_BG);
            }
        });
    });
    ui.add_space(4.0);
    let r = ui.available_rect_before_wrap();
    ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, BORDER));
    ui.add_space(12.0);
}

/// Decorative keycaps 1-0. Their glow follows mode and brightness; it is not a
/// color picker (the keyboard's LEDs are a single fixed color).
fn keycaps(ui: &mut Ui, s: &Settings) {
    let t = ui.input(|i| i.time) as f32;
    let max = s.mode.max_speed().max(1) as f32;
    let speed = s.speed.min(s.mode.max_speed()) as f32 / max;
    let level = 0.2 + 0.8 * (s.brightness.min(MAX_BRIGHTNESS) as f32 / MAX_BRIGHTNESS as f32);
    let glow = match s.mode {
        Mode::Off => 0.0,
        Mode::Steady => level,
        Mode::Pulse => {
            ui.ctx().request_repaint_after(Duration::from_millis(40));
            let hz = 0.25 + 0.9 * speed;
            level * (0.25 + 0.75 * (0.5 + 0.5 * (t * hz * std::f32::consts::TAU).sin()))
        }
        Mode::Flash => {
            ui.ctx().request_repaint_after(Duration::from_millis(40));
            let hz = 0.4 + 1.0 * speed;
            if (t * hz).fract() < 0.5 { level } else { 0.0 }
        }
    };
    card().inner_margin(Margin::same(10)).show(ui, |ui| {
        let w = ui.available_width();
        let gap = 6.0;
        let kw = (w - gap * 9.0) / 10.0;
        let kh = 40.0;
        let (row, _) = ui.allocate_exact_size(vec2(w, kh), Sense::hover());
        let p = ui.painter();
        for (i, label) in ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"].iter().enumerate() {
            let tint = lerp_color(KEY_WARM, KEY_AMBER, i as f32 / 9.0);
            let lit = lerp_color(KEY_UNLIT, tint, glow);
            let x = row.left() + i as f32 * (kw + gap);
            let cap = Rect::from_min_size(pos2(x, row.top()), vec2(kw, kh));
            if glow > 0.05 {
                p.rect_filled(cap.expand(2.5), CornerRadius::same(7), tint.gamma_multiply(0.22 * glow));
            }
            p.rect_filled(cap, CornerRadius::same(5), lerp_color(lit, Color32::BLACK, 0.35));
            let top = Rect::from_min_max(cap.min + vec2(2.0, 1.5), cap.max - vec2(2.0, 6.0));
            p.rect_filled(top, CornerRadius::same(4), lit);
            p.text(
                top.left_top() + vec2(6.0, 4.0),
                Align2::LEFT_TOP,
                label,
                FontId::proportional(12.5),
                lerp_color(Color32::from_gray(150), Color32::from_rgb(60, 48, 36), glow),
            );
        }
    })
    .response
    .on_hover_text("Preview only: the keyboard's LEDs are a single fixed color.");
}

fn value_box(ui: &mut Ui, v: u8, enabled: bool) {
    let (r, _) = ui.allocate_exact_size(vec2(VALUE_W, 28.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(6), Color32::from_rgb(28, 28, 32));
    p.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.0, BORDER), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, v.to_string(), FontId::proportional(15.0), if enabled { TEXT } else { MUTED });
}

fn row_label(ui: &mut Ui, text: &str, enabled: bool) {
    ui.allocate_ui_with_layout(vec2(LABEL_W, 28.0), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_width(LABEL_W);
        ui.add(egui::Label::new(RichText::new(text).color(if enabled { TEXT } else { MUTED })).selectable(false));
    });
}

struct App {
    kb: Option<Keyboard>,
    settings: Settings,
    message: String,
    /// Fn mode keys being edited (None = unbound).
    mode_keys: [(Mode, Option<u16>); 4],
    keys_msg: String,
}

impl App {
    fn new() -> Self {
        let mut app = App {
            kb: None,
            settings: Settings { mode: Mode::Steady, brightness: MAX_BRIGHTNESS, speed: 0 },
            message: String::new(),
            mode_keys: Mode::ALL.map(|m| (m, None)),
            keys_msg: String::new(),
        };
        let (c, _) = config::load();
        for (m, k) in c.mode_keys {
            if let Some(slot) = app.mode_keys.iter_mut().find(|(o, _)| *o == m) {
                slot.1 = Some(k);
            }
        }
        app.connect();
        app
    }

    fn connect(&mut self) {
        match Keyboard::open() {
            Ok(kb) => {
                self.kb = Some(kb);
                self.reload();
            }
            Err(e) => {
                self.kb = None;
                self.message = e.to_string();
            }
        }
    }

    fn reload(&mut self) {
        let Some(kb) = self.kb.as_mut() else { return };
        match kb.settings() {
            Ok(s) => {
                self.settings = s;
                self.message = format!("Connected ({})", kb.path());
            }
            Err(e) => {
                self.kb = None;
                self.message = e.to_string();
            }
        }
    }

    fn apply(&mut self, f: impl FnOnce(&mut Keyboard) -> onn_rgb::Result<()>) {
        let Some(kb) = self.kb.as_mut() else { return };
        if let Err(e) = f(kb) {
            // Back to the Retry screen, which re-reads the real settings on reconnect.
            self.kb = None;
            self.message = e.to_string();
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(Margin::same(18)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.body(ui));
            });
    }
}

impl App {
    /// Everything inside the window (scrolls if the screen is too short).
    fn body(&mut self, ui: &mut Ui) {
        header(ui, self.kb.is_some());

        if self.kb.is_none() {
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    ui.add_space(10.0);
                    let (r, _) = ui.allocate_exact_size(vec2(56.0, 36.0), Sense::hover());
                    paint_keyboard(ui.painter(), r, MUTED, 2.0);
                    ui.add_space(8.0);
                    ui.label(RichText::new("Keyboard not available").size(16.0).color(TEXT).strong());
                    ui.add_space(2.0);
                    ui.label(RichText::new(&self.message).color(ERR_RED));
                    ui.add_space(10.0);
                    let retry = egui::Button::new(RichText::new("Retry").color(Color32::BLACK).strong())
                        .fill(ACCENT)
                        .min_size(vec2(110.0, 30.0));
                    if ui.add(retry).clicked() {
                        self.connect();
                    }
                    ui.add_space(10.0);
                });
            });
            return;
        }

        // Mode tiles
        let gap = 10.0;
        let tile_w = (ui.available_width() - gap * 3.0) / 4.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for m in Mode::ALL {
                if mode_tile(ui, m, self.settings.mode == m, vec2(tile_w, 96.0)).clicked() {
                    self.settings.mode = m;
                    self.settings.speed = self.settings.speed.min(m.max_speed());
                    self.apply(|kb| kb.set_mode(m));
                }
            }
        });
        if self.kb.is_none() {
            return;
        }
        ui.add_space(12.0);

        keycaps(ui, &self.settings);
        ui.add_space(14.0);

        let slider_w = ui.available_width() - LABEL_W - VALUE_W - 2.0 * ui.spacing().item_spacing.x - 4.0;

        ui.horizontal(|ui| {
            row_label(ui, "Brightness", true);
            ui.spacing_mut().slider_width = slider_w;
            let mut b = self.settings.brightness.min(MAX_BRIGHTNESS);
            let slider = egui::Slider::new(&mut b, 0..=MAX_BRIGHTNESS)
                .show_value(false)
                .trailing_fill(true)
                .handle_shape(egui::style::HandleShape::Circle);
            if ui.add(slider).changed() {
                self.settings.brightness = b;
                self.apply(|kb| kb.set_brightness(b));
            }
            value_box(ui, b, true);
        });
        if self.kb.is_none() {
            return;
        }
        ui.add_space(6.0);

        let max = self.settings.mode.max_speed();
        let mut s = self.settings.speed.min(max);
        let speed_enabled = matches!(self.settings.mode, Mode::Pulse | Mode::Flash);
        ui.horizontal(|ui| {
            row_label(ui, "Speed", speed_enabled);
            ui.spacing_mut().slider_width = slider_w;
            let slider = egui::Slider::new(&mut s, 0..=max)
                .show_value(false)
                .trailing_fill(true)
                .handle_shape(egui::style::HandleShape::Circle);
            if ui
                .add_enabled(speed_enabled, slider)
                .on_disabled_hover_text("Speed only affects Pulse (0-9) and Flash (0-3)")
                .changed()
            {
                self.settings.speed = s;
                self.apply(|kb| kb.set_speed(s));
            }
            value_box(ui, s, speed_enabled);
        });
        if self.kb.is_none() {
            return;
        }
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
            ui.label(RichText::new("slow → fast · Pulse and Flash only").size(12.5).color(MUTED));
        });
        ui.add_space(12.0);

        // Fn mode keys
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::CollapsingHeader::new(RichText::new("Fn mode keys").color(TEXT).size(15.0))
                .default_open(true)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Fn+arrows always set speed (Up/Right faster, Down/Left slower).")
                            .color(MUTED)
                            .size(13.0),
                    );
                    ui.add_space(4.0);
                    egui::Grid::new("fnmodes").num_columns(3).spacing([12.0, 8.0]).show(ui, |ui| {
                        for (m, k) in self.mode_keys.iter_mut() {
                            let short = k.map(|c| key_name(c).trim_start_matches("KEY_").to_string());
                            ui.label(RichText::new(format!("Fn + {}", short.as_deref().unwrap_or("…"))).color(TEXT));
                            ui.label(RichText::new(format!("→  {}", m.label())).color(TEXT));
                            let shown = short.unwrap_or_else(|| "(none)".to_string());
                            egui::ComboBox::from_id_salt(m.name())
                                .selected_text(shown)
                                .width(150.0)
                                .height(300.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(k, None, "(none)");
                                    for (n, c) in NAMES {
                                        if matches!(*c, 103 | 105 | 106 | 108 | 115) {
                                            continue; // speed arrows and the Fn key itself
                                        }
                                        ui.selectable_value(k, Some(*c), n.trim_start_matches("KEY_"));
                                    }
                                });
                            ui.end_row();
                        }
                    });
                });
        });
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            if ui.button("Save").on_hover_text("Save the Fn mode keys").clicked() {
                let keys: Vec<(Mode, u16)> = self.mode_keys.iter().filter_map(|(m, k)| k.map(|k| (*m, k))).collect();
                let conflict = keys.iter().find_map(|&(m, k)| mode_key_conflict(&keys, m, k));
                self.keys_msg = match conflict {
                    Some(e) => format!("Not saved: {e}"),
                    None => match config::save_mode_keys(&keys) {
                        Ok(()) => "Saved. Restart the daemon (or log out/in) to apply.".into(),
                        Err(e) => format!("Not saved: {e}"),
                    },
                };
            }
            if ui.button("Reload").on_hover_text("Re-read the keyboard's current settings").clicked() {
                self.reload();
            }
        });
        if !self.keys_msg.is_empty() {
            let c = if self.keys_msg.starts_with("Not saved") { ERR_RED } else { OK_GREEN };
            ui.label(RichText::new(&self.keys_msg).color(c).size(12.5));
        }
        ui.label(RichText::new(&self.message).color(MUTED).size(12.0));
    }
}

/// Set when the tray's Quit closed this window (skip the "still running" hint).
static QUIT_FROM_TRAY: AtomicBool = AtomicBool::new(false);

/// Commands from a second launch ("show") or the tray's Quit ("quit").
fn listen(l: UnixListener, ctx: egui::Context) {
    for conn in l.incoming() {
        let Ok(mut c) = conn else { continue };
        let mut buf = String::new();
        let _ = c.read_to_string(&mut buf);
        match buf.trim() {
            "show" => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                    egui::UserAttentionType::Informational,
                ));
            }
            "quit" => {
                QUIT_FROM_TRAY.store(true, Ordering::SeqCst);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {}
        }
        ctx.request_repaint();
    }
}

/// Start the tray daemon in the background if none is running, so the app
/// keeps running in the tray after this window closes.
fn start_daemon_if_needed() -> Option<Child> {
    if instance::daemon_running() {
        return None;
    }
    let mut cmd = match std::env::var_os("APPIMAGE") {
        Some(a) => Command::new(a),
        None => {
            let sib = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("onn-rgb")));
            Command::new(sib.filter(|p| p.exists()).map(|p| p.into_os_string()).unwrap_or_else(|| "onn-rgb".into()))
        }
    };
    cmd.arg("daemon").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // Own session: closing the terminal or this window doesn't take it down.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    match cmd.spawn() {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("onn-rgb-gui: can't start the tray daemon: {e}");
            None
        }
    }
}

/// After the window closes: stay in the tray if there is one; otherwise stop
/// the daemon we started, so nothing is left running with no way to reach it.
fn after_close(started: Option<Child>, started_at: Instant) {
    if QUIT_FROM_TRAY.load(Ordering::SeqCst) {
        return;
    }
    // A daemon we just started may still be registering its tray icon.
    if started.is_some() {
        while !instance::tray_present() && started_at.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    if instance::tray_present() {
        first_close_hint();
    } else if let Some(mut c) = started {
        unsafe { libc::kill(c.id() as libc::pid_t, libc::SIGTERM) };
        let _ = c.wait();
    }
}

fn first_close_hint() {
    let Some(flag) = config::path().and_then(|p| p.parent().map(|d| d.join("tray-hint-shown"))) else { return };
    if flag.exists() {
        return;
    }
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "onn-rgb", "-i", "input-keyboard", "onn Keyboard Lighting",
               "Still running in the tray. Right-click the tray icon and choose Quit to exit."])
        .status();
    let _ = std::fs::write(flag, "");
}

fn main() -> eframe::Result {
    // Single window: a second launch brings the running one forward.
    let Some(_gui_lock) = instance::try_lock(&instance::gui_lock_path()) else {
        if !instance::send_gui("show") {
            eprintln!("onn-rgb-gui: another window is already open");
        }
        return Ok(());
    };
    let sock = instance::gui_socket();
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).ok();
    let started_at = Instant::now();
    let started = start_daemon_if_needed();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("onn Keyboard Lighting")
            .with_app_id("onn-rgb")
            .with_inner_size([440.0, 720.0])
            .with_resizable(false),
        ..Default::default()
    };
    let res = eframe::run_native(
        "onn-rgb",
        options,
        Box::new(move |cc| {
            if let Some(l) = listener {
                let ctx = cc.egui_ctx.clone();
                std::thread::spawn(move || listen(l, ctx));
            }
            setup_style(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    );
    let _ = std::fs::remove_file(&sock);
    after_close(started, started_at);
    res
}
