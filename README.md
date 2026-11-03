# onn-rgb

Linux lighting control for the **onn. 104-Key LED Gaming Membrane Keyboard**
(Walmart model `100122478`, USB ID `3938:1296`). The official onn software is
Windows-only; this replaces it on Linux with a CLI, a GUI, a tray icon and a
background daemon that handles the Fn lighting keys, written in Rust.

## Features

- Modes: **steady**, **pulse**, **flash**, **off**
- Brightness: 0–3
- Effect speed: 0 (slowest) – 12 (fastest); applies to pulse
- **Fn keys on Linux**: the daemon listens for Fn combos and sends the change to the keyboard
  - Fn+Up / Fn+Right: faster
  - Fn+Down / Fn+Left: slower
  - Fn+S steady, Fn+P pulse, Fn+F flash, Fn+O off (reassignable)
- **Tray icon**: mode, brightness, speed, open GUI, quit. Closing the GUI window keeps
  the app running in the tray; choose **Quit** from the tray to exit.
- GUI section to reassign the Fn mode keys
- Desktop notices on changes, start at login, hotplug support
- Portable **AppImage** (built against glibc 2.28, runs on most distros)
- Talks to the keyboard directly via `/dev/hidraw`; no runtime services needed

The keyboard's LEDs are fixed-color, so there is no color control.

## Install

### AppImage (easiest)

Download `onn_Keyboard_Lighting-x86_64.AppImage` from Releases, then:

```sh
chmod +x onn_Keyboard_Lighting-x86_64.AppImage
./onn_Keyboard_Lighting-x86_64.AppImage
```

### Permissions (one time, needed for either install)

```sh
sudo cp udev/70-onn-keyboard.rules udev/71-onn-keyboard-input.rules udev/72-onn-uinput.rules /etc/udev/rules.d/
sudo cp modules-load/onn-rgb.conf /etc/modules-load.d/
sudo modprobe uinput
sudo udevadm control --reload && sudo udevadm trigger
```

Log out and back in (or replug the keyboard) afterward.

- `70` lets your user talk to the keyboard's lighting interface.
- `71` lets the daemon read the Fn key.
- `72` lets the daemon pass keys through, so Fn+letter doesn't also type the letter
  (turn on "Block arrow keys during Fn combos" in the tray).

### From source

```sh
# needs Rust: https://rustup.rs
cargo install --path .
scripts/install-user.sh        # desktop entry, icon, autostart
```

CLI only, without GUI/tray dependencies: `cargo install --path . --no-default-features`

Build the AppImage yourself: `scripts/build-appimage.sh` (output in `dist/`).

## Usage

```sh
onn-rgb status
onn-rgb mode pulse            # steady | pulse | flash | off
onn-rgb brightness 3
onn-rgb speed 6
onn-rgb set --mode pulse --brightness 2 --speed 6
onn-rgb cycle-mode            # steady -> pulse -> flash -> off
onn-rgb toggle                # off / last mode
onn-rgb mode-steady | mode-off | mode-pulse | mode-flash
onn-rgb brightness up|down
onn-rgb speed up|down
onn-rgb daemon                # tray + Fn keys (started automatically by the GUI)
onn-rgb listen-keys           # show raw key events, for debugging
onn-rgb config                # print config path
onn-rgb-gui
```

The CLI commands can also be bound as custom shortcuts in your desktop's keyboard settings.

## Configuration

`~/.config/onn-rgb/config.toml`:

```toml
[fn]
window-ms = 400            # how long after Fn a combo key counts
block-combo-keys = false   # swallow the combo key so it isn't typed (needs rule 72)

[fn.modes]                 # key name, key number, or "" to unbind
steady = "KEY_S"
off    = "KEY_O"
pulse  = "KEY_P"
flash  = "KEY_F"
```

Restart the daemon (tray Quit, then reopen) after editing.

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
| 5–6 | config offset |
| 7 | status |
| 8+ | data |

Config offsets: `01` mode, `02` brightness, `03` speed (stored inverted).
Mode values: `00` pulse, `01` steady, `02` flash (fixed timing), `0F` off.

The keyboard firmware does not act on Fn lighting combos itself. Under Windows the
vendor software handles them; under Linux the Fn key arrives as `KEY_VOLUMEUP`
on a separate input node, and the onn-rgb daemon turns Fn+key into lighting commands.

## Disclaimer

Not affiliated with onn., Walmart, or KWB. Reverse-engineered by hand; use at
your own risk.
