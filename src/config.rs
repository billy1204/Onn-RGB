//! `~/.config/onn-rgb/config.toml`: hotkey bindings for `onn-rgb daemon`.

use crate::actions::Action;
use crate::fnkeys::{self, FnConfig, DEFAULT_MODE_KEYS};
use crate::Mode;
use crate::hotkeys::{key_code, key_name};
use std::time::Duration;
use std::fs;
use std::path::PathBuf;

pub const TEMPLATE: &str = r#"# onn-rgb configuration (read by `onn-rgb daemon`).
#
# Hotkeys are read straight from the keyboard's evdev nodes, so they work on
# X11 and Wayland under any desktop. By default the Fn node (event5) is
# grabbed and re-emitted through the "onn-rgb passthrough" uinput device minus
# the Fn key itself; the main typing node (event4) is grabbed only with
# block-combo-keys. Other apps still receive every other key.
#
# Run `onn-rgb listen-keys`, press a key (e.g. an Fn lighting combo), and
# copy the name or number it prints into a list below. Leave a list empty
# to disable that action. Example:  cycle-mode = ["KEY_F13", 191]

[hotkeys]
enabled = true
cycle-mode = []
toggle = []
brightness-up = []
brightness-down = []
speed-up = []
speed-down = []

[tray]
enabled = true
# Desktop notice ("Brightness 3/3 (max)") after Fn-combo and tray changes.
notices = true

# Fn combos. The keyboard reports Fn as KEY_VOLUMEUP on its second event node;
# a press arms Fn, which lasts until its release (if release-disarms) or
# window-ms without a press/repeat. If Fn+Up does nothing, try
# release-disarms = false.
[fn]
key = "KEY_VOLUMEUP"
window-ms = 400
release-disarms = true
# Grab the Fn node and re-emit everything except Fn through a uinput device
# ("onn-rgb passthrough"), so Fn doesn't change the volume. Needs
# udev/72-onn-uinput.rules; falls back to ungrabbed if uinput isn't usable.
grab = true
# Also grab the main typing node so the arrow etc. of a Fn combo don't reach
# apps. (Tray: "Block arrow keys during Fn combos".)
block-combo-keys = false

# Fn+Up / Fn+Right = speed faster, Fn+Down / Fn+Left = slower (fixed; in
# Pulse mode this is the pulse speed). Fn+<key> below selects a mode. Use a
# key name from `onn-rgb listen-keys` (or a number); "" = unbound. Arrow keys
# and duplicates are rejected. Also editable in the GUI ("Fn mode keys").
# Letter keys come from the main node, so set block-combo-keys = true to stop
# Fn+P etc. from also typing the letter. Restart the daemon after changes.
[fn.modes]
steady = "KEY_S"
off = "KEY_O"
pulse = "KEY_P"
flash = "KEY_F"
"#;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub hotkeys_enabled: bool,
    pub bindings: Vec<(u16, Action)>,
    pub tray_enabled: bool,
    pub notices: bool,
    pub fn_cfg: FnConfig,
    /// Fn mode keys as configured (before conflict filtering).
    pub mode_keys: Vec<(Mode, u16)>,
    pub grab: bool,
    pub block_combo_keys: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkeys_enabled: true,
            bindings: Vec::new(),
            tray_enabled: true,
            notices: true,
            fn_cfg: FnConfig::default(),
            mode_keys: DEFAULT_MODE_KEYS.to_vec(),
            grab: true,
            block_combo_keys: false,
        }
    }
}

pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("onn-rgb/config.toml"))
}

/// Parse a config file. Unknown keys and bad entries are reported as warnings.
pub fn parse(text: &str) -> Result<(Config, Vec<String>), String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let mut c = Config::default();
    let mut warn = Vec::new();
    if let Some(h) = table.get("hotkeys").and_then(|v| v.as_table()) {
        for (k, v) in h {
            if k == "enabled" {
                c.hotkeys_enabled = v.as_bool().unwrap_or(true);
                continue;
            }
            let Some(action) = Action::parse(k) else {
                warn.push(format!("unknown hotkey action `{k}`"));
                continue;
            };
            for item in v.as_array().cloned().unwrap_or_else(|| vec![v.clone()]) {
                let code = match &item {
                    toml::Value::Integer(n) => u16::try_from(*n).ok(),
                    toml::Value::String(s) => key_code(s),
                    _ => None,
                };
                match code {
                    Some(code) => c.bindings.push((code, action)),
                    None => warn.push(format!("{k}: can't use key {item:?}")),
                }
            }
        }
    }
    if let Some(t) = table.get("tray").and_then(|v| v.as_table()) {
        if let Some(b) = t.get("enabled").and_then(|v| v.as_bool()) {
            c.tray_enabled = b;
        }
        if let Some(b) = t.get("notices").and_then(|v| v.as_bool()) {
            c.notices = b;
        }
    }
    if let Some(f) = table.get("fn").and_then(|v| v.as_table()) {
        for (k, v) in f {
            match (k.as_str(), v) {
                ("key", toml::Value::String(s)) => match key_code(s) {
                    Some(code) => c.fn_cfg.key = code,
                    None => warn.push(format!("fn.key: unknown key {s}")),
                },
                ("key", toml::Value::Integer(n)) => c.fn_cfg.key = *n as u16,
                ("window-ms", toml::Value::Integer(n)) => c.fn_cfg.window = Duration::from_millis((*n).clamp(50, 5000) as u64),
                ("release-disarms", toml::Value::Boolean(b)) => c.fn_cfg.release_disarms = *b,
                ("grab", toml::Value::Boolean(b)) => c.grab = *b,
                ("block-combo-keys" | "block_combo_keys", toml::Value::Boolean(b)) => c.block_combo_keys = *b,
                ("keys", _) => warn.push("fn.keys is no longer used (Fn+arrows = speed); set mode keys in [fn.modes]".into()),
                ("modes", toml::Value::Table(t)) => {
                    let mut keys = DEFAULT_MODE_KEYS.to_vec();
                    for (mn, kv) in t {
                        let Some(m) = Mode::parse(mn) else {
                            warn.push(format!("fn.modes: unknown mode `{mn}` (steady, off, pulse, flash)"));
                            continue;
                        };
                        let code = match kv {
                            toml::Value::String(s) if s.trim().is_empty() => {
                                keys.retain(|(o, _)| *o != m);
                                continue;
                            }
                            toml::Value::String(s) => key_code(s),
                            toml::Value::Integer(n) => u16::try_from(*n).ok(),
                            _ => None,
                        };
                        match code {
                            Some(k) => {
                                keys.retain(|(o, _)| *o != m);
                                keys.push((m, k));
                            }
                            None => warn.push(format!("fn.modes: can't use {mn} = {kv:?}")),
                        }
                    }
                    keys.sort_by_key(|(m, _)| Mode::ALL.iter().position(|o| o == m));
                    c.mode_keys = keys;
                }
                _ => warn.push(format!("fn: unknown or bad setting `{k}`")),
            }
        }
    }
    let (map, w) = fnkeys::build_map(&c.mode_keys);
    c.fn_cfg.map = map;
    warn.extend(w.into_iter().map(|w| format!("fn.modes: {w}")));
    Ok((c, warn))
}

/// Check and save Fn mode keys (from the GUI) as the `[fn.modes]` section.
pub fn save_mode_keys(keys: &[(Mode, u16)]) -> Result<(), String> {
    for &(m, k) in keys {
        if let Some(e) = fnkeys::mode_key_conflict(keys, m, k) {
            return Err(e);
        }
    }
    let p = path().ok_or("HOME not set")?;
    let text = fs::read_to_string(&p).unwrap_or_else(|_| TEMPLATE.to_string());
    let s = set_modes(&text, keys);
    if let Some(d) = p.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    fs::write(&p, s).map_err(|e| format!("{}: {e}", p.display()))
}

/// Replace (or append) the `[fn.modes]` section, keeping everything else.
fn set_modes(text: &str, keys: &[(Mode, u16)]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut skip = false;
    let mut at = None;
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            skip = t == "[fn.modes]";
            if skip && at.is_none() {
                at = Some(out.len());
            }
        }
        if !skip {
            out.push(l.to_string());
        }
    }
    let mut sec = vec!["[fn.modes]".to_string()];
    for m in Mode::ALL {
        let v = keys.iter().find(|(o, _)| *o == m).map(|(_, k)| key_name(*k)).unwrap_or_default();
        sec.push(format!("{} = \"{v}\"", m.name()));
    }
    match at {
        Some(i) => {
            out.splice(i..i, sec);
        }
        None => {
            out.push(String::new());
            out.extend(sec);
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Persist `block-combo-keys` (from the tray) with a minimal text edit.
pub fn save_block_combo_keys(on: bool) -> std::io::Result<()> {
    save_bool("fn", &["block-combo-keys", "block_combo_keys"], on)
}

/// Persist `[tray] notices` (from the tray).
pub fn save_notices(on: bool) -> std::io::Result<()> {
    save_bool("tray", &["notices"], on)
}

fn save_bool(section: &str, keys: &[&str], on: bool) -> std::io::Result<()> {
    let p = path().ok_or_else(|| std::io::Error::other("HOME not set"))?;
    let text = fs::read_to_string(&p).unwrap_or_else(|_| TEMPLATE.to_string());
    let s = set_bool(&text, section, keys, on);
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)?;
    }
    fs::write(p, s)
}

/// Set `keys[0] = on` inside `[section]`, replacing an existing line.
fn set_bool(text: &str, section: &str, keys: &[&str], on: bool) -> String {
    let line = format!("{} = {on}", keys[0]);
    let header = format!("[{section}]");
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut done = false;
    let mut header_at = None;
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            cur = t.to_string();
            if t == header {
                header_at = Some(out.len());
            }
        }
        let k = t.split('=').next().unwrap_or("").trim();
        if !done && cur == header && keys.contains(&k) {
            out.push(line.clone());
            done = true;
        } else {
            out.push(l.to_string());
        }
    }
    if !done {
        match header_at {
            Some(i) => out.insert(i + 1, line),
            None => {
                out.push(String::new());
                out.push(header);
                out.push(line);
            }
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
#[test]
fn set_bool_edits() {
    let t = "[tray]\nenabled = true\n[fn]\nkey = \"KEY_VOLUMEUP\"\n";
    let s = set_bool(t, "tray", &["notices"], false);
    assert_eq!(s, "[tray]\nnotices = false\nenabled = true\n[fn]\nkey = \"KEY_VOLUMEUP\"\n");
    assert_eq!(set_bool(&s, "tray", &["notices"], true), s.replace("false", "true"));
    let s = set_bool(t, "fn", &["block-combo-keys", "block_combo_keys"], true);
    assert!(s.contains("[fn]\nblock-combo-keys = true\n"));
    let s2 = set_bool("[hotkeys]\n", "tray", &["notices"], false);
    assert_eq!(s2, "[hotkeys]\n\n[tray]\nnotices = false\n");
    assert!(!parse(&set_bool(TEMPLATE, "tray", &["notices"], false)).unwrap().0.notices);
}

/// Human-readable Fn map, for logs.
pub fn describe_fn(c: &FnConfig) -> String {
    c.map.iter().map(|(k, a)| format!("{}={}", key_name(*k).trim_start_matches("KEY_"), a.name())).collect::<Vec<_>>().join(" ")
}

/// Load the config, writing the commented template first if none exists.
pub fn load() -> (Config, Vec<String>) {
    let Some(p) = path() else { return (Config::default(), vec!["HOME not set".into()]) };
    if !p.exists() {
        let _ = fs::create_dir_all(p.parent().unwrap());
        let _ = fs::write(&p, TEMPLATE);
    }
    match fs::read_to_string(&p) {
        Ok(t) => parse(&t).unwrap_or_else(|e| {
            (Config::default(), vec![format!("{}: {e}; using defaults", p.display())])
        }),
        Err(e) => (Config::default(), vec![format!("{}: {e}", p.display())]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses() {
        let (mut c, w) = parse(TEMPLATE).unwrap();
        assert!(w.is_empty(), "{w:?}");
        let mut d = Config::default();
        c.fn_cfg.map.sort_by_key(|x| x.0);
        d.fn_cfg.map.sort_by_key(|x| x.0);
        assert_eq!(c, d);
    }

    #[test]
    fn bindings_by_name_and_number() {
        let (c, w) = parse("[hotkeys]\ncycle-mode = [\"KEY_F13\", 191]\ntoggle = \"KEY_SCROLLLOCK\"\nbogus = []\n").unwrap();
        assert_eq!(c.bindings, vec![(183, Action::CycleMode), (191, Action::CycleMode), (70, Action::Toggle)]);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn fn_section() {
        let (c, w) = parse("[fn]\nwindow-ms = 400\nrelease-disarms = false\nblock-combo-keys = true\n[fn.modes]\nflash = \"KEY_F1\"\noff = \"\"\npulse = \"KEY_UP\"\nrainbow = \"KEY_A\"\n").unwrap();
        assert_eq!(w.len(), 2, "{w:?}"); // unknown mode, arrow conflict
        assert_eq!(c.fn_cfg.window, Duration::from_millis(400));
        assert!(!c.fn_cfg.release_disarms && c.block_combo_keys && c.grab);
        assert!(c.fn_cfg.map.contains(&(59, Action::ModeFlash)));
        assert!(c.fn_cfg.map.contains(&(31, Action::ModeSteady)));
        assert!(!c.fn_cfg.map.iter().any(|(_, a)| *a == Action::ModeOff || *a == Action::ModePulse));
        assert!(c.fn_cfg.map.contains(&(103, Action::SpeedUp)));
    }

    #[test]
    fn old_fn_keys_ignored() {
        let (c, w) = parse("[fn.keys]\nKEY_UP = \"brightness-up\"\n").unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(c.fn_cfg, FnConfig::default());
    }

    #[test]
    fn set_modes_round_trip() {
        let keys = vec![(Mode::Steady, 59), (Mode::Pulse, 25)];
        let s = set_modes(TEMPLATE, &keys);
        assert_eq!(s.matches("[fn.modes]").count(), 1);
        let (c, w) = parse(&s).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.mode_keys.len(), 2);
        assert!(c.fn_cfg.map.contains(&(59, Action::ModeSteady)));
        let s2 = set_modes("[tray]\nnotices = true\n", &keys);
        assert!(s2.starts_with("[tray]\nnotices = true\n\n[fn.modes]\nsteady = \"KEY_F1\"\n"));
        assert!(save_mode_keys(&[(Mode::Off, 105)]).is_err());
    }
}
