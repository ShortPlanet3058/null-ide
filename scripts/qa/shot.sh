#!/bin/bash
# Looks at Null as it is: runs the debug build on PROJECT with the steps in STEPS (see
# src/qa.rs), takes a picture of its window when the steps say `ready`, and quits it.
# It runs with a home of its own (settings, sessions, history): yours are never touched.
#
#   scripts/qa/shot.sh STEPS OUT.png [PROJECT] [WAIT_SECONDS]
#
# `shot NAME` steps take more pictures on the way, as NAME.png beside OUT.png.
#
# NULL_QA_SETTINGS='"keymap": "vim", "word_wrap": true' adds settings to its own.
#
# Needs `cargo build` first (target/debug/null). The window comes to the front for the
# few seconds it runs: macOS doesn't redraw a window that's covered, so one left behind
# would show its first frame only. (NULL_QA_FRONT=0 leaves it behind.)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
steps="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
out="$2"
project="${3:-$repo}"
wait_for="${4:-30}"
bin="$repo/target/debug/null"
[ -x "$bin" ] || { echo "no $bin: run cargo build first" >&2; exit 1; }

# The window finder, built once (kept next to the build, which git leaves out).
winid="$repo/target/qa-winid"
if [ ! -x "$winid" ] || [ "$here/winid.swift" -nt "$winid" ]; then
    swiftc -O -o "$winid" "$here/winid.swift"
fi

home="$(mktemp -d)"
mkdir -p "$home/.config/null"
printf '{ "welcomed": true%s }\n' "${NULL_QA_SETTINGS:+, $NULL_QA_SETTINGS}" > "$home/.config/null/settings.json"
log="$home/qa.log"
trap 'kill "$pid" 2>/dev/null || true; rm -rf "$home"' EXIT

# (Rust's tools still find their toolchains, in the real home: only read.)
RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" \
HOME="$home" XDG_CONFIG_HOME= XDG_DATA_HOME= NULL_DATA_DIR="$home/data" NULL_QA="$steps" \
    NULL_QA_FRONT="${NULL_QA_FRONT:-1}" \
    "$bin" "$project" > "$log" 2>&1 &
pid=$!

# `shot <name>` steps: a picture each, beside OUT.png, as they come.
taken=0
take_shots() {
    local names
    names="$( (grep '^NULL_QA_SHOT ' "$log" || true) | sed 's/^NULL_QA_SHOT //' | tail -n +$((taken + 1)))"
    [ -z "$names" ] && return 0
    while IFS= read -r name; do
        screencapture -x -o -l "$("$winid" "$pid")" "$(dirname "$out")/$name.png"
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
id="$("$winid" "$pid")"
screencapture -x -o -l "$id" "$out"
cp "$log" "$out.log"
echo "$out"
