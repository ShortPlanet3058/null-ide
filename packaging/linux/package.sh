#!/bin/sh
# Builds Null's Linux release in target/release:
#  - null-<version>-linux-x86_64.tar.gz: the binary, its icon, its menu entry, install.sh
#    (which puts them in your home folder) and uninstall.sh;
#  - null-<version>-amd64.deb, for Ubuntu and Debian (`sudo apt install ./null-…deb`): `null`
#    in /usr/bin for every terminal at once, the menu entry and the icon (needs dpkg-deb).
#
#     packaging/linux/package.sh
set -eu
cd "$(dirname "$0")/../.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
name="null-$version-linux-x86_64"
cargo build --release
stage="target/release/$name"
rm -rf "$stage"
mkdir -p "$stage"
cp target/release/null packaging/linux/null.desktop packaging/linux/install.sh packaging/linux/uninstall.sh \
    packaging/null.svg \
    README.md LICENSE "$stage/"
tar -C target/release -czf "target/release/$name.tar.gz" "$name"
rm -rf "$stage"
echo "Built target/release/$name.tar.gz"

if ! command -v dpkg-deb >/dev/null; then
    echo "(No dpkg-deb here: no .deb.)"
    exit 0
fi
deb="target/release/null-deb"
rm -rf "$deb"
install -Dm755 target/release/null "$deb/usr/bin/null"
install -Dm644 packaging/linux/null.desktop "$deb/usr/share/applications/null.desktop"
install -Dm644 packaging/null.svg "$deb/usr/share/icons/hicolor/scalable/apps/null.svg"
install -Dm644 LICENSE "$deb/usr/share/doc/null-editor/copyright"
size=$(du -sk "$deb/usr" | cut -f1)
mkdir -p "$deb/DEBIAN"
# What it links to (libc, xcb, xkbcommon) and loads (Vulkan); a Vulkan driver and the
# desktop's file dialogs are there on a desktop, suggested elsewhere.
cat > "$deb/DEBIAN/control" <<CONTROL
Package: null-editor
Version: $version
Section: editors
Priority: optional
Architecture: amd64
Installed-Size: $size
Depends: libc6 (>= 2.35), libgcc-s1, libxcb1, libxkbcommon0, libxkbcommon-x11-0, libvulkan1
Recommends: mesa-vulkan-drivers | vulkan-icd, xdg-desktop-portal, git
Maintainer: ShortPlanet3058 <118638156+ShortPlanet3058@users.noreply.github.com>
Homepage: https://shortplanet3058.github.io/null-ide/
Description: A calm, fast code editor where nothing gets in your way
 Null is a code editor built around one idea: the screen belongs to your code.
 Language servers, git, a terminal, tests and a debugger are a keystroke away and
 out of sight until then; AI is there when you call it, and nowhere when it's off.
CONTROL
dpkg-deb --root-owner-group --build "$deb" "target/release/null-$version-amd64.deb" >/dev/null
rm -rf "$deb"
echo "Built target/release/null-$version-amd64.deb"
