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
# optional, for daemon hotkeys / listen-keys:
sudo cp udev/71-onn-keyboard-input.rules /etc/udev/rules.d/
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

## AppImage

```sh
./scripts/build-appimage.sh   # -> dist/onn_Keyboard_Lighting-x86_64.AppImage
```

Uses `appimagetool` cached in `~/.cache/kraken-rgb-tools/` (downloaded there
if missing; no root) and falls back to `--appimage-extract-and-run` without FUSE.

- No arguments: opens the GUI. `--gui` forces the GUI.
- Any other arguments go to the CLI: `./onn_Keyboard_Lighting-x86_64.AppImage status`, `… mode pulse`, `… --help`.
- **The udev rule is not installed by the AppImage**; install it separately (one time):

```sh
./onn_Keyboard_Lighting-x86_64.AppImage udev-rule | sudo tee /etc/udev/rules.d/70-onn-keyboard.rules
sudo udevadm control --reload && sudo udevadm trigger
```

## Tray icon and hotkeys (`onn-rgb daemon`)

`onn-rgb daemon` runs a tray icon plus a hotkey listener. Each part works on
its own: if there's no tray host, hotkeys and the CLI still work; if hotkeys
can't read the keyboard, the tray still works.

### Tray

A StatusNotifierItem (via the pure-Rust `ksni` crate, no libdbus needed) with
Mode / Brightness / Speed submenus, Open GUI, Refresh and Quit. Left-click
opens the GUI. It re-reads the keyboard every `poll-seconds` (default 3) and
when the menu opens, so it follows changes from the keyboard's own Fn keys,
the GUI or the CLI.

| Desktop | Tray |
|---|---|
| KDE Plasma, COSMIC, Cinnamon, XFCE (with the Status Tray plugin), LXQt, Budgie, MATE | built in |
| GNOME | needs the **AppIndicator and KStatusNotifierItem Support** extension (preinstalled on Ubuntu as "Ubuntu AppIndicators") |
| No tray host (sway without waybar tray, etc.) | daemon logs "tray: not available" and keeps running hotkeys |

### Hotkeys (evdev, works on X11 and Wayland)

The daemon reads the keyboard's `/dev/input/event*` nodes directly
(read-only, never grabbed, so your desktop and apps still get every key).
This works the same on any desktop or compositor. Bindings live in
`~/.config/onn-rgb/config.toml` (created on first run; `onn-rgb config`
prints the path). Nothing is bound by default, because the keyboard's Fn
lighting combos haven't been captured yet:

```sh
onn-rgb listen-keys          # press keys; prints e.g. "/dev/input/event5: KEY_F13 = 183"
```

Then add the name or number to an action in the config, e.g.
`cycle-mode = ["KEY_F13"]`, and restart the daemon. Actions: `cycle-mode`,
`toggle`, `brightness-up`, `brightness-down`, `speed-up`, `speed-down`.
If the Fn lighting combos print nothing, the keyboard handles them
internally; the tray still follows those changes via polling.

Reading input nodes needs permission:

- **systemd or elogind distros** (Fedora, Ubuntu/Pop!_OS, Debian, Arch,
  openSUSE, Gentoo/Void/Alpine with elogind): install
  `udev/71-onn-keyboard-input.rules` (`uaccess` for the seat user):
  `sudo cp udev/71-onn-keyboard-input.rules /etc/udev/rules.d/ && sudo udevadm control --reload && sudo udevadm trigger`
- **No logind** (e.g. Void/Alpine/Artix without elogind): `uaccess` does
  nothing. Install the rules anyway (they also set `GROUP="input"`), then
  `sudo usermod -aG input $USER` and log out and back in. The same applies
  to the hidraw rule (`70-onn-keyboard.rules`) for lighting control.

Either way the user can read every key typed on this keyboard; skip the
input rule if you don't want hotkeys.

### Desktop shortcut fallback

Instead of (or in addition to) evdev hotkeys, bind these commands in your
desktop's keyboard-shortcut settings (COSMIC: Settings > Keyboard >
Keyboard shortcuts > Custom; GNOME: Settings > Keyboard > Custom Shortcuts;
KDE: System Settings > Shortcuts > Custom; XFCE/Cinnamon: Keyboard >
Shortcuts). Each prints the new state:

```sh
onn-rgb cycle-mode          # pulse -> steady -> off -> pulse
onn-rgb toggle              # off <-> last mode
onn-rgb brightness up|down
onn-rgb speed up|down
```

With the AppImage, use `/path/to/onn_Keyboard_Lighting-x86_64.AppImage cycle-mode` etc.

### Starting the daemon at login

`scripts/install-user.sh` (`cargo install` + config + the files below,
with absolute paths) installs but does **not** enable anything. Pick one:

- **XDG autostart (default, any desktop, no systemd needed):**
  `cp ~/.local/share/onn-rgb/onn-rgb-daemon.desktop ~/.config/autostart/`
  (disable: remove that copy from `~/.config/autostart/`)
- **systemd user service (optional):**
  `systemctl --user daemon-reload && systemctl --user enable --now onn-rgb.service`
  (`~/.config/systemd/user/onn-rgb.service`; disable with `systemctl --user disable --now onn-rgb.service`)

For the AppImage: `./onn_Keyboard_Lighting-x86_64.AppImage autostart-entry > ~/.config/autostart/onn-rgb-daemon.desktop`
(or `systemd-unit > ~/.config/systemd/user/onn-rgb.service`); the entry points at the AppImage's current path.

## Portability

The binaries are built with `cargo zigbuild` against **glibc 2.28** and link
only libc/libm/libgcc_s. The GUI loads Wayland/X11/EGL/OpenGL libraries at
runtime from the system; they are deliberately not bundled, because GL
drivers must match the host. So the AppImage needs glibc ≥ 2.28 (Debian 10+,
Ubuntu 18.10+, Fedora 29+, RHEL/Alma/Rocky 8+, openSUSE Leap 15.2+, Arch) and
a normal desktop graphics stack. `build-appimage.sh` uses zig from
`~/.local/opt/zig-linux-x86_64-0.13.0` and `cargo install cargo-zigbuild`;
without them it falls back to a native build and prints the glibc it needs.

| | Status |
|---|---|
| Pop!_OS 24.04, COSMIC (Wayland) | **tested**: CLI, step actions, tray registration via D-Bus, GUI launch |
| Fedora, Arch, Debian/Ubuntu, openSUSE; KDE, GNOME (+AppIndicator), Cinnamon, XFCE, X11 sessions | expected to work, not tested |
| evdev hotkeys | implemented and unit-tested; not yet tested with real key presses |
