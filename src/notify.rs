//! Change notices ("Brightness 3/3 (max)") via org.freedesktop.Notifications.
//! Each notice replaces the previous one (replaces_id), so holding a combo
//! doesn't stack popups.

use crate::{Mode, Settings, MAX_BRIGHTNESS};

/// Which setting a command was about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Mode,
    Brightness,
    Speed,
}

/// Notice text for the setting `kind` after a change (or a press at a limit).
pub fn text(kind: Kind, s: Settings) -> String {
    match kind {
        Kind::Mode => match s.mode {
            Mode::Off => "Lighting off".into(),
            m => format!("Mode: {}", m.label()),
        },
        Kind::Brightness => {
            let lim = match s.brightness {
                0 => " (min)",
                MAX_BRIGHTNESS => " (max)",
                _ => "",
            };
            format!("Brightness {}/{MAX_BRIGHTNESS}{lim}", s.brightness)
        }
        Kind::Speed => {
            let lim = match s.speed {
                0 => " (slowest)",
                x if x >= s.mode.max_speed() => " (fastest)",
                _ => "",
            };
            format!("Speed {}/{}{lim}", s.speed, s.mode.max_speed())
        }
    }
}

#[cfg(feature = "tray")]
pub use dbus::Notifier;

#[cfg(feature = "tray")]
mod dbus {
    use std::collections::HashMap;
    use zbus::zvariant::Value;

    /// Lives on the HID worker thread; connects lazily on first notice.
    pub struct Notifier {
        rt: Option<tokio::runtime::Runtime>,
        conn: Option<zbus::Connection>,
        last_id: u32,
        failed: bool,
    }

    impl Default for Notifier {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Notifier {
        pub fn new() -> Self {
            Notifier { rt: None, conn: None, last_id: 0, failed: false }
        }

        pub fn show(&mut self, body: &str) {
            if self.failed {
                return;
            }
            if let Err(e) = self.try_show(body) {
                eprintln!("onn-rgb: notices: {e}");
                self.conn = None;
                self.last_id = 0;
            }
        }

        fn try_show(&mut self, body: &str) -> Result<(), String> {
            if self.rt.is_none() {
                match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => self.rt = Some(rt),
                    Err(e) => {
                        self.failed = true;
                        return Err(e.to_string());
                    }
                }
            }
            let rt = self.rt.as_ref().unwrap();
            let conn = match &self.conn {
                Some(c) => c.clone(),
                None => {
                    let c = rt.block_on(zbus::Connection::session()).map_err(|e| e.to_string())?;
                    self.conn = Some(c.clone());
                    c
                }
            };
            let mut hints: HashMap<&str, Value> = HashMap::new();
            hints.insert("transient", Value::from(true));
            hints.insert("urgency", Value::from(0u8));
            let actions: Vec<&str> = Vec::new();
            let id = self.last_id;
            let reply = rt
                .block_on(conn.call_method(
                    Some("org.freedesktop.Notifications"),
                    "/org/freedesktop/Notifications",
                    Some("org.freedesktop.Notifications"),
                    "Notify",
                    &("onn Keyboard Lighting", id, "input-keyboard", "Keyboard lighting", body, actions, hints, 2000i32),
                ))
                .map_err(|e| e.to_string())?;
            self.last_id = reply.body().deserialize::<u32>().map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn st(m: Mode, b: u8, v: u8) -> Settings {
        Settings { mode: m, brightness: b, speed: v }
    }
    #[test]
    fn texts() {
        assert_eq!(text(Kind::Brightness, st(Mode::Steady, 3, 4)), "Brightness 3/3 (max)");
        assert_eq!(text(Kind::Brightness, st(Mode::Steady, 0, 4)), "Brightness 0/3 (min)");
        assert_eq!(text(Kind::Brightness, st(Mode::Steady, 2, 4)), "Brightness 2/3");
        assert_eq!(text(Kind::Speed, st(Mode::Steady, 3, 12)), "Speed 12/12 (fastest)");
        assert_eq!(text(Kind::Speed, st(Mode::Steady, 3, 0)), "Speed 0/12 (slowest)");
        assert_eq!(text(Kind::Mode, st(Mode::Off, 3, 4)), "Lighting off");
        assert_eq!(text(Kind::Mode, st(Mode::Pulse, 3, 4)), "Mode: Pulse");
    }
}
