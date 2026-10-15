#!/bin/sh
# Removes what install.sh put in your home folder: ~/.local/bin/null, its icon, its menu
# entry and the PATH line it added. Your settings, saved API keys, history and language
# servers stay, for if you come back, unless you say so (or pass --everything).
# (Installed with the .deb instead? `sudo apt remove null-editor`.)
#
#     ~/.local/share/null/uninstall.sh [--everything]
set -eu
data="${XDG_DATA_HOME:-$HOME/.local/share}"
config="${XDG_CONFIG_HOME:-$HOME/.config}"
bin="$HOME/.local/bin/null"

everything=
[ "${1:-}" = --everything ] && everything=1
if [ -z "$everything" ] && [ -t 0 ] && { [ -d "$config/null" ] || [ -d "$data/null" ]; }; then
    printf 'Also delete your settings, saved API keys, history and language servers? [y/N] '
    read -r answer || answer=
    case "$answer" in [yYoO]*) everything=1 ;; esac
fi
# (The keys first: Null itself knows where they are.)
if [ -n "$everything" ] && [ -x "$bin" ]; then
    "$bin" --forget-keys || true
fi

rm -f "$bin" "$data/icons/hicolor/scalable/apps/null.svg" "$data/applications/null.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
for rc in "$HOME/.bashrc" "$HOME/.zshrc"; do
    if grep -qs '# added by the Null installer$' "$rc"; then
        sed -i '/# added by the Null installer$/d' "$rc"
    fi
done
echo "Removed Null: ~/.local/bin/null, its menu entry and its PATH line."

if [ -n "$everything" ]; then
    rm -rf "$config/null" "$data/null"
    echo "Deleted your settings ($config/null) and Null's data ($data/null)."
else
    rm -f "$data/null/uninstall.sh"
    echo "Kept your settings ($config/null) and Null's data ($data/null)."
fi
