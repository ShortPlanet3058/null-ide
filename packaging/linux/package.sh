#!/bin/sh
# Builds Null's Linux release: target/release/null-<version>-linux-x86_64.tar.gz, holding
# the binary, its icon, its menu entry and install.sh (which puts them in place).
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
cp target/release/null packaging/linux/null.desktop packaging/linux/install.sh packaging/null.svg \
    README.md LICENSE "$stage/"
tar -C target/release -czf "target/release/$name.tar.gz" "$name"
rm -rf "$stage"
echo "Built target/release/$name.tar.gz"
