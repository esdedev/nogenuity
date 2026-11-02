#!/bin/sh
# Installs Nogenuity for the current user (no root): binary, menu entry and icons.
# Run it from the unpacked release folder. Safe to run again (it updates the files).
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
DATA=${XDG_DATA_HOME:-$HOME/.local/share}
BIN="$HOME/.local/bin"
RULE=70-nogenuity.rules

if [ ! -f "$HERE/nogenuity" ]; then
    echo "nogenuity binary not found next to install.sh" >&2
    exit 1
fi

# Exec value of a .desktop file (Desktop Entry spec): quoted, with " ` $ \ escaped by a backslash,
# itself doubled because the value is also a string
desktop_quote() {
    printf '"%s"' "$(printf '%s' "$1" | sed -e 's/\\/\\\\\\\\/g' -e 's/["`$]/\\\\&/g')"
}
# Plain string value: only \ is escaped
desktop_string() {
    printf '%s' "$1" | sed -e 's/\\/\\\\/g'
}

mkdir -p "$BIN" "$DATA/applications"
install -m 0755 "$HERE/nogenuity" "$BIN/nogenuity"

# Menu entry pointing at the installed binary (~/.local/bin is not always in the desktop's PATH)
EXEC=$(desktop_quote "$BIN/nogenuity")
TRYEXEC=$(desktop_string "$BIN/nogenuity")
while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
        Exec=*) printf 'Exec=%s\n' "$EXEC" ;;
        TryExec=*) printf 'TryExec=%s\n' "$TRYEXEC" ;;
        *) printf '%s\n' "$line" ;;
    esac
done < "$HERE/nogenuity.desktop" > "$DATA/applications/nogenuity.desktop"
chmod 0644 "$DATA/applications/nogenuity.desktop"

for icon in "$HERE"/icons/hicolor/*/apps/nogenuity.*; do
    [ -e "$icon" ] || continue
    size=$(basename "$(dirname "$(dirname "$icon")")")
    mkdir -p "$DATA/icons/hicolor/$size/apps"
    install -m 0644 "$icon" "$DATA/icons/hicolor/$size/apps/"
done

# If it already started with the session from another place, start the installed copy instead
"$BIN/nogenuity" --refresh-autostart || true

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$DATA/applications" || true
# Only refresh an icon cache that already exists: creating one would leave it behind on uninstall
if [ -f "$DATA/icons/hicolor/icon-theme.cache" ] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t "$DATA/icons/hicolor" || true
fi

echo "Nogenuity installed in $BIN/nogenuity and added to the application menu."

# Access to the headset: udev rule (needs root once)
if [ -f "/etc/udev/rules.d/$RULE" ] || [ -f "/usr/lib/udev/rules.d/$RULE" ] || [ -f "/lib/udev/rules.d/$RULE" ]; then
    exit 0
fi
manual() {
    echo "As root, run:"
    echo "  install -m 0644 \"$HERE/$RULE\" /etc/udev/rules.d/$RULE"
    echo "  udevadm control --reload-rules && udevadm trigger --subsystem-match=hidraw --action=change"
}
echo
echo "Nogenuity also needs a udev rule to talk to the headset without root."
answer=n
if [ -t 0 ]; then
    printf "Install it now? It asks for your password once. [y/N] "
    read -r answer || answer=n
fi
case "$answer" in
    y|Y|yes|YES)
        # The same pkexec flow as the app's Grant access button (one place decides the rule)
        if "$BIN/nogenuity" --grant-access; then
            echo "Done. The app connects within a few seconds."
        else
            manual
        fi
        ;;
    *)
        echo "Skipped. The app shows a \"Grant access\" button, or:"
        manual
        ;;
esac
