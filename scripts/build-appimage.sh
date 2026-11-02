#!/usr/bin/env bash
# Build dist/onn_Keyboard_Lighting-x86_64.AppImage (no root needed).
set -euo pipefail
cd "$(dirname "$0")/.."
TOOLS="${XDG_CACHE_HOME:-$HOME/.cache}/kraken-rgb-tools"
TOOL="$TOOLS/appimagetool-x86_64.AppImage"
mkdir -p "$TOOLS" dist
if [ ! -x "$TOOL" ]; then
  curl -fL -o "$TOOL" https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod +x "$TOOL"
fi

# Build against glibc 2.28 with cargo-zigbuild when zig is available, so the
# AppImage runs on older distros too. Falls back to a native build (whose
# minimum glibc is that of the build machine).
ZIG_DIR="$HOME/.local/opt/zig-linux-x86_64-0.13.0"
[ -x "$ZIG_DIR/zig" ] && export PATH="$ZIG_DIR:$PATH"
export PATH="$HOME/.cargo/bin:$PATH"
GLIBC_TARGET=x86_64-unknown-linux-gnu.2.28
if command -v zig >/dev/null && command -v cargo-zigbuild >/dev/null; then
  cargo zigbuild --release --target "$GLIBC_TARGET"
  REL=target/x86_64-unknown-linux-gnu/release
else
  echo "warning: zig/cargo-zigbuild not found; native build (needs this machine's glibc or newer)" >&2
  cargo build --release
  REL=target/release
fi
for b in onn-rgb onn-rgb-gui; do
  echo "$b needs $(objdump -T "$REL/$b" | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"
done

APPDIR=target/AppDir
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
         "$APPDIR/usr/share/icons/hicolor/scalable/apps" "$APPDIR/usr/share/onn-rgb"
cp "$REL/onn-rgb" "$REL/onn-rgb-gui" "$APPDIR/usr/bin/"
sed 's|^Exec=.*|Exec=onn-rgb-gui|; s|^Icon=.*|Icon=onn-rgb|' onn-rgb.desktop > "$APPDIR/onn-rgb.desktop"
cp "$APPDIR/onn-rgb.desktop" "$APPDIR/usr/share/applications/"
cp packaging/onn-rgb.svg "$APPDIR/onn-rgb.svg"
cp packaging/onn-rgb.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/"
ln -sf onn-rgb.svg "$APPDIR/.DirIcon"
cp udev/*.rules packaging/autostart/onn-rgb-daemon.desktop packaging/systemd/onn-rgb.service "$APPDIR/usr/share/onn-rgb/"
cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
# No arguments: launch the GUI. Any arguments: run the CLI (onn-rgb).
# "udev-rule" prints the bundled udev rules; "autostart-entry" / "systemd-unit"
# print the daemon autostart files, pointing at this AppImage.
HERE="$(dirname "$(readlink -f "$0")")"
case "$1" in
  "") exec "$HERE/usr/bin/onn-rgb-gui" ;;
  --gui) shift; exec "$HERE/usr/bin/onn-rgb-gui" "$@" ;;
  udev-rule) exec cat "$HERE"/usr/share/onn-rgb/*.rules ;;
  autostart-entry) exec sed "s|^Exec=onn-rgb|Exec=${APPIMAGE:-onn-rgb}|" "$HERE/usr/share/onn-rgb/onn-rgb-daemon.desktop" ;;
  systemd-unit) exec sed "s|^ExecStart=onn-rgb|ExecStart=${APPIMAGE:-onn-rgb}|" "$HERE/usr/share/onn-rgb/onn-rgb.service" ;;
  *) exec "$HERE/usr/bin/onn-rgb" "$@" ;;
esac
RUN
chmod +x "$APPDIR/AppRun"

OUT=dist/onn_Keyboard_Lighting-x86_64.AppImage
EXTRA=()
[ -e /dev/fuse ] && command -v fusermount >/dev/null || EXTRA=(--appimage-extract-and-run)
ARCH=x86_64 "$TOOL" "${EXTRA[@]}" "$APPDIR" "$OUT"
chmod +x "$OUT"
ls -lh "$OUT"
