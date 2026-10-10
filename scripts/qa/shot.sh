#!/bin/bash
# Looks at Null as it is: runs the debug build on PROJECT with the steps in STEPS (see
# src/qa.rs), takes a picture of its window when the steps say `ready`, and quits it.
# It runs with a home of its own (settings, sessions, history): yours are never touched.
#
#   scripts/qa/shot.sh STEPS OUT.png [PROJECT] [WAIT_SECONDS]
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
printf '{ "welcomed": true }\n' > "$home/.config/null/settings.json"
log="$home/qa.log"
trap 'kill "$pid" 2>/dev/null || true; rm -rf "$home"' EXIT

HOME="$home" XDG_CONFIG_HOME= XDG_DATA_HOME= NULL_DATA_DIR="$home/data" NULL_QA="$steps" \
    NULL_QA_FRONT="${NULL_QA_FRONT:-1}" \
    "$bin" "$project" > "$log" 2>&1 &
pid=$!

for _ in $(seq $((wait_for * 10))); do
    grep -q NULL_QA_READY "$log" && break
    kill -0 "$pid" 2>/dev/null || { cat "$log" >&2; echo "null quit before it was ready" >&2; exit 1; }
    sleep 0.1
done
grep -q NULL_QA_READY "$log" || { cat "$log" >&2; echo "not ready after ${wait_for}s" >&2; exit 1; }
sleep 0.3
id="$("$winid" "$pid")"
screencapture -x -o -l "$id" "$out"
cp "$log" "$out.log"
echo "$out"
