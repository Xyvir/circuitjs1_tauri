#!/usr/bin/env bash
# Stage the compiled GWT site into app/www, which tauri.conf.json serves as the
# frontend. This mirrors what the Electron build did by copying site/ into app/war.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(dirname "$DIR")"
SRC="$ROOT/site"
DEST="$DIR/www"

if [ ! -f "$SRC/circuitjs.html" ]; then
    echo "No compiled frontend at $SRC/circuitjs.html" >&2
    echo "Build it first:  ./gradlew makeSite" >&2
    exit 1
fi

rm -rf "$DEST"
mkdir -p "$DEST"
cp -r "$SRC"/. "$DEST"/

# Tauri resolves frontendDist to a directory whose entry point is index.html.
# The application page is circuitjs.html, so ship the forwarding stub with it.
cp "$DIR/src-tauri/entry.html" "$DEST/index.html"

echo "Staged frontend: $DEST"
