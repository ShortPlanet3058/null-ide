#!/bin/bash
# What CI checks, on the Mac, before a pull request is marked ready: formatting, lints and
# tests here, and lints for Windows and Linux (type-checked only: scripts/check-platform.sh).
#
#   scripts/check.sh            all of it
#   scripts/check.sh --quick    without the tests
set -euo pipefail
cd "$(dirname "$0")/.."
step() { printf '\n== %s\n' "$1"; }

step "Formatting"
cargo fmt --check
step "Lints (Mac)"
cargo clippy --all-targets -- -D warnings
for platform in windows linux; do
    step "Lints ($platform)"
    NULL_CHECK=clippy scripts/check-platform.sh "$platform" --all-targets --offline -- -D warnings
done
if [ "${1:-}" != "--quick" ]; then
    step "Tests"
    cargo test
fi
printf '\nAll good.\n'
