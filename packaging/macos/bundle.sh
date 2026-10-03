#!/bin/sh
# Builds Null.app: the release binary, its icon and Info.plist, signed for this Mac
# (ad hoc). Everything it uses comes with macOS and Rust: nothing to install.
#
#     packaging/macos/bundle.sh            →  target/release/Null.app
#     packaging/macos/bundle.sh --install  →  also copies it to /Applications
set -eu
cd "$(dirname "$0")/../.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
app=target/release/Null.app

cargo build --release
rm -rf "$app" target/release/Null.iconset
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/null "$app/Contents/MacOS/Null"
sed "s/__VERSION__/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"

swift packaging/macos/make-icon.swift target/release/Null.iconset
iconutil -c icns target/release/Null.iconset -o "$app/Contents/Resources/Null.icns"
rm -rf target/release/Null.iconset

# Ad hoc: runs on this Mac. Sharing it with others needs a Developer ID signature.
codesign --force --sign - "$app"
echo "Built $app ($version)"

if [ "${1:-}" = "--install" ]; then
    rm -rf /Applications/Null.app
    cp -R "$app" /Applications/
    echo "Installed /Applications/Null.app"
fi
