use onn_rgb::actions::{self, Action};
use onn_rgb::{config, hotkeys};
use onn_rgb::{Keyboard, Mode, MAX_BRIGHTNESS, MAX_SPEED};
use std::process::exit;

fn usage() -> ! {
    eprintln!(
        "onn-rgb: lighting control for the onn 104-key LED Gaming Keyboard

usage:
  onn-rgb status
  onn-rgb mode <steady|off|pulse|flash>   (\"force\" = steady)
  onn-rgb brightness <0-{MAX_BRIGHTNESS}>
  onn-rgb speed <0-{MAX_SPEED}>          (0 = slowest)
  onn-rgb set [--mode M] [--brightness N] [--speed N]

step actions (bind these in your desktop's keyboard-shortcut settings):
  onn-rgb cycle-mode                    (steady -> pulse -> flash -> off)
  onn-rgb toggle                        (off <-> last mode)
  onn-rgb mode-steady | mode-off | mode-pulse | mode-flash
  onn-rgb brightness <up|down>
  onn-rgb speed <up|down>

hotkeys and tray:
  onn-rgb daemon                        (tray icon + evdev hotkeys)
  onn-rgb listen-keys                   (print key codes from the keyboard)
  onn-rgb config                        (print config path; creates it)"
    );
    exit(2)
}

fn num(s: Option<&String>) -> u8 {
    s.and_then(|v| v.parse().ok()).unwrap_or_else(|| usage())
}
fn mode(s: Option<&String>) -> Mode {
    s.and_then(|v| Mode::parse(v)).unwrap_or_else(|| usage())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("-h" | "--help" | "help")) {
        usage();
    }
    let step = match (args.first().map(String::as_str), args.get(1).map(String::as_str)) {
        (Some("cycle-mode"), _) => Some(Action::CycleMode),
        (Some("toggle"), _) => Some(Action::Toggle),
        (Some("mode-steady"), _) => Some(Action::ModeSteady),
        (Some("mode-force"), _) => Some(Action::ModeSteady),
        (Some("mode-pulse"), _) => Some(Action::ModePulse),
        (Some("mode-flash"), _) => Some(Action::ModeFlash),
        (Some("mode-off"), _) => Some(Action::ModeOff),
        (Some("brightness"), Some("up")) => Some(Action::BrightnessUp),
        (Some("brightness"), Some("down")) => Some(Action::BrightnessDown),
        (Some("speed"), Some("up")) => Some(Action::SpeedUp),
        (Some("speed"), Some("down")) => Some(Action::SpeedDown),
        (Some("daemon"), _) => onn_rgb::daemon::run(),
        (Some("listen-keys"), _) => listen_keys(),
        (Some("config"), _) => {
            let (_, warnings) = config::load();
            println!("{}", config::path().map(|p| p.display().to_string()).unwrap_or_default());
            for w in warnings {
                eprintln!("warning: {w}");
            }
            exit(0)
        }
        _ => None,
    };
    if let Some(a) = step {
        match actions::run(a) {
            Ok(s) => {
                println!("mode:       {}\nbrightness: {}\nspeed:      {}", s.mode, s.brightness, s.speed);
                exit(0)
            }
            Err(e) => {
                eprintln!("error: {e}");
                exit(1)
            }
        }
    }
    let mut kb = Keyboard::open().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        exit(1)
    });
    let result = match args.first().map(String::as_str) {
        None | Some("status") => kb.settings().map(|s| {
            println!("mode:       {}\nbrightness: {}\nspeed:      {}", s.mode, s.brightness, s.speed)
        }),
        Some("mode") => kb.set_mode(mode(args.get(1))),
        Some("brightness") => kb.set_brightness(num(args.get(1))),
        Some("speed") => kb.set_speed(num(args.get(1))),
        Some("set") => {
            let mut r = Ok(());
            for pair in args[1..].chunks(2) {
                r = match pair[0].as_str() {
                    "--mode" => kb.set_mode(mode(pair.get(1))),
                    "--brightness" => kb.set_brightness(num(pair.get(1))),
                    "--speed" => kb.set_speed(num(pair.get(1))),
                    _ => usage(),
                };
                if r.is_err() { break; }
            }
            r
        }
        _ => usage(),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        exit(1);
    }
}

fn listen_keys() -> ! {
    let nodes = hotkeys::event_nodes();
    if nodes.is_empty() {
        eprintln!("error: no input event nodes for the onn keyboard (3938:1296) found");
        exit(1);
    }
    let mut handles = Vec::new();
    for (node, name) in nodes {
        if let Err(e) = std::fs::File::open(&node) {
            eprintln!("{}: {name}: {e} (see README: udev/71-onn-keyboard-input.rules or the input group)", node.display());
            continue;
        }
        eprintln!("listening on {} ({name})", node.display());
        handles.push(std::thread::spawn(move || {
            let n = node.display().to_string();
            let _ = hotkeys::read_node(&node, |ev| {
                let what = match ev.value { 1 => "press", 0 => "release", 2 => "repeat", _ => "?" };
                println!("{n}: {} = {} value {} ({what})", hotkeys::key_name(ev.code), ev.code, ev.value);
            });
        }));
    }
    if handles.is_empty() {
        exit(1);
    }
    eprintln!("press keys (Ctrl+C to stop); put the name or number into ~/.config/onn-rgb/config.toml");
    for h in handles {
        let _ = h.join();
    }
    exit(0)
}
