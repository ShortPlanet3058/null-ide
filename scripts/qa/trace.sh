#!/bin/bash
# How long Null takes to start (and to open FILE): runs BIN (the release build unless
# NULL_QA_BIN says otherwise) on a copy of PROJECT with NULL_TRACE=1, in a home of its own
# as shot.sh does (your settings, sessions, history and keychain untouched, nothing
# installed), waits SECONDS, quits it, and prints what it measured.
#
#   scripts/qa/trace.sh PROJECT [FILE] [SECONDS]
#
# FILE is a path in PROJECT, opened as `null FILE` would. The window comes to the front
# while it runs.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
bin="${NULL_QA_BIN:-$repo/target/release/null}"
[ -x "$bin" ] || { echo "no $bin: build it first" >&2; exit 1; }
[ -d "${1:-}" ] || { echo "usage: $0 PROJECT [FILE] [SECONDS]" >&2; exit 1; }
wait_for="${3:-12}"

home="$(mktemp -d)"
pid=""
finish() {
    if [ -n "$pid" ]; then
        pkill -TERM -P "$pid" 2>/dev/null || true
        kill -TERM -- "-$pid" 2>/dev/null || kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
        sleep 0.5
    fi
    rm -rf "$home" 2>/dev/null || { sleep 1; rm -rf "$home"; }
}
trap finish EXIT
mkdir -p "$home/.config/null" "$home/tmp" "$home/project"
project="$home/project/$(basename "$(cd "$1" && pwd)")"
rsync -a --exclude target/ --exclude node_modules/ "$(cd "$1" && pwd)/" "$project/"
printf '{ "welcomed": true }\n' > "$home/.config/null/settings.json"
open_arg="$project${2:+/$2}"

real_home="$HOME"
tools="$real_home/.cargo/bin:$real_home/.local/bin:/opt/homebrew/bin:/usr/local/bin"
servers="$real_home/Library/Application Support/Null/servers"
tools="$tools:$servers/node_modules/.bin:$servers/bin"
log="$home/trace.log"
cd "$project"
set -m
env -i \
    PATH="$PATH:$tools" USER="${USER:-}" LOGNAME="${LOGNAME:-}" SHELL="${SHELL:-/bin/zsh}" \
    LANG="${LANG:-en_US.UTF-8}" TERM=xterm-256color TMPDIR="$home/tmp/" \
    HOME="$home" CFFIXED_USER_HOME="$home" \
    RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \
    CARGO_TARGET_DIR="$home/target" CARGO_NET_OFFLINE=true \
    NULL_DATA_DIR="$home/data" NULL_TRACE=1 \
    "$bin" "$open_arg" > "$log" 2>&1 &
pid=$!
set +m
sleep "$wait_for"
grep '^null trace' "$log" || { cat "$log" >&2; echo "nothing measured" >&2; }
