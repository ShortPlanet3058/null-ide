#!/bin/bash
# Looks at Null as it is: runs the debug build on PROJECT with the steps in STEPS (see
# src/qa.rs), takes a picture of its window when the steps say `ready`, and quits it.
#
#   scripts/qa/shot.sh STEPS OUT.png [PROJECT] [WAIT_SECONDS]
#
# `shot NAME` steps take more pictures on the way, as NAME.png beside OUT.png.
# NULL_QA_SETTINGS='"keymap": "vim", "word_wrap": true' adds settings to its own.
#
# It keeps to itself: a home of its own (settings, sessions, history, its shell's
# history), only the environment it needs (no API keys, logs or shell setup of yours),
# Rust's builds and downloads off (rust-analyzer builds into the home, offline), and,
# without PROJECT, an empty project of its own. What it can't keep apart is the Mac's:
# its clipboard (so steps don't copy or paste) and the keyboard: the window comes to
# the front for the few seconds it runs (macOS doesn't redraw a covered window), and
# keys typed then go to it. (NULL_QA_FRONT=0 leaves it behind, showing its first frame.)
#
# Needs `cargo build` first (target/debug/null).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
steps="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
out="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
wait_for="${4:-30}"
bin="$repo/target/debug/null"
[ -x "$bin" ] || { echo "no $bin: run cargo build first" >&2; exit 1; }

# The window finder, built once (kept next to the build, which git leaves out).
winid="$repo/target/qa-winid"
if [ ! -x "$winid" ] || [ "$here/winid.swift" -nt "$winid" ]; then
    swiftc -O -o "$winid" "$here/winid.swift"
fi

home="$(mktemp -d)"
project="${3:-$home/project}"
mkdir -p "$home/.config/null" "$home/tmp" "$project"
project="$(cd "$project" && pwd)"
printf '{ "welcomed": true%s }\n' "${NULL_QA_SETTINGS:+, $NULL_QA_SETTINGS}" > "$home/.config/null/settings.json"
log="$home/qa.log"
pid=""
finish() {
    if [ -n "$pid" ]; then
        # Its language servers and shell first, then it; gone before its home is.
        pkill -TERM -P "$pid" 2>/dev/null || true
        kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
    fi
    rm -rf "$home"
}
trap finish EXIT

# The window's number, or the log and why not.
window_of() {
    local id
    id="$("$winid" "$pid" || true)"
    if [ -z "$id" ]; then
        cat "$log" >&2
        echo "no window of null's to take a picture of" >&2
        exit 1
    fi
    echo "$id"
}

cd "$project"
env -i \
    PATH="$PATH" USER="${USER:-}" LOGNAME="${LOGNAME:-}" SHELL="${SHELL:-/bin/zsh}" \
    LANG="${LANG:-en_US.UTF-8}" TERM=xterm-256color TMPDIR="$home/tmp/" \
    HOME="$home" CFFIXED_USER_HOME="$home" \
    RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \
    CARGO_TARGET_DIR="$home/target" CARGO_NET_OFFLINE=true \
    NULL_DATA_DIR="$home/data" NULL_QA="$steps" NULL_QA_FRONT="${NULL_QA_FRONT:-1}" \
    "$bin" "$project" > "$log" 2>&1 &
pid=$!

# `shot <name>` steps: a picture each, beside OUT.png, as they come.
taken=0
take_shots() {
    local names
    names="$( (grep '^NULL_QA_SHOT ' "$log" || true) | sed 's/^NULL_QA_SHOT //' | tail -n +$((taken + 1)))"
    [ -z "$names" ] && return 0
    while IFS= read -r name; do
        local id
        id="$(window_of)"
        screencapture -x -o -l "$id" "$(dirname "$out")/$name.png"
        echo "$(dirname "$out")/$name.png"
        taken=$((taken + 1))
    done <<< "$names"
}
for _ in $(seq $((wait_for * 10))); do
    take_shots
    grep -q NULL_QA_READY "$log" && break
    kill -0 "$pid" 2>/dev/null || { cat "$log" >&2; echo "null quit before it was ready" >&2; exit 1; }
    sleep 0.1
done
take_shots
grep -q NULL_QA_READY "$log" || { cat "$log" >&2; echo "not ready after ${wait_for}s" >&2; exit 1; }
sleep 0.3
id="$(window_of)"
screencapture -x -o -l "$id" "$out"
cp "$log" "$out.log"
echo "$out"
