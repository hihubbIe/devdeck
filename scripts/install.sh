#!/usr/bin/env bash
# Installs devdeck for this user (docs/BUILDING.md, "Install"): the release binary in
# ~/.local/bin, its icon, and a launcher the desktop's app grid and dock show, named
# Kugelblitz (the installed app's name; files and ids stay `devdeck`, so a pin in the
# dock survives a rename), and the control API's clients beside it (`devdeck-ctl`,
# `devdeck-mcp`, R13.11). Run it again to update; `--uninstall` removes what it installed
# (settings and state stay).
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
bin="$HOME/.local/bin/devdeck"
clients=(devdeck-ctl devdeck-mcp)
icon="$data/icons/hicolor/scalable/apps/devdeck.svg"
launcher="$data/applications/devdeck.desktop"

refresh() {
    command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
    command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" || true
}

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -f "$bin" "$icon" "$launcher"
    for c in "${clients[@]}"; do rm -f "$(dirname "$bin")/$c"; done
    refresh
    echo "Kugelblitz (devdeck) uninstalled (settings and state kept)."
    exit 0
fi

# Every binary by name: with one `--bin`, cargo builds only the binaries named.
cargo build --release -p devdeck-app -p devdeck-ctl -p devdeck-mcp --bin devdeck --bin devdeck-ctl --bin devdeck-mcp --manifest-path "$repo/Cargo.toml"

# A copy, not a link into target/: a rebuild or `cargo clean` never breaks it. Written
# beside and renamed over, so a running devdeck keeps its file.
mkdir -p "$(dirname "$bin")" "$(dirname "$icon")" "$(dirname "$launcher")"
install -m 755 "$repo/target/release/devdeck" "$bin.new"
mv -f "$bin.new" "$bin"
for c in "${clients[@]}"; do
    install -m 755 "$repo/target/release/$c" "$(dirname "$bin")/$c.new"
    mv -f "$(dirname "$bin")/$c.new" "$(dirname "$bin")/$c"
done
install -m 644 "$repo/assets/app/devdeck.svg" "$icon"

# StartupWMClass is the window's app id (window_view.rs): the dock shows the open window
# on the pinned icon and a click brings it forward.
cat > "$launcher" <<EOF
[Desktop Entry]
Type=Application
Name=Kugelblitz
Comment=Run, watch and deploy your apps and services (devdeck)
Exec=$bin
Icon=devdeck
Terminal=false
Categories=Development;
StartupWMClass=devdeck
StartupNotify=true
EOF
refresh

echo "Kugelblitz (devdeck) installed: $bin, with ${clients[*]}"
echo "For Claude Code (once): claude mcp add --scope user devdeck -- $(dirname "$bin")/devdeck-mcp"
echo "Pin it: press Super, type Kugelblitz, right-click it, Pin to Dash."
