"""Exercise the built custom element in a real headless browser."""
import functools
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
from urllib.parse import parse_qs, urlsplit

ROOT = Path(__file__).resolve().parents[1]
REQUESTS = []
RESULT = threading.Event()
RESULT_VALUE = {}

class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def reply(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        REQUESTS.append(self.path)
        if self.path.startswith("/__result?"):
            query = parse_qs(urlsplit(self.path).query)
            RESULT_VALUE.update({key: values[0] for key, values in query.items()})
            RESULT.set()
            self.reply({"ok": True})
        elif self.path == "/migration":
            self.reply({"state": "available", "top": "/work", "candidates": [
                {"dir": ".", "name": "app"}, {"dir": "web", "name": "web"}]})
        else:
            super().do_GET()

    def do_POST(self):
        REQUESTS.append(self.path)
        value = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if self.path == "/migration/start" and value == {"projects": ["web"], "smoke": ["gleam test"]}:
            self.reply({"code": "fixture", "message": "fixture accepted web / gleam test"})
        else:
            self.reply({"code": "fixture", "message": "unexpected request"})

browser = next((path for name in ["chromium", "chromium-browser", "google-chrome", "brave-browser", "brave"]
                if (path := shutil.which(name))), None)
if not browser:
    raise SystemExit("No Chromium browser installed; component browser check could not run")
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Handler, directory=str(ROOT)))
threading.Thread(target=server.serve_forever, daemon=True).start()
try:
    with tempfile.TemporaryDirectory(prefix="migration-browser-") as profile:
        command = [browser, "--headless=new", "--no-sandbox", "--disable-gpu",
            "--no-proxy-server", "--user-data-dir=" + profile,
            f"http://127.0.0.1:{server.server_port}/test/component.html"]
        environment = {**os.environ, "QT_QPA_PLATFORM": "offscreen"}
        process = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=environment)
        try:
            if not RESULT.wait(20):
                raise SystemExit("Browser fixture timed out. Requests: " + repr(REQUESTS))
            if RESULT_VALUE.get("status") != "passed":
                raise SystemExit("Component browser check failed: " + RESULT_VALUE.get("error", "")
                                 + "\nRequests: " + repr(REQUESTS))
            print("Component browser check passed: standalone mount, registration, shadow CSS isolation, inputs, root POST, independent instances, disconnect and reconnect.")
        finally:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
finally:
    server.shutdown()
