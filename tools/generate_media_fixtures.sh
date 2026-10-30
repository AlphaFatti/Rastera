#!/bin/sh
# Synthetic, portable test media; ffmpeg is fixture tooling, never an app runtime dependency.
set -eu
rastera_fixture_dir=${1:-/tmp/rastera-media-fixtures}
mkdir -p "$rastera_fixture_dir"
ffmpeg -v error -y -f lavfi -i testsrc2=size=320x180:rate=30 -t 6 -an -c:v libx264 -pix_fmt yuv420p "$rastera_fixture_dir/test.mp4"
ffmpeg -v error -y -i "$rastera_fixture_dir/test.mp4" -t 2 -vf fps=5 "$rastera_fixture_dir/test.gif"
