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
#   scripts/trailer/render.sh short       the 15-second cut, 4K and 1080p
#   scripts/trailer/render.sh vertical    standing up (1080×1920), the whole trailer and its 15-second cut
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
# The 15-second cut's music: the same bars as its pictures (trailer.html, CUTS), on the track's
# own clock (the full cut's 16.05 s and later sit 7.99 s further in the track).
short="8.062-10.060@0,12.058-16.054@1.998,40.028-42.026@5.994,44.024-46.022@7.992,56.012-61.010@9.990"
out="$repo/target/trailer"
mkdir -p "$out"
case "${1:-}" in
    "")
        "$bin" "$page" "$out/pictures-4k.mp4" --fps 60 --size 3840x2160 --also 1920x1080 "$out/pictures-1080p.mp4" --poster 51.6 "$out/null-trailer-poster.jpg"
        "$score" "$here/music.mp3" "$out/pictures-4k.mp4" "$out/null-trailer-4k.mp4" "$out/pictures-4k.sounds.json" --pieces "$pieces"
        "$score" "$here/music.mp3" "$out/pictures-1080p.mp4" "$out/null-trailer-1080p.mp4" "$out/pictures-4k.sounds.json" --pieces "$pieces"
        rm -f "$out"/pictures-*.mp4
        ;;
    short)
        "$bin" "$page" "$out/pictures-short-4k.mp4" --query "cut=15" --fps 60 --size 3840x2160 --also 1920x1080 "$out/pictures-short-1080p.mp4" --poster 13.5 "$out/null-trailer-15s-poster.jpg"
        "$score" "$here/music.mp3" "$out/pictures-short-4k.mp4" "$out/null-trailer-15s-4k.mp4" "$out/pictures-short-4k.sounds.json" --pieces "$short"
        "$score" "$here/music.mp3" "$out/pictures-short-1080p.mp4" "$out/null-trailer-15s-1080p.mp4" "$out/pictures-short-4k.sounds.json" --pieces "$short"
        rm -f "$out"/pictures-short-*
        ;;
    vertical)
        "$bin" "$page" "$out/pictures-vertical.mp4" --query "vertical" --page 1080x1920 --fps 60 --size 1080x1920 --poster 51.6 "$out/null-trailer-vertical-poster.jpg"
        "$score" "$here/music.mp3" "$out/pictures-vertical.mp4" "$out/null-trailer-vertical.mp4" "$out/pictures-vertical.sounds.json" --pieces "$pieces"
        "$bin" "$page" "$out/pictures-vertical-15s.mp4" --query "vertical&cut=15" --page 1080x1920 --fps 60 --size 1080x1920
        "$score" "$here/music.mp3" "$out/pictures-vertical-15s.mp4" "$out/null-trailer-vertical-15s.mp4" "$out/pictures-vertical-15s.sounds.json" --pieces "$short"
        rm -f "$out"/pictures-vertical*
        ;;
    preview)
        "$bin" "$page" "$out/pictures-preview.mp4" --fps 30 --size 1920x1080 --bitrate 6
        "$score" "$here/music.mp3" "$out/pictures-preview.mp4" "$out/null-trailer-preview.mp4" "$out/pictures-preview.sounds.json" --pieces "$pieces"
        rm -f "$out/pictures-preview.mp4"
        ;;
    web|clips)
        # The website's: the trailer with its sound, its card, and a silent loop for each
        # thing the page shows (the trailer's own moments, without its words).
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
