import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from bench import bench


class ReportTest(unittest.TestCase):
    def test_next_run_number(self):
        with tempfile.TemporaryDirectory() as directory:
            results = Path(directory)
            self.assertEqual(bench.next_run_number(results), 1)
            for name in ("1", "3", "20260928-gpt-6-sol-low"):
                (results / name).mkdir()
            (results / "99").write_text("not a run directory")
            self.assertEqual(bench.next_run_number(results), 4)

    def test_next_rep(self):
        with tempfile.TemporaryDirectory() as directory:
            scenario = Path(directory)
            self.assertEqual(bench.next_rep(scenario, "L01"), 1)
            rows = [{"task": "L01", "rep": 1}, {"task": "L02", "rep": 4}, {"task": "L01", "rep": 2}]
            (scenario / "summary.jsonl").write_text("\n".join(map(json.dumps, rows)) + "\n")
            self.assertEqual(bench.next_rep(scenario, "L01"), 3)
            self.assertEqual(bench.next_rep(scenario, "L02"), 5)

    def test_scenario_summaries(self):
        with tempfile.TemporaryDirectory() as directory:
            run_dir = Path(directory)
            for name in ("files", "solace"):
                scenario = run_dir / name
                scenario.mkdir()
                row = {"task": "L01", "rep": 1, "arm": name, "hidden_pass": True, "tokens": 10, "cost_usd": 0}
                (scenario / "summary.jsonl").write_text(json.dumps(row) + "\n")
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                bench.report(run_dir)
            self.assertIn("L01", output.getvalue())
            self.assertIn("2/2", output.getvalue())


if __name__ == "__main__":
    unittest.main()
