#!/usr/bin/env bash
# Remove what install.sh put in place. Games, settings and your signed in
# account are left alone; this only removes the app itself.
set -euo pipefail

bin_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"

rm -fv "$bin_dir/xpedited" \
       "$data/applications/xpedited.desktop" \
       "$data/icons/hicolor/256x256/apps/xpedited.png"

command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null &&
    gtk-update-icon-cache -f -t "$data/icons/hicolor" 2>/dev/null || true

echo
echo "Removed. Left in place, delete by hand if you want them gone:"
echo "  ~/.config/xpedited/settings.json   settings"
echo "  ~/.cache/xpedited/                 cached catalogue"
echo "  your games folder                  downloaded games"
echo "  your keyring                       the signed in account ('xpedited logout' first)"
