#!/bin/bash
# Films Null's promotional film (scripts/promo/promo.html) into site/media/null-film.mp4,
# with its poster: the Mac's own WebKit draws each frame, its video encoder writes them,
# then score.swift lays the music (music.mp3, made with Google Flow) under it, cut on its
# bars, with small sounds made from the film's own cues. Nothing to install; about half an
# hour (4K at 60 frames a second). The films go to target/film. Open promo.html in a browser to watch it as you
# change it (Space pauses, ← → step, ?t=24.5 opens it at a moment).
#
#   scripts/promo/render.sh            the film
#   scripts/promo/render.sh 12 30.5    pictures of those moments, in target/promo-stills
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
bin="$repo/target/promo-render"
score="$repo/target/promo-score"
mkdir -p "$repo/target"
if [ ! -x "$bin" ] || [ "$here/render.swift" -nt "$bin" ]; then
    swiftc -O -swift-version 5 -o "$bin" "$here/render.swift"
fi
if [ ! -x "$score" ] || [ "$here/score.swift" -nt "$score" ]; then
    swiftc -O -swift-version 5 -o "$score" "$here/score.swift"
fi
if [ $# -gt 0 ]; then
    stills="$(IFS=,; echo "$*")"
    "$bin" "$here/promo.html" "$repo/target/promo-stills" --stills "$stills"
    echo "$repo/target/promo-stills"
    exit
fi
# The master in 4K at 60 frames a second, and a 1080p copy to share, both with the sound.
out="$repo/target/film"
mkdir -p "$out"
"$bin" "$here/promo.html" "$out/pictures-4k.mp4" --fps 60 --size 3840x2160 --also 1920x1080 "$out/pictures-1080p.mp4" --poster 102 "$out/null-film-poster.jpg"
"$score" "$here/music.mp3" "$out/pictures-4k.mp4" "$out/null-film-4k.mp4" "$out/pictures-4k.sounds.json"
"$score" "$here/music.mp3" "$out/pictures-1080p.mp4" "$out/null-film-1080p.mp4" "$out/pictures-4k.sounds.json"
rm -f "$out/pictures-4k.mp4" "$out/pictures-1080p.mp4"
ls -la "$out"
