#!/bin/sh
# Installs Null for this person on Linux: the binary into ~/.local/bin, its icon and its
# entry in the applications menu (as the `null` command, and "Open With"). Nothing
# system-wide, nothing downloaded.
#
#     ./install.sh                    from the release's folder: its own `null`
#     packaging/linux/install.sh      from the source: built first
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
if [ -x "$here/null" ]; then
    bin="$here/null" desktop="$here/null.desktop" icon="$here/null.svg"
else
    cd "$here/../.."
    cargo build --release
    bin=target/release/null desktop=packaging/linux/null.desktop icon=packaging/null.svg
fi

data="${XDG_DATA_HOME:-$HOME/.local/share}"
install -Dm755 "$bin" "$HOME/.local/bin/null"
install -Dm644 "$icon" "$data/icons/hicolor/scalable/apps/null.svg"
# (Exec names it whole: ~/.local/bin isn't on every desktop session's PATH.)
mkdir -p "$data/applications"
sed "s|^Exec=null |Exec=$HOME/.local/bin/null |" "$desktop" > "$data/applications/null.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
echo "Installed Null: ~/.local/bin/null and its menu entry"
