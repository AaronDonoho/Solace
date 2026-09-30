"""No provider calls: speak to the actual Solace MCP process over stdio."""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

prompt = sys.stdin.read()
task = json.loads(prompt.split('\n', 1)[1])
rid = os.environ['SOLACE_RUN']
root = Path('.solace')
(root / (rid + '.prompt')).write_text(prompt)
(root / (rid + '.pid')).write_text(str(os.getpid()))
steps = json.loads(task['body'])
mcp = subprocess.Popen([os.environ['SOLACE_BIN'], 'mcp'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
for i, step in enumerate(steps):
    if 'exit' in step:
        sys.exit(step['exit'])
    if 'sleep' in step:
        time.sleep(step['sleep'])
        continue
    mcp.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': i, 'method': 'tools/call', 'params': {'name': 'solace', 'arguments': step}}) + '\n')
    mcp.stdin.flush()
    response = json.loads(mcp.stdout.readline())
    with (root / (rid + '.responses')).open('a') as f:
        f.write(json.dumps(response) + '\n')
# Stay alive after task done long enough to test that Solace owns termination.
time.sleep(20)
