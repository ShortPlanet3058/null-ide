#!/bin/sh
# Installs Null for this person on Linux: the binary into ~/.local/bin, its icon and its
# entry in the applications menu (as the `null` command, and "Open With"). Nothing
# system-wide, nothing downloaded. ~/.local/share/null/uninstall.sh removes it again.
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
# (Quoted as a .desktop file wants it, then made safe for sed: a home named with a space,
# a `&`, a `|`, a `%` or a `\` stays as it is. A `\` is escaped twice: in the quotes, and
# in the file's text.)
exe=$(printf '%s' "$HOME/.local/bin/null" | sed -e 's/\\/\\\\\\\\/g' -e 's/["`$]/\\\\&/g' -e 's/%/%%/g')
exe=$(printf '"%s"' "$exe" | sed 's/[|&\\]/\\&/g')
sed "s|^Exec=null |Exec=$exe |" "$desktop" > "$data/applications/null.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
# (Kept where it outlives the download: removing Null is one command.)
install -Dm755 "$here/uninstall.sh" "$data/null/uninstall.sh"
echo "Installed Null: ~/.local/bin/null and its menu entry"
echo "To remove it: $data/null/uninstall.sh"

# `null` in a terminal: ~/.local/bin on the PATH. Ubuntu's ~/.profile adds it only at the
# next login, and zsh never reads that file: the line added to the shell's own file.
case ":$PATH:" in
    *":$HOME/.local/bin:"*) exit 0 ;;
esac
line='export PATH="$HOME/.local/bin:$PATH"  # added by the Null installer'
added=
for rc in "$HOME/.bashrc" "$HOME/.zshrc"; do
    [ -f "$rc" ] || continue
    grep -qs '/.local/bin' "$rc" && continue
    printf '\n%s\n' "$line" >> "$rc"
    added="$added ${rc##*/}"
done
if [ -n "$added" ]; then
    echo "Added ~/.local/bin to the PATH in$added: open a new terminal and \`null .\` works."
else
    echo "~/.local/bin isn't on your PATH: add it to your shell's settings to type \`null\`."
fi
