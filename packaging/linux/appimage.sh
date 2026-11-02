#!/bin/sh
# Builds the AppImage: appimage.sh <nogenuity binary> <version> <output dir> <appimagetool> <runtime>
# appimagetool and the type2 runtime are passed in (the release workflow downloads pinned, checked
# versions). The binary only needs system libraries every desktop has, so nothing else is bundled.
# An AppImage can't install the udev rule: the app's "Grant access" button does it.
set -eu

BIN=$1
VERSION=$2
OUT=$3
TOOL=$4
RUNTIME=$5
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
APPDIR="$STAGE/Nogenuity.AppDir"

mkdir -p "$APPDIR/usr/bin" "$OUT"
install -m 0755 "$BIN" "$APPDIR/usr/bin/nogenuity"
# Without TryExec: AppImage integration tools rewrite Exec to the AppImage, and a TryExec=nogenuity
# not found in PATH would hide the menu entry
grep -v '^TryExec=' "$ROOT/packaging/linux/nogenuity.desktop" > "$APPDIR/nogenuity.desktop"
chmod 0644 "$APPDIR/nogenuity.desktop"
mkdir -p "$APPDIR/usr/share"
cp -R "$ROOT/packaging/linux/icons" "$APPDIR/usr/share/icons"
find "$APPDIR/usr/share/icons" -type d -exec chmod 0755 {} + && find "$APPDIR/usr/share/icons" -type f -exec chmod 0644 {} +
install -m 0644 "$ROOT/packaging/linux/icons/hicolor/256x256/apps/nogenuity.png" "$APPDIR/nogenuity.png"
cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE=$(dirname "$(readlink -f "$0")")
exec "$HERE/usr/bin/nogenuity" "$@"
EOF
chmod 0755 "$APPDIR/AppRun"

# appimagetool is itself an AppImage: extract-and-run works without FUSE (CI runners)
ARCH=x86_64 VERSION="$VERSION" "$TOOL" --appimage-extract-and-run --no-appstream \
    --runtime-file "$RUNTIME" "$APPDIR" "$OUT/Nogenuity-$VERSION-x86_64.AppImage"
echo "$OUT/Nogenuity-$VERSION-x86_64.AppImage"
