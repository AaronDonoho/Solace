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
native = '--json' in sys.argv or '--output-format' in sys.argv
claude = '--output-format' in sys.argv
home = Path(os.environ['CLAUDE_CONFIG_DIR'] if claude else os.environ['CODEX_HOME'])
session = rid
if native:
    if 'resume' in sys.argv or '--resume' in sys.argv:
        at = sys.argv.index('--resume' if claude else 'resume')
        # Codex puts the session after its model; Claude immediately after --resume.
        session = sys.argv[at + 1] if claude else sys.argv[sys.argv.index('-m') + 2]
        if not (home / ('session-' + session)).exists():
            print(json.dumps({'type': 'error', 'message': 'session not found'}), flush=True)
            sys.exit(1)
    (root / (rid + '.launch')).write_text(json.dumps({'session': session, 'home': str(home), 'argv': sys.argv}))
    print(json.dumps({'type': 'system', 'subtype': 'init', 'session_id': session} if claude else {'type': 'thread.started', 'thread_id': session}), flush=True)
# Model Codex's MCP environment boundary: Solace variables need explicit forwarding.
config = json.loads((home / 'mcp.json').read_text())['mcpServers']['solace'] if claude else tomllib.loads((home / 'config.toml').read_text())['mcp_servers']['solace']
mcp_env = {k: v for k, v in os.environ.items() if not k.startswith('SOLACE_')}
mcp_env.update({k: os.environ[k] for k in (['SOLACE_TASK', 'SOLACE_RUN', 'SOLACE_SESSION', 'SOLACE_CONFIG'] if claude else config.get('env_vars', [])) if k in os.environ})
mcp = subprocess.Popen([config['command'], *config['args']], env=mcp_env,
                       stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
for i, step in enumerate(steps):
    if 'exit' in step:
        if 'error' in step:
            print(json.dumps({'type':'error', 'message':step['error']}), flush=True)
        mcp.stdin.close()
        mcp.wait(timeout=3)
        sys.exit(step['exit'])
    if 'sleep' in step:
        time.sleep(step['sleep'])
        continue
    mcp.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': i, 'method': 'tools/call', 'params': {'name': 'solace', 'arguments': step}}) + '\n')
    mcp.stdin.flush()
    response = json.loads(mcp.stdout.readline())
    with (root / (rid + '.responses')).open('a') as f:
        f.write(json.dumps(response) + '\n')
# A real provider still needs to receive task done and persist the conversation.
mcp.stdin.close()
mcp.wait(timeout=3)
time.sleep(.15)
if native:
    (home / ('session-' + session)).write_text('persisted after done')
(root / (rid + '.exited')).write_text('graceful')
