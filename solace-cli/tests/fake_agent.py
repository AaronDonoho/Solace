"""No provider calls: speak to the actual Solace MCP process over stdio."""
import json
import os
import subprocess
import sys
import time
import tomllib
from pathlib import Path

prompt = sys.stdin.read()
# The prompt is plain text for a model; this stand-in reads its steps from the task file.
task = json.loads((Path('.solace/board') / (os.environ['SOLACE_TASK'] + '.json')).read_text())
rid = os.environ['SOLACE_RUN']
root = Path('.solace')
(root / (rid + '.prompt')).write_text(prompt)
(root / (rid + '.pid')).write_text(str(os.getpid()))
steps = json.loads(task['body'])
# Model Codex's MCP environment boundary: Solace variables need explicit forwarding.
config = tomllib.loads((Path(os.environ['CODEX_HOME']) / 'config.toml').read_text())['mcp_servers']['solace']
mcp_env = {k: v for k, v in os.environ.items() if not k.startswith('SOLACE_')}
mcp_env.update({k: os.environ[k] for k in config.get('env_vars', []) if k in os.environ})
mcp = subprocess.Popen([config['command'], *config['args']], env=mcp_env,
                       stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
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
