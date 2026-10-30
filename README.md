# onn-rgb

Linux lighting control for the **onn. 104-Key LED Gaming Membrane Keyboard**
(Walmart model `100122478`, USB ID `3938:1296`). The official onn software is
Windows-only; this replaces it on Linux with a small CLI and GUI written in Rust.

## Features

- Modes: **pulse**, **steady**, **off** (plus raw mode numbers for experimenting)
- Brightness: 0–3
- Effect speed: 0 (slowest) – 12 (fastest)
- No runtime dependencies: talks to the keyboard directly via `/dev/hidraw`

The keyboard's LEDs are fixed-color, so there is no color control.

## Install

```sh
# 1. Let your user access the keyboard (one time)
sudo cp udev/70-onn-keyboard.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger

# 2. Build and install (needs Rust: https://rustup.rs)
cargo install --path .

# 3. Optional: add the GUI to your app menu
cp onn-rgb.desktop ~/.local/share/applications/
```

CLI only, without the GUI dependencies: `cargo install --path . --no-default-features`

## Usage

```sh
onn-rgb status
onn-rgb mode pulse
onn-rgb brightness 3
onn-rgb speed 2
onn-rgb set --mode pulse --brightness 2 --speed 6
onn-rgb-gui
```

## Protocol notes

The keyboard uses a Sonix/EVision-style protocol (the same family OpenRGB's
EVision controller supports) on its vendor HID interface (usage page `0xFF1C`,
report ID 4, 64-byte reports):

| Byte | Meaning |
|------|---------|
| 0 | `0x04` report ID |
| 1–2 | checksum: 16-bit LE sum of bytes 3–63 |
| 3 | command: `01` begin, `02` end, `05` read config, `06` write config |
| 4 | data size |
| 5 | config offset |
| 8+ | data |

Config offsets: `01` mode, `02` brightness, `03` speed (stored inverted).
Mode values: `00` pulse, `01` steady, `02` flash (fixed timing), `0F` off.

Fn lighting shortcuts only work under Windows, where the vendor software
handles them; under Linux those keys send media keys.

## Disclaimer

Not affiliated with onn., Walmart, or KWB. Reverse-engineered by hand; use at
your own risk.
