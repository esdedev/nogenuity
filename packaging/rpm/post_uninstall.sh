# Same job as packaging/deb/postrm. $1 is 0 on a real removal, 1 or more on an upgrade.
if [ "$1" = 0 ] && command -v udevadm >/dev/null 2>&1; then
    udevadm control --reload-rules || true
    # Drop the access the removed rule gave to a headset that is still plugged in
    udevadm trigger --subsystem-match=hidraw --action=change || true
fi
