#!/bin/sh
# Build onn-rgb-x86_64.AppImage in the project root.
# usage: packaging/build-appimage.sh /path/to/appimagetool-x86_64.AppImage
set -eu

tool=$(realpath "$1")
cd "$(dirname "$0")/.."

cargo build --release --locked --bin onn-rgb-gui

appdir=target/appimage/onn-rgb.AppDir
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin"
cp target/release/onn-rgb-gui "$appdir/usr/bin/"
cp packaging/onn-rgb.png "$appdir/"
sed 's/^Icon=.*/Icon=onn-rgb/' onn-rgb.desktop > "$appdir/onn-rgb.desktop"
ln -s usr/bin/onn-rgb-gui "$appdir/AppRun"

# --appimage-extract-and-run lets the tool itself run without FUSE.
ARCH=x86_64 "$tool" --appimage-extract-and-run "$appdir" onn-rgb-x86_64.AppImage
