#!/bin/bash
# Records Null's trailer (scripts/trailer/trailer.html): the Mac's own WebKit draws each
# frame, its video encoder writes them, then score.swift lays the music (music.mp3, "Two AM
# Window", made with Gemini) under it, with small sounds from the trailer's own cues. The
# music is cut once: bars 8 to 11 of its opening groove go, so it runs 51 seconds. Nothing
# to install. Open trailer.html in a browser to watch it as you change it (Space pauses,
# ← → step, ?t=24.5 opens it at a moment).
#
#   scripts/trailer/render.sh             the trailer, 4K and 1080p, in target/trailer
#   scripts/trailer/render.sh preview     a quick 1080p copy at 30 frames a second
#   scripts/trailer/render.sh web         the website's copy (site/media/null-trailer.mp4) and its poster
#   scripts/trailer/render.sh 12 30.5     pictures of those moments, in target/trailer-stills
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
bin="$repo/target/promo-render"
score="$repo/target/promo-score"
mkdir -p "$repo/target"
if [ ! -x "$bin" ] || [ "$repo/scripts/promo/render.swift" -nt "$bin" ]; then
    swiftc -O -swift-version 5 -o "$bin" "$repo/scripts/promo/render.swift"
fi
if [ ! -x "$score" ] || [ "$repo/scripts/promo/score.swift" -nt "$score" ]; then
    swiftc -O -swift-version 5 -o "$score" "$repo/scripts/promo/score.swift"
fi
page="$here/trailer.html"
# The music without bars 8 to 11 (16.05 s to 24.04 s of the track): one downbeat to another.
pieces="0-16.05@0,24.04-61.2@16.05"
out="$repo/target/trailer"
mkdir -p "$out"
case "${1:-}" in
    "")
        "$bin" "$page" "$out/pictures-4k.mp4" --fps 60 --size 3840x2160 --also 1920x1080 "$out/pictures-1080p.mp4" --poster 51.6 "$out/null-trailer-poster.jpg"
        "$score" "$here/music.mp3" "$out/pictures-4k.mp4" "$out/null-trailer-4k.mp4" "$out/pictures-4k.sounds.json" --pieces "$pieces"
        "$score" "$here/music.mp3" "$out/pictures-1080p.mp4" "$out/null-trailer-1080p.mp4" "$out/pictures-4k.sounds.json" --pieces "$pieces"
        rm -f "$out"/pictures-*.mp4
        ;;
    preview)
        "$bin" "$page" "$out/pictures-preview.mp4" --fps 30 --size 1920x1080 --bitrate 6
        "$score" "$here/music.mp3" "$out/pictures-preview.mp4" "$out/null-trailer-preview.mp4" "$out/pictures-preview.sounds.json" --pieces "$pieces"
        rm -f "$out/pictures-preview.mp4"
        ;;
    web)
        media="$repo/site/media"
        "$bin" "$page" "$out/pictures-web.mp4" --fps 30 --size 1920x1080 --bitrate 1.5 --poster 51.6 "$media/null-trailer-poster.jpg"
        "$score" "$here/music.mp3" "$out/pictures-web.mp4" "$media/null-trailer.mp4" "$out/pictures-web.sounds.json" --pieces "$pieces"
        rm -f "$out/pictures-web.mp4"
        ;;
    *)
        "$bin" "$page" "$repo/target/trailer-stills" --stills "$(IFS=,; echo "$*")"
        echo "$repo/target/trailer-stills"
        exit
        ;;
esac
ls -la "$out"
