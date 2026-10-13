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
store = next(d / '.solace/tasks' for d in [Path.cwd(), *Path.cwd().parents] if (d / '.solace/tasks').is_dir())
rid = os.environ['SOLACE_RUN']
if os.environ['SOLACE_TASK'] == 'strategy':
    # A strategy run (D-93) has no task: its steps are its first note.
    run = json.loads((store.parent / 'strategy/runs' / (rid + '.json')).read_text())
    steps = json.loads(run['notes'][0]['text'])
else:
    task = json.loads((store / (os.environ['SOLACE_TASK'] + '.json')).read_text())
    steps = json.loads(task['body'])
root = Path('.solace')
(root / (rid + '.prompt')).write_text(prompt)
(root / (rid + '.pid')).write_text(str(os.getpid()))
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
# The crash hook is test-only; forward it to the Solace process that runs the command.
if 'SOLACE_FAIL_AFTER' in os.environ:
    mcp_env['SOLACE_FAIL_AFTER'] = os.environ['SOLACE_FAIL_AFTER']
mcp = subprocess.Popen([config['command'], *config['args']], env=mcp_env,
                       stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
# `{draft}` in a step is the draft the last refusal kept, whose id a script can't know (D-120).
draft = ''
for i, step in enumerate(steps):
    step = {k: v.replace('{draft}', draft) if isinstance(v, str) else v for k, v in step.items()}
    if 'exit' in step:
        if 'error' in step:
            print(json.dumps({'type':'error', 'message':step['error']}), flush=True)
        mcp.stdin.close()
        mcp.wait(timeout=3)
        sys.exit(step['exit'])
    if 'say' in step:
        print(json.dumps({'type':'item.completed', 'item':{'type':'agent_message', 'text':step['say']}}), flush=True)
        continue
    if 'sleep' in step:
        time.sleep(step['sleep'])
        continue
    mcp.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': i, 'method': 'tools/call', 'params': {'name': 'solace', 'arguments': step}}) + '\n')
    mcp.stdin.flush()
    response = json.loads(mcp.stdout.readline())
    text = (response.get('result', {}).get('content') or [{}])[0].get('text', '')
    if 'Draft ' in text:
        draft = text.split('Draft ')[1].split()[0]
    with (root / (rid + '.responses')).open('a') as f:
        f.write(json.dumps(response) + '\n')
# A real provider still needs to receive task done and persist the conversation.
mcp.stdin.close()
mcp.wait(timeout=3)
time.sleep(.15)
if native:
    (home / ('session-' + session)).write_text('persisted after done')
(root / (rid + '.exited')).write_text('graceful')
