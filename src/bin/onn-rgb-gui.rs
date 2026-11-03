#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use eframe::egui;
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
                            self.settings.speed = self.settings.speed.min(m.max_speed());
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
                let max = self.settings.mode.max_speed();
                let mut s = self.settings.speed.min(max);
                let speed_enabled = matches!(self.settings.mode, Mode::Pulse | Mode::Flash);
                if ui
                    .add_enabled(speed_enabled, egui::Slider::new(&mut s, 0..=max).text("slow → fast"))
                    .on_disabled_hover_text("Speed only affects Pulse (0-9) and Flash (0-3)")
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
            .with_inner_size([380.0, 260.0])
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
            Ok(Box::new(App::new()))
        }),
    );
    let _ = std::fs::remove_file(&sock);
    after_close(started, started_at);
    res
}
