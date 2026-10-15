#!/bin/sh
# Builds Null.app: the release binary, its icon and Info.plist, signed (ad hoc unless a
# Developer ID is given). Everything it uses comes with macOS and Rust: nothing to install.
#
#     packaging/macos/bundle.sh             →  target/release/Null.app, for this Mac
#     packaging/macos/bundle.sh --install   →  also copies it to /Applications
#     packaging/macos/bundle.sh --release   →  for every Mac (Intel and Apple silicon in
#                                              one binary) and target/release/Null-<v>.dmg
#
# NULL_SIGN_IDENTITY="Developer ID Application: …" signs with that identity instead (with
# the hardened runtime, as notarising needs); without it the app is signed ad hoc and
# macOS asks before opening it the first time on another Mac.
set -eu
cd "$(dirname "$0")/../.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
app=target/release/Null.app
mode="${1:-}"

if [ "$mode" = "--release" ]; then
    # Both kinds of Mac: each built for itself, then one binary holding both.
    for arch in x86_64 aarch64; do
        rustup target add "$arch-apple-darwin" >/dev/null
        MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --release --target "$arch-apple-darwin"
    done
    mkdir -p target/release
    lipo -create -output target/release/null \
        target/x86_64-apple-darwin/release/null target/aarch64-apple-darwin/release/null
else
    cargo build --release
fi

rm -rf "$app" target/release/Null.iconset
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/null "$app/Contents/MacOS/Null"
sed "s/__VERSION__/$version/g" packaging/macos/Info.plist > "$app/Contents/Info.plist"

swift packaging/macos/make-icon.swift target/release/Null.iconset
iconutil -c icns target/release/Null.iconset -o "$app/Contents/Resources/Null.icns"
rm -rf target/release/Null.iconset

if [ -n "${NULL_SIGN_IDENTITY:-}" ]; then
    codesign --force --options runtime --timestamp --sign "$NULL_SIGN_IDENTITY" "$app"
else
    # Ad hoc: runs here; elsewhere macOS asks first (right-click → Open).
    codesign --force --sign - "$app"
fi
echo "Built $app ($version)"

case "$mode" in
    --install)
        rm -rf /Applications/Null.app
        cp -R "$app" /Applications/
        echo "Installed /Applications/Null.app"
        ;;
    --release)
        # A disk image holding the app and a link to Applications, to drag it onto.
        dmg="target/release/Null-$version.dmg"
        stage=target/release/dmg
        rm -rf "$stage" "$dmg"
        # (On CI the two builds' leftovers filled the disk before the image was made.)
        if [ -n "${CI:-}" ]; then
            for arch in x86_64 aarch64; do
                rm -rf "target/$arch-apple-darwin/release/deps" "target/$arch-apple-darwin/release/build"
            done
        fi
        mkdir -p "$stage"
        cp -R "$app" "$stage/"
        ln -s /Applications "$stage/Applications"
        hdiutil create -volname "Null $version" -srcfolder "$stage" -ov -format UDZO "$dmg" >/dev/null
        rm -rf "$stage"
        echo "Built $dmg"
        ;;
esac
