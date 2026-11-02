//! `~/.config/onn-rgb/config.toml`: hotkey bindings for `onn-rgb daemon`.

use crate::actions::Action;
use crate::fnkeys::FnConfig;
use crate::hotkeys::{key_code, key_name};
use std::time::Duration;
use std::fs;
use std::path::PathBuf;

pub const TEMPLATE: &str = r#"# onn-rgb configuration (read by `onn-rgb daemon`).
#
# Hotkeys are read straight from the keyboard's evdev nodes, so they work on
# X11 and Wayland under any desktop. Keys are only observed, never grabbed:
# other apps still receive them.
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

# Key pressed while Fn is armed -> action. Replaces the defaults if present.
[fn.keys]
KEY_UP = "brightness-up"
KEY_DOWN = "brightness-down"
KEY_RIGHT = "speed-up"
KEY_LEFT = "speed-down"
KEY_INSERT = "cycle-mode"
KEY_HOME = "mode-steady"
KEY_PAGEUP = "mode-pulse"
KEY_DELETE = "toggle"
KEY_END = "toggle"
KEY_PAGEDOWN = "cycle-mode"
"#;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub hotkeys_enabled: bool,
    pub bindings: Vec<(u16, Action)>,
    pub tray_enabled: bool,
    pub fn_cfg: FnConfig,
    pub grab: bool,
    pub block_combo_keys: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            hotkeys_enabled: true,
            bindings: Vec::new(),
            tray_enabled: true,
            fn_cfg: FnConfig::default(),
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
                ("keys", toml::Value::Table(keys)) => {
                    c.fn_cfg.map.clear();
                    for (kn, an) in keys {
                        let code = kn.parse::<u16>().ok().or_else(|| key_code(kn));
                        let act = an.as_str().and_then(Action::parse);
                        match (code, act) {
                            (Some(code), Some(a)) => c.fn_cfg.map.push((code, a)),
                            _ => warn.push(format!("fn.keys: can't use {kn} = {an:?}")),
                        }
                    }
                }
                _ => warn.push(format!("fn: unknown or bad setting `{k}`")),
            }
        }
    }
    Ok((c, warn))
}

/// Persist `block-combo-keys` (from the tray) with a minimal text edit.
pub fn save_block_combo_keys(on: bool) -> std::io::Result<()> {
    let p = path().ok_or_else(|| std::io::Error::other("HOME not set"))?;
    let text = fs::read_to_string(&p).unwrap_or_else(|_| TEMPLATE.to_string());
    let line = format!("block-combo-keys = {on}");
    let mut out: Vec<String> = Vec::new();
    let mut done = false;
    for l in text.lines() {
        let t = l.trim_start();
        if !done && (t.starts_with("block-combo-keys") || t.starts_with("block_combo_keys")) {
            out.push(line.clone());
            done = true;
        } else {
            out.push(l.to_string());
            if !done && t == "[fn]" {
                out.push(line.clone());
                done = true;
            }
        }
    }
    if !done {
        out.push(String::new());
        out.push("[fn]".into());
        out.push(line);
    }
    let mut s = out.join("\n");
    s.push('\n');
    fs::write(p, s)
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
        let (c, w) = parse("[fn]\nwindow-ms = 250\nrelease-disarms = false\nblock-combo-keys = true\n[fn.keys]\nKEY_F1 = \"toggle\"\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.fn_cfg.window, Duration::from_millis(250));
        assert!(!c.fn_cfg.release_disarms && c.block_combo_keys && c.grab);
        assert_eq!(c.fn_cfg.map, vec![(59, Action::Toggle)]);
    }
}
