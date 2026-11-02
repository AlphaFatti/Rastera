import unittest
from audio_native_probe import check_decode, check_resampler

class NativeAudioAssertions(unittest.TestCase):
    def test_channel_swap_is_rejected(self):
        rows = [{'samples': 288000, 'timestamp_gaps': [], 'seek_probe': {'first_pts': 2.25},
                 'channels': [{'frequency_amplitude': [0, .1, 0, 0], 'rms': .07, 'first_nonzero_pts': 1/48000},
                              {'frequency_amplitude': [.1, 0, 0, 0], 'rms': .07, 'first_nonzero_pts': 1/48000}]}]
        with self.assertRaises(AssertionError):
            check_decode('2ch.mov', rows)

    def test_alias_and_impulse_timing_are_required(self):
        rows = {'in_band': {'errors': 0, 'tone_amplitude': .1},
                'above_nyquist': {'errors': 0, 'rms': .05},
                'impulse': {'errors': 0, 'peak_frame': 20000, 'expected_impulse_frame': 22050}}
        with self.assertRaises(AssertionError):
            check_resampler(rows)
        rows['above_nyquist']['rms'] = .000001
        with self.assertRaises(AssertionError):
            check_resampler(rows)
        rows['impulse']['peak_frame'] = 22050
        check_resampler(rows)
