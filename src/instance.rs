//! Coordination between the tray daemon and the GUI window, all through
//! files in $XDG_RUNTIME_DIR (no polling):
//! - `onn-rgb.lock`: held by the running daemon (see daemon::lock).
//! - `onn-rgb.tray`: exists while the daemon has a tray icon registered.
//! - `onn-rgb-gui.lock` + `onn-rgb-gui.sock`: single GUI instance; a second
//!   launch sends "show", the tray's Quit sends "quit".

use std::fs::File;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

pub fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

pub fn daemon_lock_path() -> PathBuf { runtime_dir().join("onn-rgb.lock") }
pub fn tray_marker() -> PathBuf { runtime_dir().join("onn-rgb.tray") }
pub fn gui_lock_path() -> PathBuf { runtime_dir().join("onn-rgb-gui.lock") }
pub fn gui_socket() -> PathBuf { runtime_dir().join("onn-rgb-gui.sock") }

/// Try to take an exclusive lock without blocking. Some(file) = we hold it.
pub fn try_lock(p: &PathBuf) -> Option<File> {
    let f = File::create(p).ok()?;
    (unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(f)
}

/// True while some daemon holds its single-instance lock.
pub fn daemon_running() -> bool {
    match try_lock(&daemon_lock_path()) {
        Some(f) => {
            drop(f); // released immediately
            false
        }
        None => true,
    }
}

/// True if a running daemon has a tray icon registered.
pub fn tray_present() -> bool {
    daemon_running() && tray_marker().exists()
}

/// Send a one-line command to the running GUI. Returns false if none listens.
pub fn send_gui(cmd: &str) -> bool {
    match UnixStream::connect(gui_socket()) {
        Ok(mut s) => s.write_all(cmd.as_bytes()).is_ok(),
        Err(_) => false,
    }
}
