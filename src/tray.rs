//! StatusNotifierItem tray icon (KDE, COSMIC, Cinnamon, XFCE, LXQt, Budgie,
//! GNOME with the AppIndicator extension, ...). Event-driven: it never polls;
//! the daemon's worker pushes new state after every change.
//!
//! Flat menu (no submenus):
//!   status lines / Mode label + Steady/Off/Pulse/Flash radios / Brightness label +
//!   0-3 radios / "Speed N/12" label + Faster, Slower / checkboxes / Open GUI /
//!   Quit. Left-click toggles Off <-> last mode.

use crate::actions::Action;
use crate::daemon::{Cmd, Ctl};
use crate::{Mode, Settings, MAX_BRIGHTNESS, MAX_SPEED};
use ksni::menu::*;
use ksni::{Icon, MenuItem, ToolTip};
use std::path::PathBuf;
use std::sync::mpsc::Sender;

pub struct OnnTray {
    pub settings: Option<Settings>,
    pub error: Option<String>,
    /// Hotkey/grab status line (e.g. "Fn combos active").
    pub status: String,
    pub block_combo_keys: bool,
    pub notices: bool,
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

// ---- Start at login (XDG autostart) ----

const AUTOSTART_TEMPLATE: &str = include_str!("../packaging/autostart/onn-rgb-daemon.desktop");

fn autostart_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("autostart/onn-rgb-daemon.desktop"))
}

/// Command that starts this daemon: the AppImage itself when running from
/// one (the mount point changes every run), else this binary.
fn daemon_exec() -> String {
    let exe = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "onn-rgb".into());
    let quoted = if exe.contains(' ') { format!("\"{exe}\"") } else { exe };
    format!("{quoted} daemon")
}

fn autostart_entry() -> String {
    AUTOSTART_TEMPLATE
        .lines()
        .map(|l| if l.starts_with("Exec=") { format!("Exec={}", daemon_exec()) } else { l.to_string() })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

pub fn autostart_enabled() -> bool {
    autostart_file().is_some_and(|p| p.exists())
}

/// Writes or removes only ~/.config/autostart/onn-rgb-daemon.desktop.
fn set_autostart(on: bool) -> std::io::Result<()> {
    let p = autostart_file().ok_or_else(|| std::io::Error::other("HOME not set"))?;
    if on {
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(&p, autostart_entry())?;
        eprintln!("onn-rgb: start at login on ({})", p.display());
    } else if p.exists() {
        std::fs::remove_file(&p)?;
        eprintln!("onn-rgb: start at login off (removed {})", p.display());
    }
    Ok(())
}

// ---- Icon ----

/// Bundled icon dir inside a running AppImage.
fn appimage_icons() -> Option<PathBuf> {
    let d = PathBuf::from(std::env::var_os("APPDIR")?).join("usr/share/icons");
    d.join("hicolor/scalable/apps/onn-rgb.svg").exists().then_some(d)
}

/// 64x64 ARGB32 pixmap from the bundled PNG (for hosts that ignore
/// IconThemePath).
fn pixmap() -> Vec<Icon> {
    const PNG: &[u8] = include_bytes!("../packaging/onn-rgb.png");
    let decode = || -> Option<Icon> {
        let mut r = png::Decoder::new(std::io::Cursor::new(PNG)).read_info().ok()?;
        let mut buf = vec![0; r.output_buffer_size()?];
        let info = r.next_frame(&mut buf).ok()?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return None;
        }
        let (w, h) = (info.width as usize, info.height as usize);
        let f = (w / 64).max(1);
        let (ow, oh) = (w / f, h / f);
        let mut data = Vec::with_capacity(ow * oh * 4);
        for y in 0..oh {
            for x in 0..ow {
                let mut acc = [0u32; 4];
                for dy in 0..f {
                    for dx in 0..f {
                        let i = ((y * f + dy) * w + x * f + dx) * 4;
                        for c in 0..4 {
                            acc[c] += buf[i + c] as u32;
                        }
                    }
                }
                let n = (f * f) as u32;
                let [r, g, b, a] = acc.map(|v| (v / n) as u8);
                data.extend_from_slice(&[a, r, g, b]);
            }
        }
        Some(Icon { width: ow as i32, height: oh as i32, data })
    };
    decode().into_iter().collect()
}

impl OnnTray {
    fn summary(&self) -> String {
        let s = self.lighting();
        if self.block_combo_keys { format!("{s} · keys blocked") } else { s }
    }
    fn lighting(&self) -> String {
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

fn label(text: String) -> MenuItem<OnnTray> {
    StandardItem { label: text, enabled: false, ..Default::default() }.into()
}

impl ksni::Tray for OnnTray {
    fn id(&self) -> String {
        "onn-rgb".into()
    }
    fn title(&self) -> String {
        "onn Keyboard Lighting".into()
    }
    fn icon_name(&self) -> String {
        if appimage_icons().is_some() { "onn-rgb".into() } else { "input-keyboard".into() }
    }
    fn icon_theme_path(&self) -> String {
        appimage_icons().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
    }
    fn icon_pixmap(&self) -> Vec<Icon> {
        if std::env::var_os("APPDIR").is_some() { pixmap() } else { Vec::new() }
    }
    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "onn Keyboard Lighting".into(),
            description: format!("{}\n{}\nClick: lighting on/off", self.summary(), self.status),
            ..Default::default()
        }
    }
    /// Left-click: Off <-> last non-off mode.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Cmd::Act(Action::Toggle));
    }
    fn menu_about_to_show(&mut self) {
        // Picks up changes made by the GUI or CLI; one read, on demand only.
        self.send(Cmd::Refresh);
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let s = self.settings;
        let enabled = s.is_some();
        let mode_idx = s.and_then(|s| Mode::ALL.iter().position(|m| *m == s.mode)).unwrap_or(usize::MAX);
        let radio = |labels: Vec<String>| -> Vec<RadioItem> {
            labels.into_iter().map(|label| RadioItem { label, enabled, ..Default::default() }).collect()
        };
        let speed = s.map_or("Speed".into(), |s| {
            let lim = match s.speed {
                0 => " (slowest)",
                MAX_SPEED => " (fastest)",
                _ => "",
            };
            format!("Speed {}/{MAX_SPEED}{lim}", s.speed)
        });
        let mut items: Vec<MenuItem<Self>> = vec![label(self.summary())];
        if !self.status.is_empty() {
            items.push(label(self.status.clone()));
        }
        items.extend([
            MenuItem::Separator,
            label("Mode".into()),
            RadioGroup {
                selected: mode_idx,
                select: Box::new(|t: &mut Self, i| t.send(Cmd::SetMode(Mode::ALL[i]))),
                options: radio(Mode::ALL.iter().map(|m| m.label().to_string()).collect()),
            }
            .into(),
            MenuItem::Separator,
            label("Brightness".into()),
            RadioGroup {
                selected: s.map_or(usize::MAX, |s| s.brightness as usize),
                select: Box::new(|t: &mut Self, i| t.send(Cmd::SetBrightness(i as u8))),
                options: radio((0..=MAX_BRIGHTNESS).map(|b| b.to_string()).collect()),
            }
            .into(),
            MenuItem::Separator,
            label(speed),
            StandardItem {
                label: "Faster".into(),
                enabled: enabled && s.is_some_and(|s| s.speed < MAX_SPEED),
                activate: Box::new(|t: &mut Self| t.send(Cmd::Act(Action::SpeedUp))),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Slower".into(),
                enabled: enabled && s.is_some_and(|s| s.speed > 0),
                activate: Box::new(|t: &mut Self| t.send(Cmd::Act(Action::SpeedDown))),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Show change notices".into(),
                checked: self.notices,
                activate: Box::new(|t: &mut Self| {
                    t.notices = !t.notices;
                    t.ctl.set_notices(t.notices);
                    if let Err(e) = crate::config::save_notices(t.notices) {
                        eprintln!("onn-rgb: can't save config: {e}");
                    }
                }),
                ..Default::default()
            }
            .into(),
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
            CheckmarkItem {
                label: "Start at login".into(),
                // Read from disk each time the menu is built.
                checked: autostart_enabled(),
                activate: Box::new(|_: &mut Self| {
                    if let Err(e) = set_autostart(!autostart_enabled()) {
                        eprintln!("onn-rgb: start at login: {e}");
                    }
                }),
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
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                // Goes through the SIGTERM path so grabs are released cleanly.
                activate: Box::new(|_| unsafe {
                    libc::kill(libc::getpid(), libc::SIGTERM);
                }),
                ..Default::default()
            }
            .into(),
        ]);
        items
    }

    /// Keep running if the tray host (e.g. panel) goes away; it may come back.
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        eprintln!("onn-rgb: tray host went away ({reason:?}); hotkeys keep working");
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn autostart_entry_has_absolute_exec() {
        let e = autostart_entry();
        let exec = e.lines().find(|l| l.starts_with("Exec=")).unwrap();
        assert!(exec.ends_with(" daemon") && exec.len() > "Exec=onn-rgb daemon".len() - 7);
        assert!(e.contains("X-GNOME-Autostart-enabled=true"));
    }
    #[test]
    fn pixmap_decodes() {
        let p = pixmap();
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].width, p[0].height), (64, 64));
        assert_eq!(p[0].data.len(), 64 * 64 * 4);
    }
}
