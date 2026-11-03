//! Read key presses from the keyboard's evdev nodes (`/dev/input/event*`).
//!
//! The nodes are opened read-only and never grabbed (no `EVIOCGRAB`), so the
//! desktop and other apps keep receiving every key. This works the same on
//! X11 and Wayland. Access needs the udev rule in `udev/` (uaccess) or
//! membership in the `input` group.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::PathBuf;

const VENDOR: &str = "3938";
const PRODUCT: &str = "1296";
const EV_KEY: u16 = 0x01;

/// Common evdev key names (linux/input-event-codes.h).
pub const NAMES: &[(&str, u16)] = &[
    ("KEY_1", 2), ("KEY_2", 3), ("KEY_3", 4), ("KEY_4", 5), ("KEY_5", 6),
    ("KEY_6", 7), ("KEY_7", 8), ("KEY_8", 9), ("KEY_9", 10), ("KEY_0", 11),
    ("KEY_Q", 16), ("KEY_W", 17), ("KEY_E", 18), ("KEY_R", 19), ("KEY_T", 20),
    ("KEY_Y", 21), ("KEY_U", 22), ("KEY_I", 23), ("KEY_O", 24), ("KEY_P", 25),
    ("KEY_A", 30), ("KEY_S", 31), ("KEY_D", 32), ("KEY_F", 33), ("KEY_G", 34),
    ("KEY_H", 35), ("KEY_J", 36), ("KEY_K", 37), ("KEY_L", 38),
    ("KEY_Z", 44), ("KEY_X", 45), ("KEY_C", 46), ("KEY_V", 47), ("KEY_B", 48),
    ("KEY_N", 49), ("KEY_M", 50),
    ("KEY_ESC", 1), ("KEY_TAB", 15), ("KEY_CAPSLOCK", 58),
    ("KEY_F1", 59), ("KEY_F2", 60), ("KEY_F3", 61), ("KEY_F4", 62), ("KEY_F5", 63),
    ("KEY_F6", 64), ("KEY_F7", 65), ("KEY_F8", 66), ("KEY_F9", 67), ("KEY_F10", 68),
    ("KEY_NUMLOCK", 69), ("KEY_SCROLLLOCK", 70), ("KEY_F11", 87), ("KEY_F12", 88),
    ("KEY_HOME", 102), ("KEY_UP", 103), ("KEY_PAGEUP", 104), ("KEY_LEFT", 105),
    ("KEY_RIGHT", 106), ("KEY_END", 107), ("KEY_DOWN", 108), ("KEY_PAGEDOWN", 109),
    ("KEY_INSERT", 110), ("KEY_DELETE", 111),
    ("KEY_MUTE", 113), ("KEY_VOLUMEDOWN", 114), ("KEY_VOLUMEUP", 115), ("KEY_POWER", 116),
    ("KEY_PAUSE", 119), ("KEY_LEFTMETA", 125), ("KEY_RIGHTMETA", 126), ("KEY_COMPOSE", 127),
    ("KEY_STOP", 128), ("KEY_CALC", 140), ("KEY_SLEEP", 142), ("KEY_WAKEUP", 143),
    ("KEY_MAIL", 155), ("KEY_BOOKMARKS", 156), ("KEY_COMPUTER", 157), ("KEY_BACK", 158),
    ("KEY_FORWARD", 159), ("KEY_NEXTSONG", 163), ("KEY_PLAYPAUSE", 164),
    ("KEY_PREVIOUSSONG", 165), ("KEY_STOPCD", 166), ("KEY_RECORD", 167), ("KEY_REWIND", 168),
    ("KEY_CONFIG", 171), ("KEY_HOMEPAGE", 172), ("KEY_REFRESH", 173),
    ("KEY_F13", 183), ("KEY_F14", 184), ("KEY_F15", 185), ("KEY_F16", 186), ("KEY_F17", 187),
    ("KEY_F18", 188), ("KEY_F19", 189), ("KEY_F20", 190), ("KEY_F21", 191), ("KEY_F22", 192),
    ("KEY_F23", 193), ("KEY_F24", 194), ("KEY_PLAY", 207), ("KEY_FASTFORWARD", 208),
    ("KEY_PRINT", 210), ("KEY_SEARCH", 217), ("KEY_MEDIA", 226),
    ("KEY_BRIGHTNESSDOWN", 224), ("KEY_BRIGHTNESSUP", 225),
    ("KEY_KBDILLUMTOGGLE", 228), ("KEY_KBDILLUMDOWN", 229), ("KEY_KBDILLUMUP", 230),
    ("KEY_FILE", 144), ("KEY_WWW", 150), ("KEY_MICMUTE", 248),
];

pub fn key_code(name: &str) -> Option<u16> {
    let n = name.trim().to_ascii_uppercase();
    let n = if n.starts_with("KEY_") { n } else { format!("KEY_{n}") };
    NAMES.iter().find(|(k, _)| *k == n).map(|(_, c)| *c)
}

pub fn key_name(code: u16) -> String {
    NAMES.iter().find(|(_, c)| *c == code).map(|(n, _)| n.to_string()).unwrap_or_else(|| format!("code {code}"))
}

/// Event nodes belonging to the keyboard, with their device names.
pub fn event_nodes() -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for e in fs::read_dir("/sys/class/input").into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("event") {
            continue;
        }
        let id = e.path().join("device/id");
        let rd = |f: &str| fs::read_to_string(id.join(f)).unwrap_or_default().trim().to_string();
        if rd("vendor") == VENDOR && rd("product") == PRODUCT {
            let dev = fs::read_to_string(e.path().join("device/name")).unwrap_or_default();
            out.push((PathBuf::from("/dev/input").join(&name), dev.trim().to_string()));
        }
    }
    out.sort();
    out
}

/// One key press/release/repeat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub code: u16,
    /// 1 = press, 0 = release, 2 = autorepeat.
    pub value: i32,
}

/// Decode a `struct input_event` (64-bit: 16-byte timeval, u16 type, u16 code, i32 value).
pub fn decode(buf: &[u8; 24]) -> Option<KeyEvent> {
    let ty = u16::from_ne_bytes([buf[16], buf[17]]);
    if ty != EV_KEY {
        return None;
    }
    Some(KeyEvent {
        code: u16::from_ne_bytes([buf[18], buf[19]]),
        value: i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]),
    })
}

/// Block reading key events from one node, calling `f` for each.
/// Returns when the device disappears or an error occurs.
pub fn read_node(path: &PathBuf, mut f: impl FnMut(KeyEvent)) -> io::Result<()> {
    let mut file = File::open(path)?;
    let mut buf = [0u8; 24];
    loop {
        file.read_exact(&mut buf)?;
        if let Some(ev) = decode(&buf) {
            f(ev);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(key_code("KEY_MUTE"), Some(113));
        assert_eq!(key_code("f13"), Some(183));
        assert_eq!(key_name(164), "KEY_PLAYPAUSE");
        assert_eq!(key_name(999), "code 999");
    }

    #[test]
    fn decodes_key_event() {
        let mut b = [0u8; 24];
        b[16..18].copy_from_slice(&1u16.to_ne_bytes());
        b[18..20].copy_from_slice(&113u16.to_ne_bytes());
        b[20..24].copy_from_slice(&1i32.to_ne_bytes());
        assert_eq!(decode(&b), Some(KeyEvent { code: 113, value: 1 }));
        b[16] = 4; // EV_MSC
        assert_eq!(decode(&b), None);
    }
}
