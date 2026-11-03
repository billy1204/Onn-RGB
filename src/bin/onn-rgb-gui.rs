#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use eframe::egui;
use onn_rgb::fnkeys::mode_key_conflict;
use onn_rgb::hotkeys::{key_name, NAMES};
use onn_rgb::{config, Keyboard, Mode, Settings, MAX_BRIGHTNESS, MAX_SPEED};

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
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("onn Keyboard Lighting");
            ui.add_space(10.0);

            if self.kb.is_none() {
                ui.colored_label(egui::Color32::LIGHT_RED, &self.message);
                ui.add_space(6.0);
                if ui.button("Retry").clicked() {
                    self.connect();
                }
                return;
            }

            egui::Grid::new("controls").num_columns(2).spacing([16.0, 12.0]).show(ui, |ui| {
                ui.label("Mode");
                ui.horizontal(|ui| {
                    for m in Mode::ALL {
                        if ui.selectable_label(self.settings.mode == m, m.label()).clicked() {
                            self.settings.mode = m;
                            self.apply(|kb| kb.set_mode(m));
                        }
                    }
                });
                ui.end_row();

                ui.label("Brightness");
                let mut b = self.settings.brightness.min(MAX_BRIGHTNESS);
                if ui.add(egui::Slider::new(&mut b, 0..=MAX_BRIGHTNESS)).changed() {
                    self.settings.brightness = b;
                    self.apply(|kb| kb.set_brightness(b));
                }
                ui.end_row();

                ui.label("Speed");
                let mut s = self.settings.speed.min(MAX_SPEED);
                let speed_enabled = self.settings.mode == Mode::Pulse;
                if ui
                    .add_enabled(speed_enabled, egui::Slider::new(&mut s, 0..=MAX_SPEED).text("slow → fast"))
                    .on_disabled_hover_text("Speed only affects Pulse (Flash has fixed timing)")
                    .changed()
                {
                    self.settings.speed = s;
                    self.apply(|kb| kb.set_speed(s));
                }
                ui.end_row();
            });

            ui.add_space(8.0);
            ui.collapsing("Fn mode keys", |ui| {
                ui.label("Fn+arrows always set speed (Up/Right faster, Down/Left slower).");
                egui::Grid::new("fnmodes").num_columns(2).show(ui, |ui| {
                    for (m, k) in self.mode_keys.iter_mut() {
                        ui.label(format!("Fn + … → {}", m.label()));
                        let shown = k.map_or("(none)".to_string(), |c| key_name(c).trim_start_matches("KEY_").to_string());
                        egui::ComboBox::from_id_salt(m.name()).selected_text(shown).height(300.0).show_ui(ui, |ui| {
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
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
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
                    ui.label(&self.keys_msg);
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Reload").clicked() {
                    self.reload();
                }
                ui.weak(&self.message);
            });
        });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("onn Keyboard Lighting")
            .with_inner_size([380.0, 260.0])
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native("onn-rgb", options, Box::new(|_| Ok(Box::new(App::new()))))
}
