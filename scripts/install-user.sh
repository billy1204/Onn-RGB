#!/usr/bin/env bash
# Install onn-rgb for the current user (no root). Does NOT enable autostart.
#   scripts/install-user.sh [path/to/onn-rgb]   (binary or AppImage; default: cargo install)
set -euo pipefail
cd "$(dirname "$0")/.."
if [ $# -ge 1 ]; then
  BIN=$(realpath "$1")
else
  cargo install --path .
  BIN="$HOME/.cargo/bin/onn-rgb"
fi
SHARE="$HOME/.local/share/onn-rgb"
mkdir -p "$SHARE" "$HOME/.local/share/applications" "$HOME/.config/systemd/user"
sed "s|^Exec=onn-rgb daemon|Exec=$BIN daemon|" packaging/autostart/onn-rgb-daemon.desktop > "$SHARE/onn-rgb-daemon.desktop"
sed "s|^ExecStart=onn-rgb daemon|ExecStart=$BIN daemon|" packaging/systemd/onn-rgb.service > "$HOME/.config/systemd/user/onn-rgb.service"
cp udev/*.rules "$SHARE/"
cp modules-load/onn-rgb.conf "$SHARE/"
if [ $# -eq 0 ]; then cp onn-rgb.desktop "$HOME/.local/share/applications/"; fi
"$BIN" config >/dev/null
cat <<MSG
Installed (not enabled):
  $SHARE/onn-rgb-daemon.desktop      autostart entry
  ~/.config/systemd/user/onn-rgb.service
  ~/.config/onn-rgb/config.toml
Enable ONE of:
  cp $SHARE/onn-rgb-daemon.desktop ~/.config/autostart/
  systemctl --user daemon-reload && systemctl --user enable --now onn-rgb.service
MSG
