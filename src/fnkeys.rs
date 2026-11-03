//! Fn-combo state machine.
//!
//! The onn keyboard reports its Fn key as `KEY_VOLUMEUP` on its second
//! (consumer/system) event node. A press there *arms* Fn; Fn is disarmed by
//! that key's release (if `release_disarms`) or when the window expires
//! without a press or autorepeat refreshing it. A mapped key pressed while
//! armed triggers an action and is *consumed*: its press, repeats and
//! release are not forwarded (relevant only when that node is grabbed).

use crate::actions::Action;
use crate::Mode;
use std::collections::HashSet;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The node that carries the Fn key (event5 on pop-os).
    FnNode,
    /// The main typing node (event4).
    Main,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FnConfig {
    pub key: u16,
    pub window: Duration,
    pub release_disarms: bool,
    pub map: Vec<(u16, Action)>,
}

/// Fn+arrow keys: fixed to speed (Up/Right faster, Down/Left slower).
pub const SPEED_KEYS: [(u16, Action); 4] = [
    (103, Action::SpeedUp),   // KEY_UP
    (106, Action::SpeedUp),   // KEY_RIGHT
    (108, Action::SpeedDown), // KEY_DOWN
    (105, Action::SpeedDown), // KEY_LEFT
];

/// Default Fn mode keys: S = steady, O = off, P = pulse, F = flash.
pub const DEFAULT_MODE_KEYS: [(Mode, u16); 4] = [
    (Mode::Steady, 31), // KEY_S
    (Mode::Off, 24),    // KEY_O
    (Mode::Pulse, 25),  // KEY_P
    (Mode::Flash, 33),  // KEY_F
];

/// Build the Fn map from mode keys (speed arrows are always included).
/// Arrow keys and duplicates are rejected; returns the map and warnings.
pub fn build_map(modes: &[(Mode, u16)]) -> (Vec<(u16, Action)>, Vec<String>) {
    let mut map: Vec<(u16, Action)> = SPEED_KEYS.to_vec();
    let mut warn = Vec::new();
    for &(m, k) in modes {
        if let Some(err) = mode_key_conflict(modes, m, k) {
            warn.push(err);
            continue;
        }
        map.push((k, Action::select(m)));
    }
    (map, warn)
}

/// Why `k` can't be the Fn key for mode `m`, if it can't.
pub fn mode_key_conflict(modes: &[(Mode, u16)], m: Mode, k: u16) -> Option<String> {
    let name = crate::hotkeys::key_name(k);
    if SPEED_KEYS.iter().any(|(c, _)| *c == k) {
        return Some(format!("{m}: {name} is reserved for Fn speed control"));
    }
    if let Some((o, _)) = modes.iter().find(|(o, c)| *c == k && *o != m) {
        return Some(format!("{m}: {name} is already used by {o}"));
    }
    None
}

impl Default for FnConfig {
    fn default() -> Self {
        FnConfig {
            key: 115, // KEY_VOLUMEUP
            window: Duration::from_millis(400),
            release_disarms: true,
            map: build_map(&DEFAULT_MODE_KEYS).0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Out {
    /// Pass the event on (when grabbed) / let it through.
    pub forward: bool,
    pub action: Option<Action>,
}

pub struct Engine {
    pub cfg: FnConfig,
    /// Plain (non-Fn) bindings from `[hotkeys]`; observed, never consumed.
    pub plain: Vec<(u16, Action)>,
    armed_until: Option<Instant>,
    consumed: HashSet<u16>,
}

impl Engine {
    pub fn new(cfg: FnConfig, plain: Vec<(u16, Action)>) -> Engine {
        Engine { cfg, plain, armed_until: None, consumed: HashSet::new() }
    }

    /// When the daemon must wake up to expire the Fn window, if ever.
    pub fn deadline(&self) -> Option<Instant> {
        self.armed_until
    }

    pub fn is_armed(&self, now: Instant) -> bool {
        self.armed_until.is_some_and(|d| now < d)
    }

    pub fn expire(&mut self, now: Instant) {
        if !self.is_armed(now) {
            self.armed_until = None;
        }
    }

    pub fn key(&mut self, origin: Origin, code: u16, value: i32, now: Instant) -> Out {
        if origin == Origin::FnNode && code == self.cfg.key {
            match value {
                0 if self.cfg.release_disarms => self.armed_until = None,
                0 => {}
                _ => self.armed_until = Some(now + self.cfg.window),
            }
            return Out { forward: false, action: None };
        }
        self.expire(now);
        if value == 1 && self.is_armed(now) {
            if let Some((_, a)) = self.cfg.map.iter().find(|(c, _)| *c == code) {
                self.consumed.insert(code);
                // Keep Fn armed for chained presses (Fn+Up, Up, Up).
                self.armed_until = Some(now + self.cfg.window);
                return Out { forward: false, action: Some(*a) };
            }
        }
        if self.consumed.contains(&code) {
            if value == 0 {
                self.consumed.remove(&code);
            }
            return Out { forward: false, action: None };
        }
        let action = if value == 1 { self.plain.iter().find(|(c, _)| *c == code).map(|(_, a)| *a) } else { None };
        Out { forward: true, action }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::*;

    fn eng() -> Engine {
        Engine::new(FnConfig::default(), vec![(183, Toggle)])
    }
    const UP: u16 = 103;
    const FN: u16 = 115;

    #[test]
    fn fn_then_arrow_acts_and_consumes() {
        let mut e = eng();
        let t = Instant::now();
        assert_eq!(e.key(Origin::FnNode, FN, 1, t), Out { forward: false, action: None });
        assert_eq!(e.key(Origin::Main, UP, 1, t), Out { forward: false, action: Some(SpeedUp) });
        assert_eq!(e.key(Origin::Main, UP, 2, t), Out { forward: false, action: None }); // repeat ignored
        assert_eq!(e.key(Origin::FnNode, FN, 0, t), Out { forward: false, action: None });
        assert_eq!(e.key(Origin::Main, UP, 0, t), Out { forward: false, action: None }); // consumed release
        assert_eq!(e.key(Origin::Main, UP, 1, t), Out { forward: true, action: None }); // plain arrow
    }

    #[test]
    fn release_disarms() {
        let mut e = eng();
        let t = Instant::now();
        e.key(Origin::FnNode, FN, 1, t);
        e.key(Origin::FnNode, FN, 0, t);
        assert_eq!(e.key(Origin::Main, UP, 1, t).action, None);
    }

    #[test]
    fn window_expires_without_release() {
        let mut e = eng();
        let t = Instant::now();
        e.key(Origin::FnNode, FN, 1, t);
        assert_eq!(e.deadline(), Some(t + Duration::from_millis(400)));
        assert_eq!(e.key(Origin::Main, 106, 1, t + Duration::from_millis(399)).action, Some(SpeedUp));
        // the combo refreshed the window; let it lapse
        e.expire(t + Duration::from_millis(900));
        assert_eq!(e.deadline(), None);
        assert_eq!(e.key(Origin::Main, 105, 1, t + Duration::from_millis(900)).action, None);
    }

    #[test]
    fn autorepeat_of_fn_keeps_it_armed() {
        let mut e = eng();
        let t = Instant::now();
        e.key(Origin::FnNode, FN, 1, t);
        e.key(Origin::FnNode, FN, 2, t + Duration::from_millis(350));
        assert_eq!(e.key(Origin::Main, 25, 1, t + Duration::from_millis(700)).action, Some(ModePulse));
    }

    #[test]
    fn release_ignored_when_configured() {
        let mut e = Engine::new(FnConfig { release_disarms: false, ..FnConfig::default() }, vec![]);
        let t = Instant::now();
        e.key(Origin::FnNode, FN, 1, t);
        e.key(Origin::FnNode, FN, 0, t);
        assert_eq!(e.key(Origin::Main, 31, 1, t).action, Some(ModeSteady));
    }

    #[test]
    fn volumeup_on_main_node_is_not_fn_and_unmapped_keys_pass() {
        let mut e = eng();
        let t = Instant::now();
        assert_eq!(e.key(Origin::Main, FN, 1, t), Out { forward: true, action: None });
        e.key(Origin::FnNode, FN, 1, t);
        assert_eq!(e.key(Origin::Main, 30, 1, t), Out { forward: true, action: None }); // KEY_A
        assert_eq!(e.key(Origin::FnNode, 164, 1, t), Out { forward: true, action: None }); // play/pause
    }

    #[test]
    fn default_map_is_speed_and_modes_only() {
        let m = FnConfig::default().map;
        assert_eq!(m.len(), 8);
        assert!(m.contains(&(103, SpeedUp)) && m.contains(&(105, SpeedDown)));
        assert!(m.contains(&(33, ModeFlash)) && m.contains(&(24, ModeOff)));
        let (m2, w) = build_map(&[(Mode::Pulse, 103), (Mode::Off, 24), (Mode::Flash, 24)]);
        assert_eq!(w.len(), 3, "{w:?}");
        assert_eq!(m2.len(), 4);
    }

    #[test]
    fn plain_bindings_fire_but_forward() {
        let mut e = eng();
        assert_eq!(e.key(Origin::Main, 183, 1, Instant::now()), Out { forward: true, action: Some(Toggle) });
    }
}
