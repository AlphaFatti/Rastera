#!/usr/bin/env python3
"""Single build-app.sh packager: staged validation, then optional transactional installation."""
import argparse
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
BUNDLE_ID = 'org.rastera.desktop.dev'


def run(*args, **kwargs):
    return subprocess.run([str(x) for x in args], check=True, **kwargs)


def verify(bundle):
    info = plistlib.loads((bundle/'Contents/Info.plist').read_bytes())
    assert info['CFBundleIdentifier'] == BUNDLE_ID and info['NSCameraUsageDescription']
    assert info['CFBundleExecutable'] == 'Rastera'
    binary = bundle/'Contents/MacOS/Rastera'
    arches = subprocess.check_output(['lipo', '-archs', str(binary)], text=True).strip()
    if arches != 'arm64':
        raise RuntimeError(f'Expected Apple silicon arm64, got {arches}')
    for line in subprocess.check_output(['otool', '-L', str(binary)], text=True).splitlines()[1:]:
        dependency = line.strip().split(' (', 1)[0]
        if not dependency.startswith(('/System/Library/', '/usr/lib/')):
            raise RuntimeError(f'Unexpected non-system dependency: {dependency}')
    for resource in ['THIRD_PARTY_NOTICES.md', 'docs/licenses/fonts/OFL.txt', 'docs/licenses/media/gif-MIT.txt']:
        if not (bundle/'Contents/Resources'/resource).is_file():
            raise RuntimeError(f'Missing bundled notice: {resource}')
    run('plutil', '-lint', bundle/'Contents/Info.plist')
    run('codesign', '--verify', '--deep', '--strict', bundle)
    entitlements = plistlib.loads(subprocess.check_output(['codesign', '-d', '--entitlements', ':-', str(bundle)], stderr=subprocess.DEVNULL))
    assert entitlements.get('com.apple.security.device.camera') is True
    assert not entitlements.get('com.apple.security.device.audio-input', False)
    print(f'Verified arm64, system dependencies, metadata, camera entitlement and signature: {bundle}')


def replace(staged, destination):
    backup = destination.with_name(f'.Rastera.previous-{os.getpid()}.app')
    if backup.exists():
        raise RuntimeError(f'Backup already exists: {backup}')
    existed = destination.exists()
    if existed:
        if destination.is_symlink():
            raise RuntimeError(f'Refusing to replace a symlink: {destination}')
        info = plistlib.loads((destination/'Contents/Info.plist').read_bytes())
        if info.get('CFBundleIdentifier') != BUNDLE_ID:
            raise RuntimeError(f'Refusing to replace a different application: {destination}')
        destination.rename(backup)
    try:
        staged.rename(destination)
        verify(destination)
    except Exception:
        if destination.exists():
            destination.rename(staged)
        if existed:
            backup.rename(destination)
        raise
    if existed:
        shutil.rmtree(backup)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--install', action='store_true')
    parser.add_argument('--destination', type=Path, help='Applications directory or complete Rastera.app path; implies --install')
    args = parser.parse_args()
    target = ROOT/'target/Rastera.app'
    staged = ROOT/f'target/.Rastera-stage-{os.getpid()}.app'
    if staged.exists():
        raise RuntimeError(f'Staging path already exists: {staged}')
    (staged/'Contents/MacOS').mkdir(parents=True)
    resources = staged/'Contents/Resources'
    resources.mkdir()
    shutil.copy2(ROOT/'target/release/canvas-editor', staged/'Contents/MacOS/Rastera')
    shutil.copy2(ROOT/'THIRD_PARTY_NOTICES.md', resources)
    shutil.copytree(ROOT/'docs/licenses', resources/'docs/licenses')
    version = re.search(r'^version = "([^"]+)"', (ROOT/'Cargo.toml').read_text(), re.M)[1]
    info = dict(CFBundleIdentifier=BUNDLE_ID, CFBundleName='Rastera', CFBundleDisplayName='Rastera',
                CFBundleExecutable='Rastera', CFBundlePackageType='APPL', CFBundleShortVersionString=version.split('-')[0],
                CFBundleVersion=version.split('-')[0], RasteraDevelopmentVersion=version, LSMinimumSystemVersion='14.0',
                NSHighResolutionCapable=True, NSCameraUseContinuityCameraDeviceType=True,
                NSCameraUsageDescription='Rastera uses the camera or capture device you select as a live canvas source. No audio is captured.')
    (staged/'Contents/Info.plist').write_bytes(plistlib.dumps(info))
    identity = os.environ.get('RASTERA_SIGN_IDENTITY', '-')
    command = ['codesign', '--force', '--sign', identity, '--options', 'runtime', '--entitlements', ROOT/'tools/macos/camera.entitlements.plist']
    if identity == '-':
        command += ['--requirements', f'=designated => identifier "{BUNDLE_ID}"']
    run(*command, staged)
    verify(staged)
    replace(staged, target)
    if args.install or args.destination:
        destination = args.destination
        if destination is None:
            destination = next((p for p in [Path('/Applications/Rastera.app'), Path.home()/'Applications/Rastera.app'] if p.exists()), Path.home()/'Applications/Rastera.app')
        destination = destination.expanduser().absolute()
        if destination.suffix.lower() != '.app':
            destination /= 'Rastera.app'
        if destination == target or destination.is_relative_to(ROOT):
            raise RuntimeError('Installation must be a real copy outside the repository')
        destination.parent.mkdir(parents=True, exist_ok=True)
        incoming = destination.with_name(f'.Rastera-install-{os.getpid()}.app')
        if incoming.exists():
            raise RuntimeError(f'Install staging path already exists: {incoming}')
        run('ditto', target, incoming)
        verify(incoming)
        helper = ROOT/'target/tools/rastera-app-control'
        helper.parent.mkdir(parents=True, exist_ok=True)
        run('xcrun', 'clang', ROOT/'tools/native_app_control.m', '-framework', 'Cocoa', '-framework', 'ApplicationServices', '-o', helper)
        run(helper, 'quit')
        replace(incoming, destination)
        register = '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister'
        run(register, '-f', destination)
        print(f'Installed real copy: {destination}\nLaunch: open "{destination}"')
    else:
        print(f'Development app: {target}\nInstall: ./tools/build-app.sh --install')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, AssertionError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f'Build/install failed: {error}', file=sys.stderr)
        sys.exit(1)
