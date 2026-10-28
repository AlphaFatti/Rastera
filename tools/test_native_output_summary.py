"""Focused semantic checks: callback arrival must never replace presentedTime."""
import json
from pathlib import Path
import tempfile
import unittest
from summarize_native_output import summarize


class PresentationSummaryTests(unittest.TestCase):
    def collect(self, handlers, lost=0):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/"native-idle.log"
            path.write_text("\n".join([
                "clock_anchor mono_us=0 unix_us=1000000",
                "native_clock_anchor host_s=10.0 unix_us=1000000",
                "viewport name=operator max_gap_start_us=50000 max_gap_us=600000",
                "native_interval host_s=11.0 max_inflight=2 lost_events="+str(lost)+" errors=0 zero_time=1 unavailable_time=0 backpressure=0",
            ]+handlers))
            path.with_suffix(".events.jsonl").write_text("\n".join(json.dumps(r) for r in [
                dict(event="measure_start", unix_us=1000000),
                dict(event="measure_end", unix_us=2000000),
            ]))
            return summarize(path)

    def test_sorts_real_times_and_rejects_zero_instead_of_using_arrival(self):
        result = self.collect([
            "native_event kind=presented_handler seq=2 callback_host_s=10.130 presented_host_s=10.116 timestamp_valid=1",
            "native_event kind=presented_handler seq=1 callback_host_s=10.900 presented_host_s=10.100 timestamp_valid=1",
            "native_event kind=presented_handler seq=3 callback_host_s=10.800 presented_host_s=0 timestamp_valid=0",
            "native_event kind=presented_handler seq=4 callback_host_s=10.700 presented_host_s=nan timestamp_valid=0",
        ])
        self.assertAlmostEqual(result["presentation_gap_ms"], 16.0)
        self.assertGreater(result["presented_callback_gap_ms"], 500)
        self.assertEqual(result["zero_timestamp_callbacks_in_interval"], 1)
        self.assertEqual(result["valid_presentation_samples"], 2)
        self.assertEqual(result["operator_pauses"][0]["presentation_samples_during_gap"], 2)

    def test_unavailable_is_not_reported_as_zero_gap(self):
        result = self.collect([
            "native_event kind=presented_handler seq=1 callback_host_s=10.100 presented_host_s=0 timestamp_valid=0",
        ])
        self.assertIsNone(result["presentation_gap_ms"])
        self.assertEqual(result["presentation_status"], "unavailable")

    def test_event_loss_marks_trace_incomplete(self):
        result = self.collect([], lost=1)
        self.assertEqual(result["presentation_status"], "incomplete trace: events lost")


if __name__ == "__main__":
    unittest.main()
