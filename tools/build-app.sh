#!/bin/sh
# One release build/packager, optionally followed by a verified installed copy.
set -eu
rastera_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$rastera_root"
"${CARGO:-cargo}" build --release --locked -p rastera-desktop --bin canvas-editor
exec python3 "$rastera_root/tools/package_app.py" "$@"
