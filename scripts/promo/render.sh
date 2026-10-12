#!/bin/bash
# Films Null's promotional film (scripts/promo/promo.html) into site/media/null-film.mp4,
# with its poster: the Mac's own WebKit draws each frame, its video encoder writes them.
# Nothing to install; about four minutes. Open promo.html in a browser to watch it as you
# change it (Space pauses, ← → step, ?t=24.5 opens it at a moment).
#
#   scripts/promo/render.sh            the film
#   scripts/promo/render.sh 12 30.5    pictures of those moments, in target/promo-stills
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
bin="$repo/target/promo-render"
if [ ! -x "$bin" ] || [ "$here/render.swift" -nt "$bin" ]; then
    mkdir -p "$repo/target"
    swiftc -O -swift-version 5 -o "$bin" "$here/render.swift"
fi
if [ $# -gt 0 ]; then
    stills="$(IFS=,; echo "$*")"
    "$bin" "$here/promo.html" "$repo/target/promo-stills" --stills "$stills"
    echo "$repo/target/promo-stills"
    exit
fi
mkdir -p "$repo/site/media"
"$bin" "$here/promo.html" "$repo/site/media/null-film.mp4" --poster 40 "$repo/site/media/null-film-poster.jpg"
ls -la "$repo/site/media"
