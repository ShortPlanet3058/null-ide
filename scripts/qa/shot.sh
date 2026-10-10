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
# history), only the environment it needs (no API keys, logs or shell setup of yours;
# your tools and installed language servers on its PATH, only used), a clipboard of its
# own, your keychain never read or changed, nothing installed, Rust built into its home
# offline, and a copy of PROJECT (without target/ and node_modules/), or an empty one,
# so nothing it does changes your files. What it can't keep apart is the keyboard: the
# window comes to the front for the few seconds it runs (macOS doesn't redraw a covered
# window), and keys typed then go to it. (NULL_QA_FRONT=0 leaves it behind, showing its
# first frame.) A program run in its terminal is yours to choose: `run pbcopy` would
# reach the Mac's clipboard.
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

if [ -n "${3:-}" ] && [ ! -d "$3" ]; then
    echo "no folder $3" >&2
    exit 1
fi
home="$(mktemp -d)"
mkdir -p "$home/.config/null" "$home/tmp" "$home/ack" "$home/project"
# A copy, named as the original (its name shows), so the steps change nothing of yours.
if [ -n "${3:-}" ]; then
    project="$home/project/$(basename "$(cd "$3" && pwd)")"
    rsync -a --exclude target/ --exclude node_modules/ "$(cd "$3" && pwd)/" "$project/"
else
    project="$home/project/project"
    mkdir -p "$project"
fi
# (Welcomed already, unless the settings given say otherwise.)
case "${NULL_QA_SETTINGS:-}" in
    *'"welcomed"'*) printf '{ %s }\n' "$NULL_QA_SETTINGS" ;;
    *) printf '{ "welcomed": true%s }\n' "${NULL_QA_SETTINGS:+, $NULL_QA_SETTINGS}" ;;
esac > "$home/.config/null/settings.json"
log="$home/qa.log"
pid=""
finish() {
    if [ -n "$pid" ]; then
        # All it started (its group: language servers and what they run) and its shell,
        # then it; gone before its home is.
        pkill -TERM -P "$pid" 2>/dev/null || true
        kill -TERM -- "-$pid" 2>/dev/null || kill "$pid" 2>/dev/null || true
        wait "$pid" 2>/dev/null || true
        sleep 0.5
    fi
    rm -rf "$home" 2>/dev/null || { sleep 1; rm -rf "$home"; }
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

# Your tools and the language servers Null installed for you, to be found (not changed).
real_home="$HOME"
tools="$real_home/.cargo/bin:$real_home/.local/bin:$real_home/go/bin:/opt/homebrew/bin:/usr/local/bin"
servers="$real_home/Library/Application Support/Null/servers"
tools="$tools:$servers/node_modules/.bin:$servers/bin"
cd "$project"
# (Its own process group, so all it starts goes with it.)
set -m
env -i \
    PATH="$PATH:$tools" USER="${USER:-}" LOGNAME="${LOGNAME:-}" SHELL="${SHELL:-/bin/zsh}" \
    LANG="${LANG:-en_US.UTF-8}" TERM=xterm-256color TMPDIR="$home/tmp/" \
    HOME="$home" CFFIXED_USER_HOME="$home" \
    RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \
    CARGO_TARGET_DIR="$home/target" CARGO_NET_OFFLINE=true \
    NULL_DATA_DIR="$home/data" NULL_QA="$steps" NULL_QA_FRONT="${NULL_QA_FRONT:-1}" NULL_QA_ACK="$home/ack" \
    "$bin" "$project" > "$log" 2>&1 &
pid=$!
set +m

# `shot <name>` steps: a picture each, beside OUT.png, as they come.
taken=0
take_shots() {
    local names
    names="$( (grep '^NULL_QA_SHOT ' "$log" || true) | sed 's/^NULL_QA_SHOT //' | tail -n +$((taken + 1)))"
    [ -z "$names" ] && return 0
    while IFS= read -r name; do
        local id file
        # A plain name, beside OUT.png (not a path, not hidden).
        file="$(printf '%s' "$name" | tr -c 'A-Za-z0-9._-' '_' | sed 's/^[._]*//')"
        file="${file:-shot}"
        id="$(window_of)"
        screencapture -x -o -l "$id" "$(dirname "$out")/$file.png"
        echo "$(dirname "$out")/$file.png"
        taken=$((taken + 1))
        # Taken: the steps go on.
        touch "$home/ack/$taken.done"
    done <<< "$names"
}
deadline=$((SECONDS + wait_for))
while [ "$SECONDS" -lt "$deadline" ]; do
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
