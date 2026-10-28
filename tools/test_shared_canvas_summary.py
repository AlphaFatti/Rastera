import json
from pathlib import Path
import tempfile
import unittest
from summarize_shared_canvas import summarize


class SharedEvidenceTests(unittest.TestCase):
    def collect(self, presentations):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'test.log'
            lines=['shared_producer host_s=10 t_us=0 completed=2 skipped=0 leased_slots=1',
                   'shared_interval role=left max_inflight=1 busy=0 errors=0 lost=0',
                   'shared_shutdown outstanding_slots=0 capacity=4',
                   'shared_ui_gap host_s=10.6 gap_us=500000']
            for seq,frame in [(1,17),(2,18)]:
                for kind in ['commit','complete','schedule']:
                    lines.append(f'shared_event role=left kind={kind} seq={seq} frame={frame} callback_host_s=10.{seq} gpu_or_presented_host_s=0')
            path.write_text('\n'.join(lines+presentations))
            path.with_suffix('.events.jsonl').write_text('\n'.join(json.dumps(r) for r in [dict(event='measure_start',host_s=10),dict(event='measure_end',host_s=11)]))
            return summarize(path)['roles']['left']

    def test_completion_and_zero_timestamp_do_not_prove_presentation(self):
        r=self.collect(['shared_event role=left kind=presented seq=1 frame=17 callback_host_s=10.8 gpu_or_presented_host_s=0'])
        self.assertEqual(r['missing_completions'],[])
        self.assertEqual(r['missing_presented_handlers'],[('2','18')])
        self.assertEqual(len(r['without_positive_presentation']),2)
        self.assertEqual(r['zero_callbacks_whole'],1)
        self.assertIsNone(r['presentation_gap_ms'])

    def test_actual_presentation_times_and_source_ids_are_distinct_from_arrival(self):
        r=self.collect(['shared_event role=left kind=presented seq=1 frame=17 callback_host_s=10.9 gpu_or_presented_host_s=10.15',
                        'shared_event role=left kind=presented seq=2 frame=18 callback_host_s=10.3 gpu_or_presented_host_s=10.17'])
        self.assertEqual(r['presentation_gap_ms'],20)
        self.assertEqual(r['during_ui_gaps'][0]['source_frame_range'],[17,18])
        self.assertEqual(r['during_ui_gaps'][0]['positive_presentation_reports'],2)
