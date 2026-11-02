import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("analyze_profile", Path(__file__).with_name("Analyze-Profile.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ReportTests(unittest.TestCase):
    def test_memory_is_grouped_only_with_recent_view(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            def write(name, rows):
                (directory / name).write_text('\n'.join(json.dumps(row) for row in rows), encoding='utf-8')
            write('timings.jsonl', [{'name':'ui.paint', 'view':'Lyrics', 'time_unix_ms':10000, 'duration_us':1000}, {'name':'ui.paint', 'view':'Album', 'time_unix_ms':20000, 'duration_us':1000}])
            write('processes.jsonl', [{'time_unix_ms':10500, 'processes':[{'pid':1,'name':'app','private_bytes':104857600,'working_set_bytes':83886080}]}, {'time_unix_ms':15000, 'processes':[]}, {'time_unix_ms':20500, 'processes':[{'pid':1,'name':'app','private_bytes':209715200,'working_set_bytes':167772160}]}])
            write('rust-alloc.jsonl', [{'time_unix_ms':10500,'rust_live_bytes':10485760}])
            write('render-memory.jsonl', [{'kind':'render_memory','view':'Lyrics','artwork_bytes':1048576,'time_unix_ms':10500}])
            result = module.report(directory)
            self.assertIn('| Lyrics | 100.0 / 100.0 (1) | 80.0 / 80.0 (1) | 10.0 / 10.0 (1) |', result)
            self.assertIn('| Album | 200.0 / 200.0 (1) |', result)
            self.assertIn('1 process samples had no view event', result)
            self.assertIn('| Lyrics | 1 | 1.0 |', result)

    def test_view_slow_counts_and_missing_coverage(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            rows = [
                {"name": "ui.paint", "view": "Lyrics", "duration_us": 10000, "dropped_total": 2},
                {"name": "ui.paint", "view": "Lyrics", "duration_us": 20000, "dropped_total": 2},
                {"name": "render.composite", "view": "Lyrics", "duration_us": 3000, "dropped_total": 2},
            ]
            (directory / "timings.jsonl").write_text("\n".join(json.dumps(row) for row in rows), encoding="utf-8")
            result = module.report(directory)
            self.assertIn("1/2 (50.0%)", result)
            self.assertIn("| ui.paint | Lyrics | 2 |", result)
            self.assertIn("queue drops: 2", result)
            self.assertIn("Missing processes.jsonl", result)
            self.assertNotIn("no view labels", result)

    def test_older_session_explains_coverage(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            (directory / "timings.jsonl").write_text('{"name":"render.labels","duration_us":1000}\n', encoding="utf-8")
            result = module.report(directory)
            self.assertIn("combines text and pixel compositing", result)
            self.assertIn("no view labels", result)
            self.assertIn("| render.labels | unattributed |", result)

    def test_page_allocations_remain_attributed_after_navigation(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            rows = [
                {'kind':'render_memory','view':'Lyrics','page_tag':1,'time_unix_ms':1000,'page_scoped_all_live_bytes':[0,1048576,0]},
                {'kind':'render_memory','view':'Album','page_tag':2,'time_unix_ms':2000,'page_scoped_all_live_bytes':[0,524288,2097152]},
            ]
            (directory / 'render-memory.jsonl').write_text('\n'.join(json.dumps(row) for row in rows), encoding='utf-8')
            result = module.report(directory)
            self.assertIn('| Lyrics | 1.00 / 0.50 | not recorded | not recorded | not recorded |', result)
            self.assertIn('| Album | 2.00 / 2.00 | not recorded | not recorded | not recorded |', result)

    def test_worker_categories_and_unattributed_rust(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            counters = [0] * 256
            counters[1], counters[65], counters[129] = 1048576, 2097152, 3145728
            row = {'kind':'render_memory', 'view':'Lyrics', 'page_tag':1, 'time_unix_ms':1000,
                   'page_scoped_all_live_bytes':counters, 'measured_rust_live_bytes':10485760}
            (directory / 'render-memory.jsonl').write_text(json.dumps(row), encoding='utf-8')
            result = module.report(directory)
            self.assertIn('| Lyrics | 1.00 / 1.00 | 2.00 / 2.00 | 3.00 / 3.00 | 0.00 / 0.00 |', result)
            self.assertIn('approximately 4.0 MiB remains untagged', result)

    def test_go_memory_and_busy_source(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            rows = [
                {'heap_live_bytes':10485760, 'player':{'current':{'present':True,'busy':False,'encoded_capacity':4194304,'read_cache_capacity':1048576,'pcm_capacity':524288,'disk_spill':True},'next':{'present':False},'callbacks':5,'underruns':1}},
                {'heap_live_bytes':12582912, 'player':{'current':{'present':True,'busy':True,'encoded_capacity':999999999},'next':{'present':False},'callbacks':10,'underruns':2}},
            ]
            (directory / 'go-memory.jsonl').write_text('\n'.join(json.dumps(row) for row in rows), encoding='utf-8')
            result = module.report(directory)
            self.assertIn('Go live heap: median 11.0 MiB; peak 12.0 MiB', result)
            self.assertIn('peak encoded buffer capacity 4.00 MiB', result)
            self.assertIn('cumulative underruns: first 1, last 2', result)


if __name__ == "__main__":
    unittest.main()
