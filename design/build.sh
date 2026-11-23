#!/bin/sh
# Stitch a round's variant fragments into one flick-through page.
# usage: build.sh <round-dir> <output.html> <title>
set -eu
cd "$(dirname "$0")"
round=$1 out=$2 title=$3
cat shell-head.html logos.html "$round"/*.html shell-tail.html \
    | sed "s|@TITLE@|$title|g; s|@KEY@|view-$round|g" > "$out"
echo "wrote $(pwd)/$out ($(wc -c < "$out") bytes)"
