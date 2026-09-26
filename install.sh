#!/usr/bin/env bash
# Install Xpedited for the current user: binary on PATH, icon and menu entry
# where the desktop will find them. Nothing here needs root.
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
bin_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
apps_dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icon_dir="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"

binary="$here/target/release/xpedited"
if [ ! -x "$binary" ]; then
    echo "Building first; this takes a few minutes the first time."
    cargo build --release --manifest-path "$here/Cargo.toml"
fi

mkdir -p "$bin_dir" "$apps_dir" "$icon_dir"
install -m 755 "$binary" "$bin_dir/xpedited"
install -m 644 "$here/assets/xpedited.png" "$icon_dir/xpedited.png"

cat > "$apps_dir/xpedited.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Xpedited
GenericName=Xbox Game Pass
Comment=Browse, install and play Xbox Game Pass games
Exec=$bin_dir/xpedited app
Icon=xpedited
Terminal=false
Categories=Game;
StartupWMClass=xpedited
DESKTOP

command -v update-desktop-database >/dev/null && update-desktop-database "$apps_dir" || true
command -v gtk-update-icon-cache >/dev/null &&
    gtk-update-icon-cache -f -t "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor" 2>/dev/null || true

echo "Installed to $bin_dir/xpedited"
case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) echo "Note: $bin_dir is not on your PATH, so the menu entry will work but the" \
            "'xpedited' command will not until you add it." ;;
esac
echo
echo "Run it from your applications menu, or:  xpedited app"
echo "The first run needs the path to the patched Wine build:"
echo "  xpedited app /path/to/wine        (remembered afterwards)"
