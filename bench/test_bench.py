import contextlib
import io
import json
import tempfile
import unittest
from unittest import mock
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


def step(input, output, read=0, reasoning=0, cost=0, ts=0):
    return json.dumps({"type": "step_finish", "timestamp": ts, "part": {"type": "step-finish", "cost": cost,
        "tokens": {"input": input, "output": output, "reasoning": reasoning, "cache": {"read": read, "write": 0}}}})


def tool(call, name, **input):
    return json.dumps({"type": "tool_use", "part": {"type": "tool", "callID": call, "tool": name,
                                                    "state": {"status": "completed", "input": input}}})


class ShTest(unittest.TestCase):
    def test_pwd_follows_cwd(self):
        with tempfile.TemporaryDirectory() as d:
            r = bench.sh(["sh", "-c", "echo $PWD"], d, env={"PATH": "/usr/bin:/bin", "PWD": "/elsewhere"})
            self.assertEqual(r.stdout.strip(), d)
            self.assertEqual(bench.sh(["sh", "-c", "echo $PWD"], d).stdout.strip(), d)


class RepoGuardTest(unittest.TestCase):
    def test_changed_paths(self):
        before = (" M a.py\n", 1, ["HEAD", "config"])
        self.assertEqual(bench.changed_paths(before, before), [])
        after = (" M a.py\n?? b.gleam\n", 2, ["HEAD", "config", "opencode"])
        self.assertEqual(bench.changed_paths(before, after), ["?? b.gleam", ".solace/store.db", ".git/opencode"])

    def test_repo_state_ignores_results(self):
        state = bench.repo_state()
        self.assertNotIn("bench/results", state[0])
        self.assertIn("HEAD", state[2])


def pi_call(input, output, read=0, calls=(), stop="toolUse", ts=0):
    content = [{"type": "toolCall", "id": f"c{i}", "name": n, "arguments": a} for i, (n, a) in enumerate(calls)]
    return json.dumps({"type": "message_end", "message": {"role": "assistant", "content": content, "stopReason": stop,
                       "timestamp": ts, "usage": {"input": input, "output": output, "cacheRead": read, "cacheWrite": 0}}})


class PiTest(unittest.TestCase):
    def test_parse(self):
        user = json.dumps({"type": "message_end", "message": {"role": "user", "content": "task", "timestamp": 0}})
        lines = [user, pi_call(100, 10, calls=[("mcp__solace__solace", {"command": "view x"})], ts=1000),
                 pi_call(50, 20, read=100, calls=[("mcp__solace__solace", {"command": "submit"})], ts=4000),
                 pi_call(10, 5, read=170, stop="stop", ts=5000), "not json"]
        m = bench.parse_pi(lines)
        self.assertEqual(m["tokens"], 110 + 170 + 185)
        self.assertEqual(m["uncached_tokens"], 110 + 70 + 15)
        self.assertEqual(m["work_tokens"], 465 - 300)
        self.assertEqual((m["tool_calls"], m["turns"]), (2, 3))
        self.assertEqual((m["tokens_before_first_edit"], m["tool_calls_before_first_edit"], m["turns_to_first_edit"]), (100, 1, 2))
        self.assertEqual(m["duration_ms"], 4000)
        self.assertFalse(m["agent_error"])

    def test_failures(self):
        self.assertTrue(bench.parse_pi([])["agent_error"])
        self.assertTrue(bench.parse_pi([pi_call(1, 1, stop="error")])["agent_error"])

    def test_home(self):
        with tempfile.TemporaryDirectory() as d:
            bench.pi_home(Path(d), "m", "http://127.0.0.1:1/v1", Path("/w"), Path("/bin"))
            solace = json.loads((Path(d) / "mcp.json").read_text())["mcpServers"]["solace"]
            self.assertEqual((solace["cwd"], solace["exposure"]), ("/w", "direct"))
            model = json.loads((Path(d) / "models.json").read_text())["providers"]["local"]["models"][0]
            self.assertEqual(model, {"id": "m"})  # server unreachable: no limits


class OpencodeTest(unittest.TestCase):
    def test_parse_files_arm(self):
        lines = [tool("1", "read", filePath="src/a.gleam"), step(100, 10, ts=1000),
                 tool("2", "edit", filePath="src/a.gleam"), tool("2", "edit"),  # repeated event, one call
                 step(50, 20, read=100, reasoning=5, cost=0.5, ts=4000), "not json"]
        m = bench.parse_opencode(lines)
        self.assertEqual(m["tokens"], 110 + 175)
        self.assertEqual(m["uncached_tokens"], 110 + 75)
        self.assertEqual(m["output_tokens"], 35)
        self.assertEqual(m["work_tokens"], 285 - 200)
        self.assertEqual((m["tool_calls"], m["turns"]), (2, 2))
        self.assertEqual((m["tokens_before_first_edit"], m["tool_calls_before_first_edit"], m["turns_to_first_edit"]), (100, 1, 2))
        self.assertEqual(m["duration_ms"], 3000)
        self.assertEqual(m["cost_usd"], 0.5)
        self.assertFalse(m["agent_error"])

    def test_parse_solace_submit_and_failures(self):
        m = bench.parse_opencode([tool("1", "solace_solace", command="view x\nsubmit"), step(10, 1)])
        self.assertEqual(m["turns_to_first_edit"], 1)
        self.assertIsNone(m["cost_usd"])
        empty = bench.parse_opencode([])
        self.assertTrue(empty["agent_error"])
        self.assertIsNone(empty["turns_to_first_edit"])
        self.assertIsNone(empty["duration_ms"])
        self.assertTrue(bench.parse_opencode([step(1, 1), json.dumps({"type": "error"})])["agent_error"])

    def test_config_arms(self):
        files = bench.opencode_config("files", "m", "http://127.0.0.1:1/v1", Path("/bin"))
        self.assertEqual(files["model"], "local/m")
        self.assertTrue(files["tools"]["edit"] and not files["tools"]["webfetch"])
        self.assertEqual(files["permission"]["bash"]["*"], "deny")
        self.assertNotIn("mcp", files)
        self.assertNotIn("limit", files["provider"]["local"]["models"]["m"])  # server unreachable
        solace = bench.opencode_config("solace", "m", "http://127.0.0.1:1/v1", Path("/bin"))
        self.assertFalse(any(solace["tools"].values()))
        self.assertEqual(solace["mcp"]["solace"]["command"][0], "/bin/solace")
        self.assertEqual(solace["permission"]["solace_*"], "allow")
        self.assertEqual(files["permission"]["external_directory"], "deny")
        self.assertEqual(solace["permission"]["external_directory"], "deny")

    def test_config_reads_context_window(self):
        reply = io.BytesIO(json.dumps({"data": [{"meta": {"n_ctx": 80000}}]}).encode())
        with mock.patch("urllib.request.urlopen", return_value=contextlib.nullcontext(reply)):
            c = bench.opencode_config("files", "m", "http://h/v1/", Path("/bin"))
        self.assertEqual(c["provider"]["local"]["models"]["m"]["limit"], {"context": 80000, "output": 20000})


class CompareTest(unittest.TestCase):
    def run_dir(self, root, name, agent, tasks):
        d = Path(root) / name / "files"
        d.mkdir(parents=True)
        rows = [{"task": t, "rep": 1, "agent": agent, "model": "m", "arm": "files", "hidden_pass": t != "L02",
                 "tokens": 100, "work_tokens": 50, "turns": 3, "commands_per_call": 1.5} for t in tasks]
        (d / "summary.jsonl").write_text("".join(json.dumps(r) + "\n" for r in rows))
        return d.parent

    def test_report_compares_harnesses(self):
        with tempfile.TemporaryDirectory() as root:
            a = self.run_dir(root, "1", "claude", ["L01", "L02"])
            b = self.run_dir(root, "2", "opencode", ["L01"])
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                bench.report(a, b)
            text = out.getvalue()
            self.assertIn("compare only the shared ones: L01", text)
            self.assertRegex(text, r"claude\s+m\s+files\s+1/2")
            self.assertRegex(text, r"opencode\s+m\s+files\s+1/1")
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                bench.report(a)  # one setup: no comparison table
            self.assertNotIn("tok/pass", out.getvalue())
            self.assertRegex(out.getvalue(), r"cmd/call\n(.*\n)*ALL .* 1\.5\n(.*\n)*SUM .* -\n")


def claude_call(mid, calls):
    content = [{"type": "tool_use", "id": f"{mid}.{i}", "name": n, "input": {"command": c}} for i, (n, c) in enumerate(calls)]
    return json.dumps({"type": "assistant", "message": {"id": mid, "content": content, "usage": {"input_tokens": 10}}})


class ClaudeStreamTest(unittest.TestCase):
    def test_commands_per_call(self):
        lines = [claude_call("m1", [("mcp__solace__solace", "session start --task\n"),
                                    ("mcp__solace__solace", "inspect a\n\nview b\nread c")]),
                 claude_call("m2", [("Read", ""), ("mcp__solace__solace", "submit\ntest")])]
        m = bench.parse_stream(lines)
        self.assertEqual(m["tool_calls"], 4)
        self.assertEqual(m["commands_per_call"], 1.5)  # 6 commands over 4 calls
        self.assertIsNone(bench.parse_stream([claude_call("m1", [("Read", "")])])["commands_per_call"])


if __name__ == "__main__":
    unittest.main()
