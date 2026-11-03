//! Step-wise lighting actions shared by the CLI, hotkeys and tray.

use crate::{Keyboard, Mode, Result, Settings, MAX_BRIGHTNESS, MAX_SPEED};
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    CycleMode,
    Toggle,
    BrightnessUp,
    BrightnessDown,
    SpeedUp,
    SpeedDown,
    ModeSteady,
    ModePulse,
    ModeFlash,
    ModeOff,
}

impl Action {
    pub const ALL: [Action; 10] = [
        Action::CycleMode,
        Action::Toggle,
        Action::BrightnessUp,
        Action::BrightnessDown,
        Action::SpeedUp,
        Action::SpeedDown,
        Action::ModeSteady,
        Action::ModePulse,
        Action::ModeFlash,
        Action::ModeOff,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Action::CycleMode => "cycle-mode",
            Action::Toggle => "toggle",
            Action::BrightnessUp => "brightness-up",
            Action::BrightnessDown => "brightness-down",
            Action::SpeedUp => "speed-up",
            Action::SpeedDown => "speed-down",
            Action::ModeSteady => "mode-steady",
            Action::ModePulse => "mode-pulse",
            Action::ModeFlash => "mode-flash",
            Action::ModeOff => "mode-off",
        }
    }

    pub fn parse(s: &str) -> Option<Action> {
        if s == "mode-force" {
            return Some(Action::ModeSteady);
        }
        Action::ALL.into_iter().find(|a| a.name() == s)
    }

    /// The action that selects `m`.
    pub fn select(m: Mode) -> Action {
        match m {
            Mode::Steady => Action::ModeSteady,
            Mode::Off => Action::ModeOff,
            Mode::Pulse => Action::ModePulse,
            Mode::Flash => Action::ModeFlash,
        }
    }
}

/// Where `toggle` remembers the mode to return to.
fn last_mode_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("onn-rgb/last-mode"))
}

pub(crate) fn remember_mode(m: Mode) {
    if let Some(p) = last_mode_file() {
        let _ = fs::create_dir_all(p.parent().unwrap());
        let _ = fs::write(p, m.raw().to_string());
    }
}

fn last_mode() -> Mode {
    last_mode_file()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse().ok())
        .map(Mode::from_raw)
        .filter(|m| *m != Mode::Off)
        .unwrap_or(Mode::Steady)
}

/// Compute the settings an action leads to (pure; for tests).
pub fn next(s: Settings, a: Action, toggle_to: Mode) -> Settings {
    let mut n = s;
    match a {
        Action::CycleMode => {
            n.mode = match s.mode {
                Mode::Steady => Mode::Pulse,
                Mode::Pulse => Mode::Flash,
                Mode::Flash => Mode::Off,
                Mode::Off => Mode::Steady,
            }
        }
        Action::Toggle => n.mode = if s.mode == Mode::Off { toggle_to } else { Mode::Off },
        Action::BrightnessUp => n.brightness = (s.brightness + 1).min(MAX_BRIGHTNESS),
        Action::BrightnessDown => n.brightness = s.brightness.saturating_sub(1).min(MAX_BRIGHTNESS),
        Action::SpeedUp => n.speed = (s.speed + 1).min(MAX_SPEED),
        Action::SpeedDown => n.speed = s.speed.saturating_sub(1),
        Action::ModeSteady => n.mode = Mode::Steady,
        Action::ModePulse => n.mode = Mode::Pulse,
        Action::ModeFlash => n.mode = Mode::Flash,
        Action::ModeOff => n.mode = Mode::Off,
    }
    n.speed = n.speed.min(n.mode.max_speed());
    n
}

/// Read the keyboard, apply the action, return the new settings.
pub fn apply(kb: &mut Keyboard, a: Action) -> Result<Settings> {
    let s = kb.settings()?;
    apply_from(kb, s, a)
}

/// Apply an action given known current settings: writes only what changes
/// (one HID write per action, none if already at a limit).
pub fn apply_from(kb: &mut Keyboard, s: Settings, a: Action) -> Result<Settings> {
    let n = next(s, a, last_mode());
    if n.mode != s.mode {
        if s.mode != Mode::Off {
            remember_mode(s.mode);
        }
        kb.set_mode(n.mode)?;
    }
    if n.brightness != s.brightness {
        kb.set_brightness(n.brightness)?;
    }
    if n.speed != s.speed {
        kb.set_speed(n.speed)?;
    }
    Ok(n)
}

/// Open the keyboard, apply one action, close it again.
pub fn run(a: Action) -> Result<Settings> {
    apply(&mut Keyboard::open()?, a)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn st(mode: Mode, brightness: u8, speed: u8) -> Settings {
        Settings { mode, brightness, speed }
    }

    #[test]
    fn steps_clamp_and_cycle() {
        let s = st(Mode::Steady, 3, 12);
        assert_eq!(next(s, Action::BrightnessUp, Mode::Steady).brightness, 3);
        assert_eq!(next(s, Action::BrightnessDown, Mode::Steady).brightness, 2);
        assert_eq!(next(s, Action::SpeedUp, Mode::Steady).speed, 12);
        assert_eq!(next(st(Mode::Steady, 0, 0), Action::SpeedDown, Mode::Steady).speed, 0);
        assert_eq!(next(s, Action::CycleMode, Mode::Steady).mode, Mode::Pulse);
        assert_eq!(next(st(Mode::Pulse, 3, 4), Action::CycleMode, Mode::Steady).mode, Mode::Flash);
        assert_eq!(next(st(Mode::Flash, 3, 4), Action::CycleMode, Mode::Steady).mode, Mode::Off);
        assert_eq!(next(st(Mode::Off, 3, 4), Action::CycleMode, Mode::Steady).mode, Mode::Steady);
        assert_eq!(next(st(Mode::Pulse, 3, 4), Action::SpeedUp, Mode::Steady).speed, 5);
        assert_eq!(next(st(Mode::Pulse, 3, 9), Action::SpeedUp, Mode::Steady).speed, 9);
        assert_eq!(next(st(Mode::Flash, 3, 3), Action::SpeedUp, Mode::Steady).speed, 3);
        assert_eq!(next(st(Mode::Pulse, 3, 8), Action::ModeFlash, Mode::Steady).speed, 3);
        assert_eq!(next(st(Mode::Pulse, 3, 4), Action::ModeFlash, Mode::Steady).mode, Mode::Flash);
        assert_eq!(next(s, Action::Toggle, Mode::Steady).mode, Mode::Off);
        assert_eq!(next(st(Mode::Off, 3, 4), Action::Toggle, Mode::Pulse).mode, Mode::Pulse);
    }

    #[test]
    fn names_round_trip() {
        for a in Action::ALL {
            assert_eq!(Action::parse(a.name()), Some(a));
        }
    }
}
