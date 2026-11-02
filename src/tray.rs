//! StatusNotifierItem tray icon (KDE, COSMIC, Cinnamon, XFCE, LXQt, Budgie,
//! GNOME with the AppIndicator extension, ...). Event-driven: it never polls;
//! the daemon's worker pushes new state after every change.

use crate::daemon::{Cmd, Ctl};
use crate::{Mode, Settings, MAX_BRIGHTNESS, MAX_SPEED};
use ksni::menu::*;
use ksni::{MenuItem, ToolTip};
use std::sync::mpsc::Sender;

pub struct OnnTray {
    pub settings: Option<Settings>,
    pub error: Option<String>,
    /// Hotkey/grab status line (e.g. "Fn combos active (grabbed)").
    pub status: String,
    pub block_combo_keys: bool,
    pub tx: Sender<Cmd>,
    pub ctl: Ctl,
}

pub fn open_gui() {
    let sibling = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("onn-rgb-gui")));
    let prog = sibling.filter(|p| p.exists()).map(|p| p.into_os_string()).unwrap_or_else(|| "onn-rgb-gui".into());
    if let Err(e) = std::process::Command::new(&prog).spawn() {
        eprintln!("onn-rgb: can't start {}: {e}", prog.to_string_lossy());
    }
}

impl OnnTray {
    fn summary(&self) -> String {
        match (&self.settings, &self.error) {
            (_, Some(e)) => e.clone(),
            (Some(s), None) => format!("{}, brightness {}/{MAX_BRIGHTNESS}, speed {}/{MAX_SPEED}", s.mode, s.brightness, s.speed),
            (None, None) => "reading keyboard…".into(),
        }
    }
    fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }
}

impl ksni::Tray for OnnTray {
    fn id(&self) -> String {
        "onn-rgb".into()
    }
    fn title(&self) -> String {
        "onn Keyboard Lighting".into()
    }
    fn icon_name(&self) -> String {
        "input-keyboard".into()
    }
    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "onn Keyboard Lighting".into(),
            description: format!("{}\n{}", self.summary(), self.status),
            ..Default::default()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        open_gui();
    }
    fn menu_about_to_show(&mut self) {
        // Picks up changes made by the GUI or CLI; one read, on demand only.
        self.send(Cmd::Refresh);
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let s = self.settings;
        let enabled = s.is_some();
        let mode_idx = match s.map(|s| s.mode) {
            Some(Mode::Pulse) => 0,
            Some(Mode::Steady) => 1,
            Some(Mode::Off) => 2,
            _ => usize::MAX,
        };
        let radio = |labels: Vec<String>| -> Vec<RadioItem> {
            labels.into_iter().map(|label| RadioItem { label, enabled, ..Default::default() }).collect()
        };
        vec![
            StandardItem { label: self.summary(), enabled: false, ..Default::default() }.into(),
            StandardItem { label: self.status.clone(), enabled: false, ..Default::default() }.into(),
            MenuItem::Separator,
            SubMenu {
                label: "Mode".into(),
                submenu: vec![RadioGroup {
                    selected: mode_idx,
                    select: Box::new(|t: &mut Self, i| t.send(Cmd::SetMode(Mode::NAMED[i]))),
                    options: radio(vec!["Pulse".into(), "Steady".into(), "Off".into()]),
                }
                .into()],
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Brightness".into(),
                submenu: vec![RadioGroup {
                    selected: s.map_or(usize::MAX, |s| s.brightness as usize),
                    select: Box::new(|t: &mut Self, i| t.send(Cmd::SetBrightness(i as u8))),
                    options: radio((0..=MAX_BRIGHTNESS).map(|b| b.to_string()).collect()),
                }
                .into()],
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Speed".into(),
                submenu: vec![RadioGroup {
                    selected: s.map_or(usize::MAX, |s| s.speed as usize),
                    select: Box::new(|t: &mut Self, i| t.send(Cmd::SetSpeed(i as u8))),
                    options: radio(
                        (0..=MAX_SPEED)
                            .map(|v| match v { 0 => "0 (slowest)".into(), MAX_SPEED => format!("{v} (fastest)"), _ => v.to_string() })
                            .collect(),
                    ),
                }
                .into()],
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Block arrow keys during Fn combos".into(),
                checked: self.block_combo_keys,
                activate: Box::new(|t: &mut Self| {
                    t.block_combo_keys = !t.block_combo_keys;
                    t.ctl.set_block_combo_keys(t.block_combo_keys);
                    if let Err(e) = crate::config::save_block_combo_keys(t.block_combo_keys) {
                        eprintln!("onn-rgb: can't save config: {e}");
                    }
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Open GUI".into(),
                icon_name: "preferences-desktop-keyboard".into(),
                activate: Box::new(|_| open_gui()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Refresh".into(),
                activate: Box::new(|t: &mut Self| t.send(Cmd::Refresh)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                // Goes through the SIGTERM path so grabs are released cleanly.
                activate: Box::new(|_| unsafe {
                    libc::kill(libc::getpid(), libc::SIGTERM);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }

    /// Keep running if the tray host (e.g. panel) goes away; it may come back.
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        eprintln!("onn-rgb: tray host went away ({reason:?}); hotkeys keep working");
        true
    }
}
