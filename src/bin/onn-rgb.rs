use onn_rgb::{Keyboard, Mode, MAX_BRIGHTNESS, MAX_MODE, MAX_SPEED};
use std::process::exit;

fn usage() -> ! {
    eprintln!(
        "onn-rgb: lighting control for the onn 104-key LED Gaming Keyboard

usage:
  onn-rgb status
  onn-rgb mode <pulse|steady|off|0-{MAX_MODE}>
  onn-rgb brightness <0-{MAX_BRIGHTNESS}>
  onn-rgb speed <0-{MAX_SPEED}>          (0 = slowest)
  onn-rgb set [--mode M] [--brightness N] [--speed N]"
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
