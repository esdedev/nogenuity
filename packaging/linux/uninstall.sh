#!/bin/sh
# Removes what install.sh installed, and its "start on login" entry. Settings in ~/.config/nogenuity
# are kept (delete that folder by hand to remove them too). Other installs (the .deb, an AppImage)
# are left alone.
set -eu

DATA=${XDG_DATA_HOME:-$HOME/.local/share}
CONFIG=${XDG_CONFIG_HOME:-$HOME/.config}
BIN="$HOME/.local/bin/nogenuity"
RULE=/etc/udev/rules.d/70-nogenuity.rules

# Stop only the installed copy (not a .deb or AppImage one)
for proc in /proc/[0-9]*; do
    if [ "$(readlink "$proc/exe" 2>/dev/null || true)" = "$BIN" ]; then
        kill "${proc#/proc/}" 2>/dev/null || true
    fi
done

# The autostart entry only if it starts this copy (its Exec path is escaped as in install.sh)
AUTOSTART="$CONFIG/autostart/nogenuity.desktop"
QUOTED=$(printf '%s' "$BIN" | sed -e 's/\\/\\\\\\\\/g' -e 's/["`$]/\\\\&/g')
if [ -f "$AUTOSTART" ] && grep -qF -e "Exec=\"$QUOTED\"" -e "Exec=\"$BIN\"" "$AUTOSTART"; then
    rm -f "$AUTOSTART"
fi

rm -f "$BIN" "$DATA/applications/nogenuity.desktop"
rm -f "$DATA"/icons/hicolor/*/apps/nogenuity.png "$DATA"/icons/hicolor/scalable/apps/nogenuity.svg

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$DATA/applications" || true
if [ -f "$DATA/icons/hicolor/icon-theme.cache" ] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t "$DATA/icons/hicolor" || true
fi

echo "Nogenuity removed."
if [ -f "$RULE" ]; then
    echo "The udev rule is still in $RULE. To remove it, as root:"
    echo "  rm $RULE && udevadm control --reload-rules"
    echo "  rm /etc/alsa-card-profile/mixer/paths/nogenuity-alpha-s-game.conf /etc/alsa-card-profile/mixer/profile-sets/nogenuity-alpha-s.conf"
fi
