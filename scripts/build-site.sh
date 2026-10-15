#!/bin/bash
# Puts Null's website together in OUT (target/site if not given): site/, the README's
# pictures, the app's icon, its own fonts (served from the site: no font service is
# contacted), and the film's engine (scripts/promo) that the story section plays as you
# scroll. The GitHub workflow publishes what this makes; look at it locally with
#   python3 -m http.server --directory target/site
set -euo pipefail
repo="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$repo/target/site}"
case "$out" in site|_site|*/site|*/_site) ;; *) echo "build-site: OUT should end in /site or /_site" >&2; exit 1 ;; esac
rm -rf "$out"
mkdir -p "$out/images" "$out/fonts" "$out/film/scripts/promo" "$out/film/assets/fonts"
cp -R "$repo/site/." "$out/"
cp "$repo"/docs/images/*.png "$out/images/"
cp "$repo/packaging/null.svg" "$out/icon.svg"
for f in instrument-sans/InstrumentSans-Regular.ttf instrument-sans/InstrumentSans-Medium.ttf \
    instrument-sans/InstrumentSans-SemiBold.ttf geist-mono/GeistMono-Regular.ttf geist-mono/GeistMono-Medium.ttf; do
    cp "$repo/assets/fonts/$f" "$out/fonts/"
done
cp "$repo/scripts/promo/promo.html" "$out/film/scripts/promo/"
cp -R "$repo/scripts/promo/shots" "$out/film/scripts/promo/"
cp -R "$repo/assets/fonts/instrument-sans" "$repo/assets/fonts/geist-mono" "$out/film/assets/fonts/"
echo "$out"
