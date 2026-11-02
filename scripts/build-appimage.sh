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

cargo build --release

APPDIR=target/AppDir
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
         "$APPDIR/usr/share/icons/hicolor/scalable/apps" "$APPDIR/usr/share/onn-rgb"
cp target/release/onn-rgb target/release/onn-rgb-gui "$APPDIR/usr/bin/"
sed 's|^Exec=.*|Exec=onn-rgb-gui|; s|^Icon=.*|Icon=onn-rgb|' onn-rgb.desktop > "$APPDIR/onn-rgb.desktop"
cp "$APPDIR/onn-rgb.desktop" "$APPDIR/usr/share/applications/"
cp packaging/onn-rgb.svg "$APPDIR/onn-rgb.svg"
cp packaging/onn-rgb.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/"
ln -sf onn-rgb.svg "$APPDIR/.DirIcon"
cp udev/70-onn-keyboard.rules "$APPDIR/usr/share/onn-rgb/"
cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
# No arguments: launch the GUI. Any arguments: run the CLI (onn-rgb).
# "udev-rule" prints the bundled udev rule.
HERE="$(dirname "$(readlink -f "$0")")"
case "$1" in
  "") exec "$HERE/usr/bin/onn-rgb-gui" ;;
  --gui) shift; exec "$HERE/usr/bin/onn-rgb-gui" "$@" ;;
  udev-rule) exec cat "$HERE/usr/share/onn-rgb/70-onn-keyboard.rules" ;;
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
