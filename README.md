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
Mode / Brightness / Speed submenus, a "Block arrow keys during Fn combos"
checkbox, Open GUI, Refresh and Quit. Left-click opens the GUI. Updates are
event-driven: the tray changes when the daemon applies a change and re-reads
the keyboard when the menu opens. There is no polling timer; at idle the daemon
sleeps in `poll()` and makes no wakeups.

| Desktop | Tray |
|---|---|
| KDE Plasma, COSMIC, Cinnamon, XFCE (with the Status Tray plugin), LXQt, Budgie, MATE | built in |
| GNOME | needs the **AppIndicator and KStatusNotifierItem Support** extension (preinstalled on Ubuntu as "Ubuntu AppIndicators") |
| No tray host (sway without waybar tray, etc.) | daemon logs "tray: not available" and keeps running hotkeys |

### Fn combos (evdev + uinput, works on X11 and Wayland)

The keyboard's firmware does nothing for Fn+key on its own. Pressing **Fn**
sends `KEY_VOLUMEUP` (115) on the second input node (event5 here), and Fn+arrow
then sends a plain arrow on the main node (event4). The daemon treats that
Vol+ as an Fn modifier:

| Combo | Action |
|---|---|
| Fn+Up / Fn+Down | brightness +1 / -1 (0–3) |
| Fn+Right / Fn+Left | speed +1 / -1 (0–12) |
| Fn+Insert, Fn+PageDown | cycle mode Pulse → Steady → Off |
| Fn+Home | Steady |
| Fn+PageUp | Pulse |
| Fn+Delete, Fn+End | toggle Off / last mode |

Fn arms for `window-ms` (400 ms, renewed by each combo) and disarms when
released. Only presses act; autorepeat is ignored; each action is one HID
write, done on a separate thread so typing is never delayed.

`~/.config/onn-rgb/config.toml` (`onn-rgb config` prints the path):

```toml
[fn]
key = "KEY_VOLUMEUP"
window-ms = 400
release-disarms = true    # set false if Fn+Up does nothing (keyboard sends Fn release instantly)
grab = true               # hide Fn's Vol+ from the desktop (needs uinput, below)
block-combo-keys = false  # also swallow the arrow/Insert/... of a combo

[fn.keys]
KEY_UP = "brightness-up"
KEY_DOWN = "brightness-down"
KEY_RIGHT = "speed-up"
KEY_LEFT = "speed-down"
KEY_INSERT = "cycle-mode"
KEY_HOME = "mode-steady"
KEY_PAGEUP = "mode-pulse"
KEY_DELETE = "toggle"
KEY_END = "toggle"
KEY_PAGEDOWN = "cycle-mode"
```

Actions: `cycle-mode`, `toggle`, `brightness-up`, `brightness-down`,
`speed-up`, `speed-down`, `mode-steady`, `mode-pulse`. The older `[hotkeys]`
section still binds plain keys (no Fn) to actions.

**Grab and passthrough.** With `grab = true` the daemon grabs the Fn node
(`EVIOCGRAB`) and re-emits everything except the Fn Vol+ through a virtual
device named **"onn-rgb passthrough"**, so Fn no longer changes the volume.
With `block-combo-keys` (or the tray checkbox) it grabs the main node too and
drops only the keys consumed by a combo; Caps/Num Lock LEDs are forwarded back
to the real keyboard. Grabs are released on exit, Quit, SIGTERM/SIGINT and
crashes (the kernel drops a grab when the process dies). A grab is only taken
when no key is held. If `/dev/uinput` can't be opened the daemon logs it, shows
a desktop notification and runs **ungrabbed**: combos still work but Fn also
raises the volume. The keyboard is never left dead. Side effect: a real Vol+
key on this keyboard would act as Fn.

**Hotplug and resume.** Nodes are reopened after replug or suspend from a
kernel uevent (netlink) monitor, not by polling. Only one daemon runs at a time
(lock in `$XDG_RUNTIME_DIR/onn-rgb.lock`).

`onn-rgb listen-keys` prints every event with its value (1 press, 0 release,
2 repeat), e.g. `/dev/input/event5: KEY_VOLUMEUP = 115 value 1 (press)`.

**Permissions** (one time):

- Input nodes: `udev/71-onn-keyboard-input.rules`.
- uinput (for grab): `udev/72-onn-uinput.rules` plus `modules-load/onn-rgb.conf`
  so the `uinput` module loads at boot:

```sh
sudo cp udev/71-onn-keyboard-input.rules udev/72-onn-uinput.rules /etc/udev/rules.d/
sudo cp modules-load/onn-rgb.conf /etc/modules-load.d/
sudo modprobe uinput && sudo udevadm control --reload && sudo udevadm trigger
```

- **systemd or elogind distros** (Fedora, Ubuntu/Pop!_OS, Debian, Arch,
  openSUSE, Gentoo/Void/Alpine with elogind): the rules' `uaccess` tag gives
  the logged-in seat user access.
- **No logind** (Void/Alpine/Artix without elogind): `uaccess` does nothing;
  the rules also set `GROUP="input", MODE="0660"`, so run
  `sudo usermod -aG input $USER` and log in again. Without systemd,
  `/etc/modules-load.d` may be ignored; load uinput via your init's module
  list (e.g. `/etc/modules` on OpenRC/Alpine, `/etc/modules-load.d` is read by
  Void's runit too) or `modprobe uinput` at boot. The same group applies to
  the hidraw rule (`70-onn-keyboard.rules`).

Note: input access lets the user read every key typed on this keyboard, and
uinput lets them inject input. Skip these rules if you don't want hotkeys.

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
