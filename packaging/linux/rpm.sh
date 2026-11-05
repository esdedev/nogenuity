#!/bin/sh
# Builds the .rpm: rpm.sh <target triple> <output dir>, from the repository root, with the release
# binary already built for that target. Needs cargo-generate-rpm.
# Fedora has no plugdev group: the packaged rule goes without it (uaccess is enough).
set -eu

TARGET=$1
OUT=$2

mkdir -p target/rpm "$OUT"
sed 's/, GROUP="plugdev"//' packaging/linux/70-nogenuity.rules > target/rpm/70-nogenuity.rules
cargo generate-rpm --target "$TARGET" -o "$OUT"
