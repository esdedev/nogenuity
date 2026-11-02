#!/bin/sh
# Builds the Linux release tarball: package.sh <nogenuity binary> <version> <output dir>
# Contents: the binary, install.sh / uninstall.sh, the menu entry, icons, the udev rule and docs.
set -eu

BIN=$1
VERSION=$2
OUT=$3
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
NAME="nogenuity-$VERSION-linux-x86_64"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
DIR="$STAGE/$NAME"

mkdir -p "$DIR" "$OUT"
install -m 0755 "$BIN" "$DIR/nogenuity"
install -m 0755 "$ROOT/packaging/linux/install.sh" "$ROOT/packaging/linux/uninstall.sh" "$DIR/"
install -m 0644 "$ROOT/packaging/linux/nogenuity.desktop" "$ROOT/packaging/linux/70-nogenuity.rules" "$DIR/"
cp -R "$ROOT/packaging/linux/icons" "$DIR/icons"
find "$DIR/icons" -type d -exec chmod 0755 {} + && find "$DIR/icons" -type f -exec chmod 0644 {} +
install -m 0644 "$ROOT/README.md" "$ROOT/README.es.md" "$ROOT/LICENSE" "$ROOT/CONTRIBUTING.md" "$DIR/"

tar -C "$STAGE" -czf "$OUT/$NAME.tar.gz" "$NAME"
echo "$OUT/$NAME.tar.gz"
