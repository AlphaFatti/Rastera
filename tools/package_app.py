#!/usr/bin/env python3
"""Single build-app.sh packager: validated app, transactional install or tester DMG."""
import argparse
import hashlib
import json
import tempfile
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
    assert info['CFBundleIdentifier'] == BUNDLE_ID and info['NSCameraUsageDescription'] and info['NSMicrophoneUsageDescription']
    assert info['CFBundleExecutable'] == 'Rastera'
    binary = bundle/'Contents/MacOS/Rastera'
    if info['CFBundleShortVersionString'] != version().split('-')[0] or info['CFBundleVersion'] != version().split('-')[0]:
        raise RuntimeError('Bundle and workspace versions differ')
    minimum = deployment_target(binary)
    if normalized_version(minimum) != normalized_version(info['LSMinimumSystemVersion']) or normalized_version(minimum) != (14,):
        raise RuntimeError(f'Executable/bundle minimum OS mismatch: {minimum} vs {info["LSMinimumSystemVersion"]}')
    build_info = json.loads((bundle/'Contents/Resources/build-info.json').read_text())
    if (build_info['source_commit'] != info['RasteraSourceRevision'] or build_info['version'] != version()
            or build_info['source_dirty'] != info['RasteraSourceDirty'] or build_info['signing'] != info['RasteraSigningMode']):
        raise RuntimeError('Build provenance/version mismatch')
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
    signature = subprocess.run(['codesign', '-d', '--verbose=4', str(bundle)], capture_output=True, text=True, check=True)
    if not re.search(r'flags=.*\bruntime\b', signature.stderr):
        raise RuntimeError('Application is missing hardened runtime')
    entitlements = plistlib.loads(subprocess.check_output(['codesign', '-d', '--entitlements', ':-', str(bundle)], stderr=subprocess.DEVNULL))
    assert entitlements.get('com.apple.security.device.camera') is True
    assert entitlements.get('com.apple.security.device.audio-input') is True
    print(f'Verified arm64, system dependencies, metadata, camera/audio-input entitlements and signature: {bundle}')


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


def version():
    return re.search(r'^version = "([^"]+)"', (ROOT/'Cargo.toml').read_text(), re.M)[1]


def source_metadata(require_clean=False):
    revision = subprocess.check_output(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], text=True).strip()
    dirty = bool(subprocess.check_output(['git', '-C', str(ROOT), 'status', '--porcelain'], text=True).strip())
    if require_clean and dirty:
        raise RuntimeError('DMG distribution requires a clean committed release revision')
    return {'source_commit': revision, 'source_dirty': dirty, 'version': version(),
            'architecture': 'arm64', 'minimum_macos': '14.0'}


def deployment_target(binary):
    output = subprocess.check_output(['xcrun', 'vtool', '-show-build', str(binary)], text=True)
    match = re.search(r'^\s*minos\s+([0-9.]+)', output, re.M)
    if not match or not re.search(r'^\s*platform\s+MACOS\s*$', output, re.M):
        raise RuntimeError('Executable has no macOS LC_BUILD_VERSION deployment target')
    return match[1]


def normalized_version(value):
    parts = [int(part) for part in value.split('.')]
    while parts and parts[-1] == 0:
        parts.pop()
    return tuple(parts)


def distribution_signing(mode):
    output = subprocess.check_output(['security', 'find-identity', '-v', '-p', 'codesigning'], text=True)
    identities = [(digest, label) for digest, label in re.findall(r'([0-9A-Fa-f]{40}) "([^"]+)"', output)
                  if label.startswith('Developer ID Application:')]
    requested = os.environ.get('RASTERA_DISTRIBUTION_IDENTITY')
    profile = os.environ.get('RASTERA_NOTARY_PROFILE')
    matches = [(digest, label) for digest, label in identities if not requested or requested in (digest, label)]
    ready = len(matches) == 1 and bool(profile)
    if mode == 'developer-id' and not ready:
        raise RuntimeError('Developer ID distribution requires one valid RASTERA_DISTRIBUTION_IDENTITY and RASTERA_NOTARY_PROFILE (keychain profile); no credentials are embedded')
    if mode != 'adhoc' and ready:
        return matches[0][0], profile, 'developer-id-notarized'
    return '-', None, 'ad-hoc-unnotarized'


def sign_app(bundle, identity):
    command = ['codesign', '--force', '--sign', identity, '--options', 'runtime', '--entitlements', ROOT/'tools/macos/camera.entitlements.plist']
    if identity == '-':
        command += ['--requirements', f'=designated => identifier "{BUNDLE_ID}"']
    else:
        command += ['--timestamp']
    run(*command, bundle)


def notarize(path, profile, output_dir, label, staple=True):
    result = subprocess.run(['xcrun', 'notarytool', 'submit', str(path), '--keychain-profile', profile,
                             '--wait', '--output-format', 'json'], capture_output=True, text=True)
    try:
        receipt = json.loads(result.stdout)
    except ValueError as error:
        raise RuntimeError('Notarytool did not return a result; check the configured keychain profile/network') from error
    (output_dir/f'{label}.notary-result.json').write_text(json.dumps(receipt, indent=2)+'\n')
    if receipt.get('id'):
        log = subprocess.run(['xcrun', 'notarytool', 'log', receipt['id'], '--keychain-profile', profile], capture_output=True, text=True)
        (output_dir/f'{label}.notary-log.json').write_text(log.stdout)
    if result.returncode or receipt.get('status') != 'Accepted':
        raise RuntimeError(f'Notarization was not accepted; inspect {label}.notary-result.json and log')
    if staple:
        run('xcrun', 'stapler', 'staple', path)
        run('xcrun', 'stapler', 'validate', path)


def render_guide(metadata):
    text = (ROOT/'docs/tester-guide-template.md').read_text()
    if metadata['signing'] == 'ad-hoc-unnotarized':
        signing = 'Ad-hoc signed locally. NOT Developer ID signed and NOT notarized. Strict codesign integrity passes; Gatekeeper acceptance is not claimed.'
        opening = ('macOS may block this downloaded app because Apple cannot verify its developer/notarization. '
                   'Only if you trust this exact package and its checksum: first try opening it, then open System Settings > Privacy & Security, '
                   'find the Rastera blocked-app entry, choose Open Anyway, and confirm Open. This is a per-application exception; '
                   'managed Macs may disallow it. Do not disable Gatekeeper or change global security settings. '
                   'If macOS reports damage or malware, stop and send the exact message to the developer.')
    else:
        signing = 'Developer ID Application signed, hardened runtime with secure timestamp, notarized by Apple and stapled. Gatekeeper assessment passes for this delivered build.'
        opening = 'On first launch, macOS may ask you to confirm opening a downloaded application. Check its source and choose Open if you trust it.'
    replacements = {'VERSION': metadata['version'], 'SOURCE_COMMIT': metadata['source_commit'],
                    'SIGNING': signing, 'OPENING': opening}
    for key, value in replacements.items():
        text = text.replace('{{'+key+'}}', value)
    if '{{' in text:
        raise RuntimeError('Unresolved tester-guide template field')
    return text


def validate_payload(folder):
    if {entry.name for entry in folder.iterdir()} != {'Rastera.app', 'Applications', 'Tester Guide.txt'}:
        raise RuntimeError('Unexpected DMG payload; personal/configuration files must never be shipped')
    if not (folder/'Applications').is_symlink() or os.readlink(folder/'Applications') != '/Applications':
        raise RuntimeError('Missing Applications-folder shortcut')
    allowed = {'Contents/Info.plist', 'Contents/MacOS/Rastera', 'Contents/Resources/THIRD_PARTY_NOTICES.md',
               'Contents/Resources/build-info.json', 'Contents/_CodeSignature/CodeResources'}
    for item in (folder/'Rastera.app').rglob('*'):
        if item.is_symlink() or item.name == '.DS_Store' or item.name.startswith('._'):
            raise RuntimeError('Symlink or machine-specific file inside application payload')
        if item.is_dir():
            continue
        relative = item.relative_to(folder/'Rastera.app').as_posix()
        if relative not in allowed and not relative.startswith('Contents/Resources/docs/licenses/'):
            raise RuntimeError(f'Unexpected app payload: {relative}')


def assessment(path, kind):
    command = ['spctl', '--assess', '--type', kind, '--verbose=4']
    if kind == 'open':
        command += ['--context', 'context:primary-signature']
    result = subprocess.run(command+[str(path)], capture_output=True, text=True)
    return {'exit_code': result.returncode, 'output': (result.stdout+result.stderr).strip().replace(str(path), path.name)}


def checksums(paths):
    lines = []
    for path in paths:
        digest = hashlib.sha256()
        with path.open('rb') as file:
            for block in iter(lambda: file.read(1024*1024), b''):
                digest.update(block)
        lines.append(f'{digest.hexdigest()}  {path.name}')
    return '\n'.join(lines)+'\n'


def make_dmg(bundle, output_dir, metadata, identity, profile):
    output_dir = output_dir.expanduser().absolute()
    output_dir.mkdir(parents=True, exist_ok=True)
    stem = f'Rastera-{metadata["version"]}-macOS-arm64'
    dmg = output_dir/f'{stem}.dmg'
    guide = output_dir/f'Rastera-{metadata["version"]}-Tester-Guide.txt'
    if dmg.exists():
        raise RuntimeError(f'DMG already exists; choose a fresh --dist-dir: {dmg}')
    if profile:
        archive = output_dir/f'.{stem}.notary-app.zip'
        run('ditto', '-c', '-k', '--keepParent', bundle, archive)
        notarize(archive, profile, output_dir, 'application', staple=False)
        # ZIP tickets cannot be stapled. Staple the enclosed app after Accepted instead.
        run('xcrun', 'stapler', 'staple', bundle)
        run('xcrun', 'stapler', 'validate', bundle)
        archive.unlink()
    guide.write_text(render_guide(metadata))
    with tempfile.TemporaryDirectory(prefix='rastera-dmg-', dir=ROOT/'target') as temporary:
        folder = Path(temporary)/'payload'
        folder.mkdir()
        run('ditto', bundle, folder/'Rastera.app')
        (folder/'Applications').symlink_to('/Applications', target_is_directory=True)
        shutil.copy2(guide, folder/'Tester Guide.txt')
        validate_payload(folder)
        run('hdiutil', 'create', '-volname', f'Rastera {metadata["version"]}', '-srcfolder', folder,
            '-fs', 'HFS+', '-format', 'UDZO', '-imagekey', 'zlib-level=9', dmg)
    command = ['codesign', '--force', '--sign', identity]
    if identity != '-':
        command += ['--timestamp']
    run(*command, dmg)
    if profile:
        notarize(dmg, profile, output_dir, 'disk-image')
    run('codesign', '--verify', '--strict', dmg)
    run('hdiutil', 'verify', dmg)
    host = subprocess.run(['spctl', '--status'], capture_output=True, text=True)
    host_status = (host.stdout+host.stderr).strip()
    app_assessment, dmg_assessment = assessment(bundle, 'execute'), assessment(dmg, 'open')
    if profile and (host_status != 'assessments enabled' or app_assessment['exit_code'] or dmg_assessment['exit_code']):
        raise RuntimeError('Notarized distribution failed final Gatekeeper assessment')
    report = dict(metadata, gatekeeper_host_status=host_status,
                  gatekeeper_enforced_acceptance_verified=host_status == 'assessments enabled' and bool(profile),
                  bundle_gatekeeper=app_assessment, dmg_gatekeeper=dmg_assessment,
                  disk_image_format='UDZO compressed read-only', demo_included=False)
    info = output_dir/f'{stem}-Build-Info.json'
    info.write_text(json.dumps(report, indent=2)+'\n')
    (output_dir/'SHA256SUMS').write_text(checksums([dmg, guide, info]))
    print(f'Tester package: {dmg}\nGuide: {guide}\nChecksums: {output_dir/"SHA256SUMS"}\nSigning: {metadata["signing"]}')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--install', action='store_true')
    parser.add_argument('--dmg', action='store_true', help='Create a tester distribution from a clean revision')
    parser.add_argument('--dist-dir', type=Path, default=ROOT/'target/dist')
    parser.add_argument('--distribution-signing', choices=['auto','adhoc','developer-id'], default='auto')
    parser.add_argument('--destination', type=Path, help='Applications directory or complete Rastera.app path; implies --install')
    args = parser.parse_args()
    if args.dmg and (args.install or args.destination):
        raise RuntimeError('--dmg preserves the installed copy; use --install separately')
    metadata = source_metadata(require_clean=args.dmg)
    if args.dmg and args.dist_dir.expanduser().absolute().is_relative_to(ROOT) and not args.dist_dir.expanduser().absolute().is_relative_to(ROOT/'target'):
        raise RuntimeError('Repository distributions must be under ignored target/; external directories are also supported')
    if args.dmg:
        identity, profile, signing = distribution_signing(args.distribution_signing)
    else:
        identity, profile = os.environ.get('RASTERA_SIGN_IDENTITY', '-'), None
        signing = 'ad-hoc-local-development' if identity == '-' else 'configured-local-signing'
    metadata['signing'] = signing
    target = ROOT/'target/Rastera.app'
    staged = ROOT/f'target/.Rastera-stage-{os.getpid()}.app'
    if staged.exists():
        raise RuntimeError(f'Staging path already exists: {staged}')
    (staged/'Contents/MacOS').mkdir(parents=True)
    resources = staged/'Contents/Resources'
    resources.mkdir()
    shutil.copy2(ROOT/'target/release/canvas-editor', staged/'Contents/MacOS/Rastera')
    shutil.copy2(ROOT/'THIRD_PARTY_NOTICES.md', resources)
    shutil.copytree(ROOT/'docs/licenses', resources/'docs/licenses', ignore=shutil.ignore_patterns('.DS_Store', '._*', '__pycache__', '.env*'))
    app_version = version()
    info = dict(CFBundleIdentifier=BUNDLE_ID, CFBundleName='Rastera', CFBundleDisplayName='Rastera',
                CFBundleExecutable='Rastera', CFBundlePackageType='APPL', CFBundleShortVersionString=app_version.split('-')[0],
                CFBundleVersion=app_version.split('-')[0], RasteraDevelopmentVersion=app_version, RasteraSourceRevision=metadata['source_commit'], RasteraSourceDirty=metadata['source_dirty'], RasteraSigningMode=signing, LSMinimumSystemVersion='14.0',
                NSHighResolutionCapable=True, NSCameraUseContinuityCameraDeviceType=True,
                NSCameraUsageDescription='Rastera uses the camera or capture device you explicitly select as a live canvas source.',
                NSMicrophoneUsageDescription='Rastera captures audio only from the input device and channels you explicitly enable in its mixer. No microphone is automatically associated with a camera.')
    (staged/'Contents/Info.plist').write_bytes(plistlib.dumps(info))
    (resources/'build-info.json').write_text(json.dumps(metadata, indent=2)+'\n')
    sign_app(staged, identity)
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
    elif args.dmg:
        make_dmg(target, args.dist_dir, metadata, identity, profile)
    else:
        print(f'Development app: {target}\nInstall: ./tools/build-app.sh --install')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, AssertionError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f'Build/install failed: {error}', file=sys.stderr)
        sys.exit(1)
