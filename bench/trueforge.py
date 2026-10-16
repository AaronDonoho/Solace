"""TrueForge as a bench harness (https://github.com/truefoundry/trueforge).

TrueForge is an agent server, not a CLI: the bench starts a private server
(its own data folder, so the user's sessions and settings stay out), registers
the model, and drives each task as one session over its HTTP API.

- files arm: the agent gets TrueForge's local sandbox (its `exec` tool). The
  sandbox is a fresh folder TrueForge makes per session, on the first `exec` or
  when a message carries a file. So the task goes in as an attached TASK.md
  too (about 40 tokens of upload notice), which makes the sandbox before the
  model is called; `Gate` holds the model's first request until the project is
  copied in. The agent's tree is copied back out for grading.
- solace arm: no sandbox, only the `solace` MCP tool. TrueForge reaches MCP
  servers over HTTP only, so `solace mcp` (stdio) is served through `Bridge`.
"""

import base64
import http.client
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import threading
import time
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

NPX_PACKAGE = "@truefoundry/trueforge@0.179.1"
# A turn pauses for tool approval or a question; the bench approves and continues this many times.
MAX_RESUMES = 50


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def slug(model):
    """TrueForge model and provider names: lowercase letters, digits and dashes."""
    return re.sub(r"[^a-z0-9]+", "-", model.lower()).strip("-")


class Server:
    """A private TrueForge server with one OpenAI-compatible model registered as `local/<slug>`."""

    def __init__(self, home: Path, model, base_url, context=None):
        self.home, self.port = home, free_port()
        self.url = f"http://localhost:{self.port}"
        (home / "tmp").mkdir()
        # Its own data and temp folders: TrueForge clears $TMPDIR/tf_cms at start, which a user's own server uses.
        env = {**os.environ, "XDG_DATA_HOME": str(home / "data"), "TMPDIR": str(home / "tmp"),
               "PORT": str(self.port), "SENTRY_ENABLED": "false",
               # The sandbox runtime sets TMPDIR=/tmp/claude, which TrueForge's own rules make read-only, and
               # gleam then fails. Relative instead: gleam runs at the project root, which is the sandbox root.
               "CLAUDE_CODE_TMPDIR": ".tmp",
               # TrueForge refuses loopback URLs unless allowed: the model and the solace bridge are local.
               "OUTBOUND_URL_ALLOWED_HOSTS": json.dumps(["127.0.0.1", "localhost"])}
        self.gate = Gate(base_url)
        self.log = open(home / "server.log", "w")
        self.proc = subprocess.Popen(["npx", "-y", NPX_PACKAGE, "--port", str(self.port)], cwd=home, env=env,
                                     stdout=self.log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            for _ in range(120):
                try:
                    self.api("GET", "/api/v1/capabilities")
                    break
                except OSError:
                    if self.proc.poll() is not None:
                        raise RuntimeError(f"TrueForge did not start; see {home / 'server.log'}")
                    time.sleep(1)
            props = {"context_length": context, "max_output_tokens": min(32000, context // 4)} if context else {}
            self.api("PUT", "/api/v1/settings/model-providers", {"manifest": {
                "type": "custom", "name": "local", "base_url": self.gate.url, "auth": {"api_key": "none"},
                "models": [{"model_id": model, "name": slug(model), "properties": props}]}})
            self.model = f"local/{slug(model)}"
        except BaseException:  # a half-started server would outlive the bench
            self.close()
            raise

    def api(self, method, path, body=None, stream=False):
        req = urllib.request.Request(self.url + path, method=method, data=None if body is None else json.dumps(body).encode(),
                                     headers={"content-type": "application/json", "accept": "text/event-stream" if stream else "application/json"})
        r = urllib.request.urlopen(req, timeout=1800)
        if stream:
            return r
        with r:
            return json.load(r)

    def run_task(self, *args):
        return run_task(self, *args)

    def close(self):
        os.killpg(self.proc.pid, signal.SIGTERM)  # npx and the node server it started
        try:
            self.proc.wait(10)
        except subprocess.TimeoutExpired:
            os.killpg(self.proc.pid, signal.SIGKILL)
        self.log.close()
        self.gate.close()


class Gate:
    """A pass-through to the model server that holds requests while closed."""

    def __init__(self, upstream):
        self.open = threading.Event()
        self.open.set()
        target, gate = urllib.parse.urlsplit(upstream), self

        class Handler(BaseHTTPRequestHandler):
            def relay(self):
                gate.open.wait()
                body = self.rfile.read(int(self.headers.get("content-length", 0)))
                conn = http.client.HTTPConnection(target.hostname, target.port, timeout=1800)
                path = target.path.rstrip("/") + self.path.removeprefix("/v1")
                conn.request(self.command, path, body, {k: v for k, v in self.headers.items() if k.lower() != "host"})
                r = conn.getresponse()
                self.send_response(r.status)
                for k, v in r.getheaders():
                    if k.lower() not in ("transfer-encoding", "content-length", "connection"):
                        self.send_header(k, v)
                self.end_headers()  # HTTP/1.0: the body ends when the connection closes
                while chunk := r.read1(65536):
                    self.wfile.write(chunk)
                    self.wfile.flush()
                conn.close()

            do_GET = do_POST = relay

            def log_message(self, *_):
                pass

        self.http = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.http.server_port}/v1"
        threading.Thread(target=self.http.serve_forever, daemon=True).start()

    def close(self):
        self.http.shutdown()
        self.http.server_close()


class Bridge:
    """`solace mcp` (one JSON-RPC message per line on stdio) served as MCP over HTTP, plain JSON replies."""

    def __init__(self, cmd, cwd, env):
        env = None if env is None else {**env, "PWD": str(cwd)}
        self.proc = subprocess.Popen(cmd, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        lock, proc = threading.Lock(), self.proc

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                msg = self.rfile.read(int(self.headers.get("content-length", 0))).decode()
                with lock:
                    proc.stdin.write(msg.replace("\n", " ") + "\n")
                    proc.stdin.flush()
                    # solace replies to requests, never to notifications (no id)
                    reply = proc.stdout.readline() if "id" in json.loads(msg) else None
                self.send_response(200 if reply else 202)
                self.send_header("content-type", "application/json")
                self.end_headers()
                if reply:
                    self.wfile.write(reply.encode())

            def do_GET(self):  # no server-initiated stream
                self.send_response(405)
                self.end_headers()

            def do_DELETE(self):
                self.send_response(200)
                self.end_headers()

            def log_message(self, *_):
                pass

        self.http = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.http.server_port}/mcp"
        threading.Thread(target=self.http.serve_forever, daemon=True).start()

    def close(self):
        self.http.shutdown()
        self.http.server_close()
        self.proc.terminate()
        self.proc.wait()
        self.proc.stdin.close()
        self.proc.stdout.close()


def spec(arm, model):
    """The agent: the model and the arm's tools; TrueForge's extras off, as in the other harnesses."""
    config = {"sandbox": {"enabled": arm == "files"}, "dynamic_sub_agents": {"enabled": False},
              "generative_ui": {"enabled": False}, "ask_user_questions": {"enabled": False},
              "web_search": {"enabled": False}}
    s = {"model": {"name": model}, "config": config}
    if arm == "solace":  # D-8: the protocol only, as one MCP tool, loaded up front like Claude Code's
        s["mcp_servers"] = [{"name": "solace", "preload": True, "require_approval_for_tools": []}]
    return s


def sse(response):
    """Events from a text/event-stream reply."""
    data = []
    for raw in response:
        line = raw.decode().rstrip("\r\n")
        if line.startswith("data:"):
            data.append(line[5:].lstrip())
        elif not line and data:
            try:
                yield json.loads("\n".join(data))
            except json.JSONDecodeError:
                pass
            data = []


def run_task(server: Server, arm, work: Path, prompt, solace_cmd=None, env=None):
    """One task as one session. Returns its events (JSON lines); `work` holds the agent's result."""
    bridge = None
    if arm == "solace":
        bridge = Bridge(solace_cmd, work, env)
        server.api("PUT", "/api/v1/settings/mcp-servers", {"manifest": {
            "type": "remote", "name": "solace", "url": bridge.url, "description": "The Solace semantic store"}})
    sandbox = None
    try:
        session = server.api("POST", "/api/v1/sessions", {"agent": {"spec": spec(arm, server.model)}})["data"]["id"]
        content = prompt
        if arm == "files":
            server.gate.open.clear()
            task = "data:text/markdown;base64," + base64.b64encode(prompt.encode()).decode()
            content = [{"type": "text", "text": prompt}, {"type": "file", "name": "TASK.md", "data": task}]
        turn_input = [{"type": "user.message", "content": content}]
        turns = []
        for _ in range(MAX_RESUMES):
            actions = []
            with server.api("POST", f"/api/v1/sessions/{session}/turns", {"input": turn_input, "stream": True}, stream=True) as r:
                for ev in sse(r):
                    ev = ev.get("data", ev)
                    if ev.get("type") == "turn.created" and ev["turn_id"] not in turns:
                        turns.append(ev["turn_id"])
                    elif ev.get("type") == "sandbox.created" and sandbox is None:
                        sandbox = seed(ev["sandbox_id"], work)
                        server.gate.open.set()
                    elif ev.get("type") == "turn.done":
                        actions = (ev.get("state") or {}).get("required_actions") or []
            turn_input = resume(actions)
            if not turn_input:
                break
        return [json.dumps(e) for t in turns for e in turn_events(server, session, t)]
    finally:
        server.gate.open.set()
        if bridge:
            bridge.close()
        if sandbox:  # the agent worked there: its tree, less TrueForge's own folders, replaces the copy
            root, harness = sandbox
            shutil.rmtree(work)
            shutil.copytree(root, work, ignore=lambda d, names: harness & set(names) if Path(d) == root else ())


def seed(sandbox_id, work: Path):
    """Copy the task's project into the new sandbox, noting the folders TrueForge made there itself."""
    root = Path(sandbox_id.rsplit(":", 1)[-1])  # "v1:local:/…/sandboxes/<session>/<sandbox>"
    (root / ".tmp").mkdir(exist_ok=True)  # the sandbox's TMPDIR, see Server
    harness = {p.name for p in root.iterdir()}
    shutil.copytree(work, root, dirs_exist_ok=True)
    return root, harness


def resume(actions):
    """The next turn's input after a pause: approve each tool call; there is nobody to answer questions."""
    out = []
    for a in actions:
        if a.get("type") == "tool.approval_required":
            out += [{"type": "user.tool_approval", "thread_id": a["thread_id"], "tool_call_id": c["id"],
                     "approval": {"status": "allow"}} for c in a.get("tool_calls", [])]
        elif a.get("type") == "tool.response_required":
            out += [{"type": "user.tool_response", "thread_id": a["thread_id"], "tool_call_id": c["id"],
                     "content": "No one can answer; decide yourself and continue."} for c in a.get("tool_calls", [])]
    return out


def turn_events(server: Server, session, turn):
    """A turn's events, oldest first, with streamed messages already merged."""
    events, token = [], None
    while True:
        q = "?limit=100&order=asc" + (f"&page_token={token}" if token else "")
        page = server.api("GET", f"/api/v1/sessions/{session}/turns/{turn}/events{q}")
        events += page.get("data", [])
        token = (page.get("pagination") or {}).get("next_page_token")
        if not token:
            return events
