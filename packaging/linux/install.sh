#!/bin/sh
# Installs Null for this person on Linux: the release binary into ~/.local/bin, its icon
# and its entry in the applications menu (as the `null` command, and "Open With").
# Nothing system-wide, nothing downloaded.
#
#     packaging/linux/install.sh
set -eu
cd "$(dirname "$0")/../.."

data="${XDG_DATA_HOME:-$HOME/.local/share}"
cargo build --release
install -Dm755 target/release/null "$HOME/.local/bin/null"
install -Dm644 packaging/null.svg "$data/icons/hicolor/scalable/apps/null.svg"
install -Dm644 packaging/linux/null.desktop "$data/applications/null.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
echo "Installed Null: ~/.local/bin/null and its menu entry"
