import unittest
from summarize_animation_captures import holds


class CaptureBoundsTests(unittest.TestCase):
    def test_acquisition_intervals_bound_identical_images(self):
        rows = [dict(file=str(i), start_us=a, end_us=b, sha256=h)
                for i,a,b,h in [(1,0,100,'x'), (2,300,400,'x'), (3,600,700,'y')]]
        result = holds(rows)
        self.assertEqual(len(result), 1)
        self.assertEqual(result[0]['request_start_span_ms'], 0.3)
        self.assertEqual(result[0]['acquisition_lower_bound_ms'], 0.2)
        self.assertIsNone(result[0]['freeze_upper_bound_ms'])

    def test_no_repeat_is_not_proof_of_no_short_pause(self):
        self.assertEqual(holds([dict(file='a', start_us=0, end_us=100, sha256='x')]), [])

    def test_different_neighbors_bound_whole_hold(self):
        rows = [dict(file=str(i), start_us=i*300, end_us=i*300+100, sha256=h)
                for i,h in enumerate(['before', 'same', 'same', 'after'])]
        result = holds(rows)[0]
        self.assertEqual(result['acquisition_lower_bound_ms'], 0.2)
        self.assertEqual(result['freeze_upper_bound_ms'], 1.0)
