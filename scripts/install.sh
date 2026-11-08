#!/usr/bin/env bash
# Installs devdeck for this user (docs/BUILDING.md, "Install"): the release binary in
# ~/.local/bin, its icon, and a launcher the desktop's app grid and dock show. Run it
# again to update; `--uninstall` removes the three files (settings and state stay).
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
bin="$HOME/.local/bin/devdeck"
icon="$data/icons/hicolor/scalable/apps/devdeck.svg"
launcher="$data/applications/devdeck.desktop"

refresh() {
    command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
    command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" || true
}

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -f "$bin" "$icon" "$launcher"
    refresh
    echo "devdeck uninstalled (settings and state kept)."
    exit 0
fi

cargo build --release -p devdeck-app --bin devdeck --manifest-path "$repo/Cargo.toml"

# A copy, not a link into target/: a rebuild or `cargo clean` never breaks it. Written
# beside and renamed over, so a running devdeck keeps its file.
mkdir -p "$(dirname "$bin")" "$(dirname "$icon")" "$(dirname "$launcher")"
install -m 755 "$repo/target/release/devdeck" "$bin.new"
mv -f "$bin.new" "$bin"
install -m 644 "$repo/assets/app/devdeck.svg" "$icon"

# StartupWMClass is the window's app id (window_view.rs): the dock shows the open window
# on the pinned icon and a click brings it forward.
cat > "$launcher" <<EOF
[Desktop Entry]
Type=Application
Name=devdeck
Comment=Run, watch and deploy your apps and services
Exec=$bin
Icon=devdeck
Terminal=false
Categories=Development;
StartupWMClass=devdeck
StartupNotify=true
EOF
refresh

echo "devdeck installed: $bin"
echo "Pin it: press Super, type devdeck, right-click it, Pin to Dash."
