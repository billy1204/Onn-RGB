#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use eframe::egui;
use onn_rgb::{Keyboard, Mode, Settings, MAX_BRIGHTNESS, MAX_MODE, MAX_SPEED};

struct App {
    kb: Option<Keyboard>,
    settings: Settings,
    message: String,
}

impl App {
    fn new() -> Self {
        let mut app = App {
            kb: None,
            settings: Settings { mode: Mode::Steady, brightness: MAX_BRIGHTNESS, speed: 0 },
            message: String::new(),
        };
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
                    for m in Mode::NAMED {
                        let label = match m { Mode::Pulse => "Pulse", Mode::Steady => "Steady", _ => "Off" };
                        if ui.selectable_label(self.settings.mode == m, label).clicked() {
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
                let speed_enabled = self.settings.mode == Mode::Pulse || matches!(self.settings.mode, Mode::Raw(_));
                if ui
                    .add_enabled(speed_enabled, egui::Slider::new(&mut s, 0..=MAX_SPEED).text("slow → fast"))
                    .on_disabled_hover_text("Speed only affects animated modes")
                    .changed()
                {
                    self.settings.speed = s;
                    self.apply(|kb| kb.set_speed(s));
                }
                ui.end_row();
            });

            ui.add_space(8.0);
            ui.collapsing("Advanced", |ui| {
                ui.horizontal(|ui| {
                    ui.label("Raw mode");
                    let mut m = self.settings.mode.raw();
                    let resp = ui.add(egui::Slider::new(&mut m, 0..=MAX_MODE));
                    if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                        let mode = Mode::from_raw(m);
                        self.settings.mode = mode;
                        self.apply(|kb| kb.set_mode(mode));
                    }
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
