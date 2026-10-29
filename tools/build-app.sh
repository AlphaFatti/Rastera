#!/bin/sh
# Reproducible local development bundle, not a notarized/distribution package.
set -eu
rastera_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$rastera_root"
"${CARGO:-cargo}" build --release --locked -p rastera-desktop --bin canvas-editor
rastera_app="$rastera_root/target/Rastera.app"
mkdir -p "$rastera_app/Contents/MacOS"
cp target/release/canvas-editor "$rastera_app/Contents/MacOS/Rastera.next"
mv "$rastera_app/Contents/MacOS/Rastera.next" "$rastera_app/Contents/MacOS/Rastera"
python3 - "$rastera_app" <<'PY'
import plistlib,sys,re
from pathlib import Path
version=re.search(r'^version = "([^"]+)"',Path('Cargo.toml').read_text(),re.M)[1]
plist=dict(CFBundleIdentifier='org.rastera.desktop.dev',CFBundleName='Rastera',CFBundleDisplayName='Rastera',CFBundleExecutable='Rastera',CFBundlePackageType='APPL',CFBundleShortVersionString=version.split('-')[0],CFBundleVersion=version.split('-')[0],RasteraDevelopmentVersion=version,LSMinimumSystemVersion='14.0',NSHighResolutionCapable=True,NSCameraUseContinuityCameraDeviceType=True,NSCameraUsageDescription='Rastera uses the camera or capture device you select as a live canvas source. No audio is captured.')
with (Path(sys.argv[1])/'Contents/Info.plist').open('wb') as f:plistlib.dump(plist,f)
PY
if [ "${RASTERA_SIGN_IDENTITY:--}" = "-" ]; then
    codesign --force --sign - --options runtime --entitlements tools/macos/camera.entitlements.plist --requirements '=designated => identifier "org.rastera.desktop.dev"' "$rastera_app"
else
    codesign --force --sign "$RASTERA_SIGN_IDENTITY" --options runtime --entitlements tools/macos/camera.entitlements.plist "$rastera_app"
fi
codesign --verify --deep --strict "$rastera_app"
printf 'Development app: %s\nRun: open "%s"\n' "$rastera_app" "$rastera_app"
