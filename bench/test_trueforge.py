import io
import json
import sys
import threading
import unittest
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from bench import bench, trueforge


def message(input, output, cache=0, calls=(), at="2026-10-15T10:00:00Z"):
    tool_calls = [{"id": f"c{i}", "type": "function", "function": {"name": n, "arguments": json.dumps(a)},
                   "tool_info": {"type": "truefoundry-system", "name": n}} for i, (n, a) in enumerate(calls)]
    return json.dumps({"type": "model.message", "created_at": at, "tool_calls": tool_calls,
                       "usage": {"input_tokens": input, "output_tokens": output, "cache_read_tokens": cache}})


class ParseTest(unittest.TestCase):
    def test_files_arm(self):
        lines = [json.dumps({"type": "turn.created", "created_at": "2026-10-15T09:59:59Z"}),
                 message(100, 10, calls=[("exec", {"command": "cat src/a.gleam"})]),
                 json.dumps({"type": "model.message.delta"}),  # no usage: not a model call
                 message(150, 20, cache=100, calls=[("exec", {"command": "sed -i 's/a/b/' src/a.gleam"})],
                         at="2026-10-15T10:00:03Z"),
                 json.dumps({"type": "turn.done", "state": {"status": "done"}}), "not json"]
        m = bench.parse_trueforge(lines)
        self.assertEqual(m["tokens"], 280)
        self.assertEqual(m["uncached_tokens"], 180)
        self.assertEqual(m["work_tokens"], 280 - 200)
        self.assertEqual((m["tool_calls"], m["turns"]), (2, 2))
        self.assertEqual((m["tokens_before_first_edit"], m["tool_calls_before_first_edit"], m["turns_to_first_edit"]), (100, 1, 2))
        self.assertEqual(m["duration_ms"], 4000)
        self.assertFalse(m["agent_error"])

    def test_failures(self):
        self.assertTrue(bench.parse_trueforge([])["agent_error"])
        failed = bench.parse_trueforge([message(1, 1), json.dumps({"type": "turn.done", "state": {"status": "failed"}})])
        self.assertTrue(failed["agent_error"])
        self.assertIsNone(failed["turns_to_first_edit"])


class EditTest(unittest.TestCase):
    def test_solace(self):
        self.assertTrue(bench.tf_edit("solace", json.dumps({"command": "view x\nsubmit", "text": "code"})))
        self.assertFalse(bench.tf_edit("solace", json.dumps({"command": "view x"})))
        deferred = {"mcp_server": "solace", "tool_name": "solace", "input": {"command": "delete pilot/x.f"}}
        self.assertTrue(bench.tf_edit("call_tool", json.dumps(deferred)))

    def test_exec(self):
        self.assertTrue(bench.tf_edit("exec", json.dumps({"command": "echo x > src/a.gleam"})))
        self.assertTrue(bench.tf_edit("exec", json.dumps({"command": "python3 -c \"open('src/a.gleam', 'w')\""})))
        self.assertFalse(bench.tf_edit("exec", json.dumps({"command": "gleam test"})))
        self.assertFalse(bench.tf_edit("exec", "not json"))


class ProtocolTest(unittest.TestCase):
    def test_slug_and_spec(self):
        self.assertEqual(trueforge.slug("Swift-1.5_IQ2_XS"), "swift-1-5-iq2-xs")
        files, solace = trueforge.spec("files", "local/m"), trueforge.spec("solace", "local/m")
        self.assertTrue(files["config"]["sandbox"]["enabled"])
        self.assertNotIn("mcp_servers", files)
        self.assertFalse(solace["config"]["sandbox"]["enabled"])
        self.assertEqual(solace["mcp_servers"][0]["require_approval_for_tools"], [])
        self.assertFalse(any(solace["config"][k]["enabled"] for k in ("dynamic_sub_agents", "web_search", "ask_user_questions")))

    def test_sse(self):
        stream = io.BytesIO(b'data: {"type": "a"}\r\n\r\n: comment\n\ndata: {"data": {"type": "b"}}\n\ndata: broken\n\n')
        self.assertEqual(list(trueforge.sse(stream)), [{"type": "a"}, {"data": {"type": "b"}}])

    def test_resume(self):
        actions = [{"type": "tool.approval_required", "thread_id": "main", "tool_calls": [{"id": "a"}, {"id": "b"}]},
                   {"type": "tool.response_required", "thread_id": "t2", "tool_calls": [{"id": "q"}]},
                   {"type": "mcp.auth_required"}]
        out = trueforge.resume(actions)
        self.assertEqual([(o["type"], o["tool_call_id"]) for o in out],
                         [("user.tool_approval", "a"), ("user.tool_approval", "b"), ("user.tool_response", "q")])
        self.assertEqual(out[0]["approval"], {"status": "allow"})
        self.assertEqual(trueforge.resume([]), [])


ECHO = ("import json, sys\n"
        "for line in sys.stdin:\n"
        "    m = json.loads(line)\n"
        "    if 'id' in m:\n"
        "        print(json.dumps({'id': m['id'], 'result': m.get('params')}), flush=True)\n")


def post(url, body):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=10) as r:
        return r.status, r.read().decode()


class BridgeTest(unittest.TestCase):
    def test_requests_and_notifications(self):
        bridge = trueforge.Bridge([sys.executable, "-c", ECHO], None, None)
        try:
            status, body = post(bridge.url, {"jsonrpc": "2.0", "id": 7, "method": "x", "params": {"text": "a\nb"}})
            self.assertEqual((status, json.loads(body)), (200, {"id": 7, "result": {"text": "a\nb"}}))
            self.assertEqual(post(bridge.url, {"jsonrpc": "2.0", "method": "notifications/initialized"}), (202, ""))
            with self.assertRaises(urllib.error.HTTPError) as e:
                urllib.request.urlopen(bridge.url, timeout=10)
            self.assertEqual(e.exception.code, 405)
        finally:
            bridge.close()


class GateTest(unittest.TestCase):
    def test_holds_until_open(self):
        class Upstream(BaseHTTPRequestHandler):
            def do_POST(self):
                body = self.rfile.read(int(self.headers["content-length"]))
                self.send_response(200)
                self.send_header("content-type", "text/plain")
                self.end_headers()
                self.wfile.write(self.path.encode() + b" " + body)

            def log_message(self, *_):
                pass
        upstream = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
        threading.Thread(target=upstream.serve_forever, daemon=True).start()
        gate = trueforge.Gate(f"http://127.0.0.1:{upstream.server_port}/v1")
        try:
            gate.open.clear()
            got = []
            t = threading.Thread(target=lambda: got.append(post(gate.url + "/chat/completions", {"a": 1})))
            t.start()
            t.join(0.3)
            self.assertEqual(got, [])  # held
            gate.open.set()
            t.join(10)
            self.assertEqual(got, [(200, '/v1/chat/completions {"a": 1}')])
        finally:
            gate.close()
            upstream.shutdown()
            upstream.server_close()


if __name__ == "__main__":
    unittest.main()
