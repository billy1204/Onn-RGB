//! Fn-combo state machine.
//!
//! The onn keyboard reports its Fn key as `KEY_VOLUMEUP` on its second
//! (consumer/system) event node. A press there *arms* Fn; Fn is disarmed by
//! that key's release (if `release_disarms`) or when the window expires
//! without a press or autorepeat refreshing it. A mapped key pressed while
//! armed triggers an action and is *consumed*: its press, repeats and
//! release are not forwarded (relevant only when that node is grabbed).

use crate::actions::Action;
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

impl Default for FnConfig {
    fn default() -> Self {
        use Action::*;
        FnConfig {
            key: 115, // KEY_VOLUMEUP
            window: Duration::from_millis(400),
            release_disarms: true,
            map: vec![
                (103, BrightnessUp),   // KEY_UP
                (108, BrightnessDown), // KEY_DOWN
                (106, SpeedUp),        // KEY_RIGHT
                (105, SpeedDown),      // KEY_LEFT
                (110, CycleMode),      // KEY_INSERT
                (102, ModeSteady),     // KEY_HOME
                (104, ModePulse),      // KEY_PAGEUP
                (111, Toggle),         // KEY_DELETE
                (107, Toggle),         // KEY_END
                (109, CycleMode),      // KEY_PAGEDOWN
            ],
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
        assert_eq!(e.key(Origin::Main, UP, 1, t), Out { forward: false, action: Some(BrightnessUp) });
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
        assert_eq!(e.key(Origin::Main, 110, 1, t + Duration::from_millis(700)).action, Some(CycleMode));
    }

    #[test]
    fn release_ignored_when_configured() {
        let mut e = Engine::new(FnConfig { release_disarms: false, ..FnConfig::default() }, vec![]);
        let t = Instant::now();
        e.key(Origin::FnNode, FN, 1, t);
        e.key(Origin::FnNode, FN, 0, t);
        assert_eq!(e.key(Origin::Main, 102, 1, t).action, Some(ModeSteady));
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
    fn plain_bindings_fire_but_forward() {
        let mut e = eng();
        assert_eq!(e.key(Origin::Main, 183, 1, Instant::now()), Out { forward: true, action: Some(Toggle) });
    }
}
