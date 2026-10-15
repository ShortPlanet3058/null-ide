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
#   scripts/trailer/render.sh web         the website's videos and pictures (site/media)
#   scripts/trailer/render.sh clips       only the website's silent loops, not its trailer
#   scripts/trailer/render.sh 12 30.5     pictures of those moments, in target/trailer-stills
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
bin="$repo/target/promo-render"
score="$repo/target/promo-score"
mkdir -p "$repo/target"
if [ ! -x "$bin" ] || [ "$repo/scripts/trailer/render.swift" -nt "$bin" ]; then
    swiftc -O -swift-version 5 -o "$bin" "$repo/scripts/trailer/render.swift"
fi
if [ ! -x "$score" ] || [ "$repo/scripts/trailer/score.swift" -nt "$score" ]; then
    swiftc -O -swift-version 5 -o "$score" "$repo/scripts/trailer/score.swift"
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
    web|clips)
        # The website's: the trailer with its sound, the hero's moving background, and a
        # silent loop for each thing it shows (the trailer's own moments, without its words).
        media="$repo/site/media"
        mkdir -p "$media"
        if [ "$1" = web ]; then
            "$bin" "$page" "$out/pictures-web.mp4" --fps 30 --size 1920x1080 --bitrate 2 --poster 51.6 "$media/null-trailer-poster.jpg"
            "$score" "$here/music.mp3" "$out/pictures-web.mp4" "$media/null-trailer.mp4" "$out/pictures-web.sounds.json" --pieces "$pieces"
            rm -f "$out/pictures-web.mp4" "$out/pictures-web.sounds.json"
            # The trailer's card on the page: its nine scenes at once.
            "$bin" "$page" "$out/card" --stills 49.6 --size 1920x1080
            sips -s format jpeg -s formatOptions 82 "$out/card/49.60.png" --out "$media/trailer-card.jpg" >/dev/null
            rm -rf "$out/card"
        fi
        "$bin" "$page" "$media/hero.mp4" --query "clip=hero" --fps 30 --size 1600x900 --bitrate 1.6 --poster 1.5 "$media/hero.jpg"
        for clip in calm fast precise run commit write ai; do
            "$bin" "$page" "$media/$clip.mp4" --query "clip=$clip" --fps 30 --size 1280x720 --bitrate 1.4 --poster 1.8 "$media/$clip.jpg"
        done
        rm -f "$media"/*.sounds.json
        ls -la "$media"
        exit
        ;;
    *)
        "$bin" "$page" "$repo/target/trailer-stills" --stills "$(IFS=,; echo "$*")"
        echo "$repo/target/trailer-stills"
        exit
        ;;
esac
ls -la "$out"
