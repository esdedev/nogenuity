# Same job as packaging/deb/postinst. Runs on install and on upgrade.
# A copy in /etc (from the app's Grant access button or install.sh) would hide the packaged rule
# in /usr/lib, and with it any headset a later version adds. Remove it if Nogenuity wrote it.
ETC_RULE=/etc/udev/rules.d/70-nogenuity.rules
if [ -f "$ETC_RULE" ] && head -n 1 "$ETC_RULE" | grep -q '^# Nogenuity:'; then
    rm -f "$ETC_RULE"
    echo "Nogenuity: removed $ETC_RULE, the package's rule in /usr/lib/udev/rules.d replaces it."
    # And the audio profile it wrote, which would hide the packaged one in the same way
    rm -f /etc/alsa-card-profile/mixer/paths/nogenuity-alpha-s-game.conf \
        /etc/alsa-card-profile/mixer/paths/nogenuity-alpha-s-chat.conf \
        /etc/alsa-card-profile/mixer/profile-sets/nogenuity-alpha-s.conf
fi
# Apply the udev rule now, so the headset works without unplugging it
if command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules || true
    udevadm trigger --subsystem-match=hidraw --action=change || true
fi
echo "Nogenuity: with a Cloud Alpha S, plug it back in to get its separate Game and Chat outputs."
