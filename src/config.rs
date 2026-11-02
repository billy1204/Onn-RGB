//! `~/.config/onn-rgb/config.toml`: hotkey bindings for `onn-rgb daemon`.

use crate::actions::Action;
use crate::hotkeys::key_code;
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
# Seconds between re-reading the keyboard so the tray follows changes made
# by the keyboard's own Fn keys, the GUI or the CLI. 0 disables polling.
poll-seconds = 3
"#;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub hotkeys_enabled: bool,
    pub bindings: Vec<(u16, Action)>,
    pub tray_enabled: bool,
    pub poll_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config { hotkeys_enabled: true, bindings: Vec::new(), tray_enabled: true, poll_seconds: 3 }
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
        if let Some(n) = t.get("poll-seconds").and_then(|v| v.as_integer()) {
            c.poll_seconds = n.max(0) as u64;
        }
    }
    Ok((c, warn))
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
        let (c, w) = parse(TEMPLATE).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c, Config::default());
    }

    #[test]
    fn bindings_by_name_and_number() {
        let (c, w) = parse("[hotkeys]\ncycle-mode = [\"KEY_F13\", 191]\ntoggle = \"KEY_SCROLLLOCK\"\nbogus = []\n").unwrap();
        assert_eq!(c.bindings, vec![(183, Action::CycleMode), (191, Action::CycleMode), (70, Action::Toggle)]);
        assert_eq!(w.len(), 1);
    }
}
