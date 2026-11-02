//! `onn-rgb daemon`: tray icon + evdev hotkeys. Either part may be missing
//! (no tray host, no input permission, no keyboard) without the other failing.

use crate::actions::{self, Action};
use crate::{config, hotkeys};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Something that wants to hear about lighting changes (the tray).
type Notify = Arc<dyn Fn(Option<Action>) + Send + Sync>;

fn log(msg: impl AsRef<str>) {
    eprintln!("onn-rgb: {}", msg.as_ref());
}

/// Watch for the keyboard's event nodes and spawn one reader per node.
/// Re-scans every few seconds, so replugging the keyboard works.
fn hotkey_threads(bindings: Vec<(u16, Action)>, notify: Notify) {
    let active: Arc<Mutex<HashSet<std::path::PathBuf>>> = Default::default();
    let bindings = Arc::new(bindings);
    thread::spawn(move || {
        let mut warned = HashSet::new();
        loop {
            for (node, name) in hotkeys::event_nodes() {
                if active.lock().unwrap().contains(&node) {
                    continue;
                }
                if let Err(e) = std::fs::File::open(&node) {
                    if warned.insert(node.clone()) {
                        log(format!(
                            "hotkeys: can't read {} ({name}): {e}. Install udev/71-onn-keyboard-input.rules or join the `input` group (see README)",
                            node.display()
                        ));
                    }
                    continue;
                }
                warned.remove(&node);
                active.lock().unwrap().insert(node.clone());
                log(format!("hotkeys: listening on {} ({name})", node.display()));
                let (active, bindings, notify) = (active.clone(), bindings.clone(), notify.clone());
                thread::spawn(move || {
                    let r = hotkeys::read_node(&node, |ev| {
                        if ev.value != 1 {
                            return;
                        }
                        for (_, a) in bindings.iter().filter(|(c, _)| *c == ev.code) {
                            match actions::run(*a) {
                                Ok(s) => log(format!("{} -> {} b{} s{}", a.name(), s.mode, s.brightness, s.speed)),
                                Err(e) => log(format!("{}: {e}", a.name())),
                            }
                            notify(Some(*a));
                        }
                    });
                    log(format!("hotkeys: {} closed ({r:?})", node.display()));
                    active.lock().unwrap().remove(&node);
                });
            }
            thread::sleep(Duration::from_secs(5));
        }
    });
}

pub fn run() -> ! {
    let (cfg, warnings) = config::load();
    for w in warnings {
        log(format!("config: {w}"));
    }
    if let Some(p) = config::path() {
        log(format!("config: {}", p.display()));
    }

    let mut notify: Notify = Arc::new(|_| {});

    #[cfg(feature = "tray")]
    if cfg.tray_enabled {
        use ksni::blocking::TrayMethods;
        match crate::tray::OnnTray::new().spawn() {
            Ok(handle) => {
                log("tray: registered StatusNotifierItem");
                let h = handle.clone();
                notify = Arc::new(move |_| {
                    h.update(|t| { t.refresh(); });
                });
                if cfg.poll_seconds > 0 {
                    let secs = cfg.poll_seconds;
                    thread::spawn(move || loop {
                        thread::sleep(Duration::from_secs(secs));
                        if handle.is_closed() {
                            break;
                        }
                        handle.update(|t| {
                            t.refresh();
                        });
                    });
                }
            }
            Err(e) => log(format!(
                "tray: not available ({e}). Your desktop needs a StatusNotifierItem host \
                 (on GNOME: the AppIndicator extension). Hotkeys still work."
            )),
        }
    }
    #[cfg(not(feature = "tray"))]
    let _ = cfg.tray_enabled;

    if cfg.hotkeys_enabled {
        if cfg.bindings.is_empty() {
            log("hotkeys: no keys bound; edit the config (use `onn-rgb listen-keys` to find codes)");
        } else {
            hotkey_threads(cfg.bindings.clone(), notify);
        }
    }
    loop {
        thread::park();
    }
}
