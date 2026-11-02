#!/usr/bin/env python3
"""Digital native-decoder/SRC regression checks. No audio device starts without --devices."""
import argparse
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]

def check_decode(name, data):
    expected = [0, 1] if name in ('2ch.mov', 'stereo.wav', 'stereo.aiff') else [0, 1, 2, 3]
    observed = []
    offsets = []
    for track in data:
        assert track['samples'] > 280000, (name, track['samples'])
        assert not track['timestamp_gaps'], (name, track['timestamp_gaps'])
        assert abs(track['seek_probe']['first_pts'] - 2.25) < 2 / 48000, (name, track['seek_probe'])
        for channel in track['channels']:
            amplitude = channel['frequency_amplitude']
            dominant = max(range(4), key=amplitude.__getitem__)
            assert amplitude[dominant] > .085, (name, amplitude)
            assert max(a for i, a in enumerate(amplitude) if i != dominant) < .00015, (name, amplitude)
            assert .065 < channel['rms'] < .072, (name, channel['rms'])
            assert channel['first_nonzero_pts'] is not None
            observed.append(dominant)
            offsets.append(channel['first_nonzero_pts'])
    assert observed == expected, (name, observed)
    delay = {'dual-stereo.mov': [0, 0, .125, .125], 'quad-mono.mov': [0, .125, .25, .375]}.get(name, [0] * len(expected))
    for actual, target in zip(offsets, delay):
        assert abs(actual - target) <= 2 / 48000, (name, offsets, delay)
    return {'channels_in_order': observed, 'first_nonzero_pts': offsets, 'track_count': len(data)}

def check_resampler(data):
    assert all(row['errors'] == 0 for row in data.values()), data
    assert abs(data['in_band']['tone_amplitude'] - .1) < .001, data
    assert data['above_nyquist']['rms'] < .00001, data
    impulse = data['impulse']
    assert abs(impulse['peak_frame'] - impulse['expected_impulse_frame']) <= 1, data


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, default=ROOT/'target/release/audio-probe')
    parser.add_argument('--out', type=Path, default=Path('/tmp/rastera-audio-native-reviewed'))
    parser.add_argument('--fixtures', type=Path, default=Path('/tmp/rastera-audio-fixtures'))
    parser.add_argument('--devices', help='Explicit comma-separated output UIDs; starts quiet output tests')
    parser.add_argument('--seconds', type=float, default=12)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    if not (args.fixtures/'expected.json').exists():
        subprocess.run(['python3', str(ROOT/'tools/generate_audio_fixtures.py'), str(args.fixtures)], check=True)
    def run(*options):
        result = subprocess.run([str(args.binary.resolve()), *options], cwd='/', check=True, capture_output=True, text=True)
        return json.loads(result.stdout)
    results = {'physical_listening': 'pending', 'physical_AV_offset': 'pending', 'decodes': {}}
    for name in ['2ch.mov', '4ch.mov', 'dual-stereo.mov', 'quad-mono.mov', 'stereo.wav', 'stereo.aiff']:
        data = run('--decode='+str((args.fixtures/name).resolve()))
        (args.out/(name+'.json')).write_text(json.dumps(data, indent=2)+'\n')
        results['decodes'][name] = check_decode(name, data)
    results['resampler_44100'] = run('--resampler-check=44100')
    check_resampler(results['resampler_44100'])
    if args.devices:
        for name in ['stereo.wav', 'stereo.aiff', '4ch.mov']:
            data = run('--devices='+args.devices, '--seconds='+str(args.seconds), '--file='+str((args.fixtures/name).resolve()), '--no-synthetic')
            assert data['nonzero_mixed_frames'] > 10000, data
            assert data['file']['decode_errors'] == 0, data
            for device in data['devices']:
                assert device['details']['content_frames'] > 10000, device
                assert device['details']['output_peak'] > 0.0001, device
                assert device['details']['converter_errors'] == 0, device
            results[name+'_outputs'] = data
        data = run('--devices='+args.devices, '--seconds='+str(args.seconds), '--file='+str((args.fixtures/'4ch.mov').resolve()))
        assert all(max(bus) > 0.0001 for bus in data['bus_rms']), data['bus_rms']
        results['ten_buses_and_file'] = data
    (args.out/'results.json').write_text(json.dumps(results, indent=2)+'\n')
    print(json.dumps({'passed': True, 'artifacts': str(args.out), 'hardware_tested': bool(args.devices)}, indent=2))

if __name__ == '__main__':
    main()
