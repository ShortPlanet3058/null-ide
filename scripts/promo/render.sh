#!/bin/bash
# Films Null's promotional film (scripts/promo/promo.html) into site/media/null-film.mp4,
# with its poster: the Mac's own WebKit draws each frame, its video encoder writes them,
# then score.swift lays the music (music.mp3, made with Google Flow) under it, cut on its
# bars, with small sounds made from the film's own cues. Nothing to install; about half an
# hour (4K at 60 frames a second). The films go to target/film. Open promo.html in a browser to watch it as you
# change it (Space pauses, ← → step, ?t=24.5 opens it at a moment).
#
#   scripts/promo/render.sh            the film
#   scripts/promo/render.sh 30         the 30-second cut, wide and vertical
#   scripts/promo/render.sh web        the website's videos and pictures (site/media)
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
if [ $# -gt 0 ] && [ "$1" != "30" ] && [ "$1" != "web" ]; then
    stills="$(IFS=,; echo "$*")"
    "$bin" "$here/promo.html" "$repo/target/promo-stills" --stills "$stills"
    echo "$repo/target/promo-stills"
    exit
fi
out="$repo/target/film"
mkdir -p "$out"
if [ "${1:-}" = "web" ]; then
    # The website's: four silent background scenes (small: they sit behind words), the
    # film itself at 1080p with its sound, and pictures for the cards.
    media="$repo/site/media"
    mkdir -p "$media/stills"
    for scene in calm fast ai everywhere; do
        "$bin" "$here/promo.html" "$media/$scene.mp4" --query "cut=$scene&nocaptions&nokeys" --fps 30 --size 1920x1080 --bitrate 2.5 --poster 0.6 "$media/$scene.jpg"
        rm -f "$media/$scene.sounds.json"
    done
    "$bin" "$here/promo.html" "$out/pictures-web.mp4" --fps 30 --size 1920x1080 --bitrate 1.5 --poster 70.2 "$media/null-film-poster.jpg"
    "$score" "$here/music.mp3" "$out/pictures-web.mp4" "$media/null-film.mp4" "$out/pictures-web.sounds.json"
    rm -f "$out/pictures-web.mp4"
    "$bin" "$here/promo.html" "$out/stills" --query "nocaptions&nokeys" --stills 37.6,41.2,51.6,70.2,78.2,82.9,84.5,87.7
    i=0
    for name in write find run ai commit tasks debug folders; do
        i=$((i + 1))
        file=$(ls "$out/stills" | sed -n "${i}p")
        sips -s format jpeg -s formatOptions 82 -Z 1600 "$out/stills/$file" --out "$media/stills/$name.jpg" >/dev/null
    done
    for name in md vim keyboard themes; do
        sips -s format jpeg -s formatOptions 82 -Z 1600 "$here/shots/$name.jpg" --out "$media/stills/$name.jpg" >/dev/null
    done
    rm -rf "$out/stills"
    ls -la "$media" "$media/stills"
    exit
fi
if [ "${1:-}" = "30" ]; then
    # The 30-second cut, wide (4K and 1080p) and vertical for phones (1080x1920), its music
    # cut on the bars: the build from a bar, the silence shortened, the big beat, five bars
    # of groove, the last phrase.
    pieces="24.06-29.4@0,31.6-49.66@5.34,136.06-145.4@23.4"
    "$bin" "$here/promo.html" "$out/pictures-30-4k.mp4" --query "cut=30" --fps 60 --size 3840x2160 --also 1920x1080 "$out/pictures-30-1080p.mp4"
    "$bin" "$here/promo.html" "$out/pictures-30-vertical.mp4" --query "cut=30&vertical" --page 1080x1920 --fps 60 --size 1080x1920
    "$score" "$here/music.mp3" "$out/pictures-30-4k.mp4" "$out/null-film-30s-4k.mp4" "$out/pictures-30-4k.sounds.json" --pieces "$pieces"
    "$score" "$here/music.mp3" "$out/pictures-30-1080p.mp4" "$out/null-film-30s-1080p.mp4" "$out/pictures-30-4k.sounds.json" --pieces "$pieces"
    "$score" "$here/music.mp3" "$out/pictures-30-vertical.mp4" "$out/null-film-30s-vertical.mp4" "$out/pictures-30-vertical.sounds.json" --pieces "$pieces"
    rm -f "$out"/pictures-30-*.mp4
    ls -la "$out"
    exit
fi
# The master in 4K at 60 frames a second, and a 1080p copy to share, both with the sound.
"$bin" "$here/promo.html" "$out/pictures-4k.mp4" --fps 60 --size 3840x2160 --also 1920x1080 "$out/pictures-1080p.mp4" --poster 102 "$out/null-film-poster.jpg"
"$score" "$here/music.mp3" "$out/pictures-4k.mp4" "$out/null-film-4k.mp4" "$out/pictures-4k.sounds.json"
"$score" "$here/music.mp3" "$out/pictures-1080p.mp4" "$out/null-film-1080p.mp4" "$out/pictures-4k.sounds.json"
rm -f "$out/pictures-4k.mp4" "$out/pictures-1080p.mp4"
ls -la "$out"
