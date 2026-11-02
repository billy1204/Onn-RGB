//! StatusNotifierItem tray icon (KDE, COSMIC, Cinnamon, XFCE, LXQt, Budgie,
//! GNOME with the AppIndicator extension, ...).

use crate::actions::{self, Action};
use crate::{Keyboard, Mode, Settings, MAX_BRIGHTNESS, MAX_SPEED};
use ksni::menu::*;
use ksni::{MenuItem, ToolTip};

pub struct OnnTray {
    pub settings: Option<Settings>,
    pub error: Option<String>,
}

impl OnnTray {
    pub fn new() -> Self {
        let mut t = OnnTray { settings: None, error: None };
        t.refresh();
        t
    }

    /// Re-read the keyboard. Returns true if anything visible changed.
    pub fn refresh(&mut self) -> bool {
        let (s, e) = match Keyboard::open().and_then(|mut kb| kb.settings()) {
            Ok(s) => (Some(s), None),
            Err(e) => (self.settings, Some(e.to_string())),
        };
        let changed = s != self.settings || e != self.error;
        self.settings = s;
        self.error = e;
        changed
    }

    pub fn action(&mut self, a: Action) {
        match actions::run(a) {
            Ok(s) => {
                self.settings = Some(s);
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn with_kb(&mut self, f: impl FnOnce(&mut Keyboard) -> crate::Result<()>) {
        let r = Keyboard::open().and_then(|mut kb| f(&mut kb));
        if let Err(e) = r {
            self.error = Some(e.to_string());
        }
        self.refresh();
    }

    fn summary(&self) -> String {
        match (&self.settings, &self.error) {
            (_, Some(e)) => e.clone(),
            (Some(s), None) => format!("{}, brightness {}/{MAX_BRIGHTNESS}, speed {}/{MAX_SPEED}", s.mode, s.brightness, s.speed),
            (None, None) => "keyboard not read yet".into(),
        }
    }
}

impl Default for OnnTray {
    fn default() -> Self {
        Self::new()
    }
}

pub fn open_gui() {
    let sibling = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("onn-rgb-gui")));
    let prog = sibling.filter(|p| p.exists()).map(|p| p.into_os_string()).unwrap_or_else(|| "onn-rgb-gui".into());
    if let Err(e) = std::process::Command::new(&prog).spawn() {
        eprintln!("onn-rgb: can't start {}: {e}", prog.to_string_lossy());
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
        ToolTip { title: "onn Keyboard Lighting".into(), description: self.summary(), ..Default::default() }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        open_gui();
    }
    fn menu_about_to_show(&mut self) {
        self.refresh();
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
            MenuItem::Separator,
            SubMenu {
                label: "Mode".into(),
                submenu: vec![RadioGroup {
                    selected: mode_idx,
                    select: Box::new(|t: &mut Self, i| {
                        let m = Mode::NAMED[i];
                        t.with_kb(|kb| kb.set_mode(m));
                    }),
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
                    select: Box::new(|t: &mut Self, i| t.with_kb(|kb| kb.set_brightness(i as u8))),
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
                    select: Box::new(|t: &mut Self, i| t.with_kb(|kb| kb.set_speed(i as u8))),
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
            StandardItem {
                label: "Open GUI".into(),
                icon_name: "preferences-desktop-keyboard".into(),
                activate: Box::new(|_| open_gui()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Refresh".into(),
                activate: Box::new(|t: &mut Self| {
                    t.refresh();
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| std::process::exit(0)),
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
