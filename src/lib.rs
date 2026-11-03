//! Lighting control for the onn 104-key LED Gaming Keyboard (USB `3938:1296`).
//!
//! The keyboard speaks a Sonix/EVision-style protocol: 64-byte HID reports
//! (report ID 4) on its vendor interface (usage page `0xFF1C`). Every
//! transaction is wrapped in BEGIN/END packets, and each packet carries a
//! 16-bit little-endian checksum of bytes 3..64.

pub mod actions;
pub mod config;
pub mod daemon;
pub mod evdev;
pub mod fnkeys;
pub mod hotkeys;
pub mod notify;
#[cfg(feature = "tray")]
pub mod tray;

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

const HID_ID: &str = "00003938:00001296";
const VENDOR_PAGE: [u8; 3] = [0x06, 0x1c, 0xff];
const REPORT_ID: u8 = 0x04;
/// `O_NONBLOCK` on Linux x86 and ARM, so reads can honor the reply timeout.
const O_NONBLOCK: i32 = 0o4000;

const CMD_BEGIN: u8 = 0x01;
const CMD_END: u8 = 0x02;
const CMD_READ: u8 = 0x05;
const CMD_WRITE: u8 = 0x06;

const OFF_MODE: u8 = 0x01;
const OFF_BRIGHTNESS: u8 = 0x02;
const OFF_SPEED: u8 = 0x03;

/// Highest brightness level the hardware supports (values wrap above this).
pub const MAX_BRIGHTNESS: u8 = 3;
/// Highest user-facing speed (0 = slowest). The firmware stores it inverted.
pub const MAX_SPEED: u8 = 12;
/// The keyboard's four lighting modes. Raw values: `00` pulse, `01` steady,
/// `02` flash, `0F` off. Any other value read back is shown as Steady.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    Steady,
    Off,
    Pulse,
    Flash,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Steady, Mode::Off, Mode::Pulse, Mode::Flash];

    /// Unknown raw values (old firmware/experiments) fall back to Steady.
    pub fn from_raw(v: u8) -> Mode {
        match v {
            0 => Mode::Pulse,
            2 => Mode::Flash,
            15 => Mode::Off,
            _ => Mode::Steady,
        }
    }
    pub fn raw(self) -> u8 {
        match self {
            Mode::Pulse => 0,
            Mode::Steady => 1,
            Mode::Flash => 2,
            Mode::Off => 15,
        }
    }
    /// Parse a mode name; "force" is an alias for steady.
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "steady" | "force" => Some(Mode::Steady),
            "off" => Some(Mode::Off),
            "pulse" => Some(Mode::Pulse),
            "flash" => Some(Mode::Flash),
            _ => None,
        }
    }
    /// Lowercase config/CLI name.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Steady => "steady",
            Mode::Off => "off",
            Mode::Pulse => "pulse",
            Mode::Flash => "flash",
        }
    }
    /// Capitalized label for menus.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Steady => "Steady",
            Mode::Off => "Off",
            Mode::Pulse => "Pulse",
            Mode::Flash => "Flash",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Current lighting settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub mode: Mode,
    pub brightness: u8,
    pub speed: u8,
}

#[derive(Debug)]
pub enum Error {
    NotFound,
    Io(String, io::Error),
    NoResponse,
    OutOfRange(&'static str, u8, u8),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::NotFound => write!(f, "onn keyboard (3938:1296) not found"),
            Error::Io(p, e) if e.kind() == io::ErrorKind::PermissionDenied => write!(
                f,
                "permission denied opening {p}; install udev/70-onn-keyboard.rules (see README)"
            ),
            Error::Io(p, e) => write!(f, "{p}: {e}"),
            Error::NoResponse => write!(f, "keyboard did not respond"),
            Error::OutOfRange(what, v, max) => write!(f, "{what} must be 0-{max}, got {v}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn find_hidraw() -> Option<String> {
    for entry in fs::read_dir("/sys/class/hidraw").ok()?.flatten() {
        let dev = entry.path().join("device");
        let uevent = fs::read_to_string(dev.join("uevent")).unwrap_or_default();
        if !uevent.to_ascii_uppercase().contains(HID_ID) {
            continue;
        }
        let desc = fs::read(dev.join("report_descriptor")).unwrap_or_default();
        if desc.windows(3).any(|w| w == VENDOR_PAGE) {
            return Some(format!("/dev/{}", entry.file_name().to_string_lossy()));
        }
    }
    None
}

fn packet(cmd: u8, size: u8, offset: u8, data: &[u8]) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = REPORT_ID;
    b[3] = cmd;
    b[4] = size;
    b[5] = offset;
    b[8..8 + data.len()].copy_from_slice(data);
    let sum: u32 = b[3..].iter().map(|&x| x as u32).sum();
    b[1] = sum as u8;
    b[2] = (sum >> 8) as u8;
    b
}

/// This interface also carries key and mouse reports, so check the report ID too.
fn is_reply(r: &[u8], cmd: u8) -> bool {
    r.len() > 3 && r[0] == REPORT_ID && r[3] == cmd
}

/// An open connection to the keyboard.
pub struct Keyboard {
    file: File,
    path: String,
}

impl Keyboard {
    /// Find and open the keyboard's vendor HID interface.
    pub fn open() -> Result<Keyboard> {
        let path = find_hidraw().ok_or(Error::NotFound)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open(&path)
            .map_err(|e| Error::Io(path.clone(), e))?;
        Ok(Keyboard { file, path })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    fn xfer(&mut self, p: [u8; 64]) -> Result<Vec<u8>> {
        self.file
            .write_all(&p)
            .map_err(|e| Error::Io(self.path.clone(), e))?;
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut buf = [0u8; 64];
        loop {
            match self.file.read(&mut buf) {
                Ok(n) if is_reply(&buf[..n], p[3]) => return Ok(buf[..n].to_vec()),
                Ok(_) => continue, // other report; read again
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(Error::Io(self.path.clone(), e)),
            }
            // Sleep in poll() until a report arrives or the deadline passes.
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let mut pfd = libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let ms = left.as_millis().clamp(1, i32::MAX as u128) as i32;
            let r = unsafe { libc::poll(&mut pfd, 1, ms) };
            if r < 0 {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::Interrupted {
                    return Err(Error::Io(self.path.clone(), e));
                }
            } else if r == 0 {
                break;
            }
        }
        Err(Error::NoResponse)
    }

    fn transaction(&mut self, p: [u8; 64]) -> Result<Vec<u8>> {
        self.xfer(packet(CMD_BEGIN, 0, 0, &[]))?;
        let r = self.xfer(p);
        self.xfer(packet(CMD_END, 0, 0, &[]))?;
        r
    }

    fn write_byte(&mut self, offset: u8, value: u8) -> Result<()> {
        self.transaction(packet(CMD_WRITE, 1, offset, &[value]))
            .map(|_| ())
    }

    /// Read the current settings.
    pub fn settings(&mut self) -> Result<Settings> {
        let r = self.transaction(packet(CMD_READ, 0x26, 0, &[]))?;
        if r.len() < 12 {
            return Err(Error::NoResponse);
        }
        Ok(Settings {
            mode: Mode::from_raw(r[9]),
            brightness: r[10],
            speed: MAX_SPEED.saturating_sub(r[11]),
        })
    }

    pub fn set_mode(&mut self, mode: Mode) -> Result<()> {
        self.write_byte(OFF_MODE, mode.raw())
    }

    pub fn set_brightness(&mut self, level: u8) -> Result<()> {
        if level > MAX_BRIGHTNESS {
            return Err(Error::OutOfRange("brightness", level, MAX_BRIGHTNESS));
        }
        self.write_byte(OFF_BRIGHTNESS, level)
    }

    /// Set effect speed, 0 (slowest) to [`MAX_SPEED`] (fastest).
    pub fn set_speed(&mut self, speed: u8) -> Result<()> {
        if speed > MAX_SPEED {
            return Err(Error::OutOfRange("speed", speed, MAX_SPEED));
        }
        self.write_byte(OFF_SPEED, MAX_SPEED - speed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_matches_captured_packets() {
        assert_eq!(&packet(CMD_BEGIN, 0, 0, &[])[..4], &[0x04, 0x01, 0x00, 0x01]);
        assert_eq!(&packet(CMD_READ, 0x26, 0, &[])[..6], &[0x04, 0x2b, 0x00, 0x05, 0x26, 0x00]);
        assert_eq!(&packet(CMD_WRITE, 1, 2, &[4])[..9], &[0x04, 0x0d, 0x00, 0x06, 0x01, 0x02, 0, 0, 0x04]);
    }

    #[test]
    fn reply_needs_matching_report_id_and_command() {
        assert!(is_reply(&packet(CMD_READ, 0x26, 0, &[]), CMD_READ));
        assert!(!is_reply(&packet(CMD_READ, 0x26, 0, &[]), CMD_WRITE));
        assert!(!is_reply(&[0x01, 0, 0, CMD_READ, 0], CMD_READ));
        assert!(!is_reply(&[REPORT_ID, 0, 0], CMD_READ));
    }

    #[test]
    fn mode_round_trip() {
        for m in Mode::ALL {
            assert_eq!(Mode::from_raw(m.raw()), m);
            assert_eq!(Mode::parse(m.name()), Some(m));
        }
        assert_eq!(Mode::parse("force"), Some(Mode::Steady));
        assert_eq!(Mode::from_raw(7), Mode::Steady);
        assert_eq!(Mode::parse("7"), None);
        assert_eq!(Mode::parse("rainbow"), None);
    }
}
