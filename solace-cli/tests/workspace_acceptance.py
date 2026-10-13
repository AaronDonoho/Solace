"""One Solace root (D-57): its projects are the root itself and folders below it, on one board."""
import http.client
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time
import unittest

BIN = os.environ['SOLACE_TEST_BIN']
FAKE = str(Path(__file__).with_name('fake_agent.py'))

def project(path, name='pilot', body='pub fn one() { 1 }\n', target='javascript'):
    (path/'src').mkdir(parents=True, exist_ok=True)
    # JavaScript builds skip the Erlang compiler, which takes most of a build's time.
    (path/'gleam.toml').write_text(f'name="{name}"\nversion="1.0.0"\n' + (f'target="{target}"\n' if target else ''))
    (path/'src/app.gleam').write_text(body)

class Root(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='solace-root-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)/'fixture'
        self.root.mkdir()
        (self.root/'.git').mkdir()
        config = self.root/'.git/system.toml'
        config.write_text('task_timeout=20\n[[models]]\nid="fake"\nlabel="Fake agent"\nagent="codex"\nrun=["python3",' + json.dumps(FAKE) + ']\n')
        self.env = {**os.environ, 'SOLACE_CONFIG': str(config)}
        self.servers = []
        self.clients = []
    def tearDown(self):
        for c in self.clients: c.close()
        for s in self.servers:
            s.terminate(); s.wait(timeout=3)
            s.stdout.close(); s.stderr.close()
        self.tmp.cleanup()
    def cli(self, *args, cwd=None, input=None, ok=True):
        p = subprocess.run([BIN, *args], cwd=cwd or self.root, env=self.env, input=input, text=True, capture_output=True)
        if ok: self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        else: self.assertNotEqual(p.returncode, 0, p.stdout + p.stderr)
        return p.stdout + p.stderr
    def migrate(self, *args):
        self.cli('migrate', '--yes', *args)
        self.cli('migrate', '--adopt', '--yes')
    def serve(self, cwd):
        return subprocess.Popen([BIN, 'serve', '--port', '0'], cwd=cwd, env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    def start(self, cwd=None):
        server = self.serve(cwd or self.root)
        self.servers.append(server)
        line = server.stdout.readline()
        self.assertIn('listening', line, line or server.stderr.read())
        self.port = int(line.strip().rsplit(':', 1)[1])
    def call(self, method, url, body=None):
        c = http.client.HTTPConnection('127.0.0.1', self.port, timeout=30)
        c.request(method, url, None if body is None else json.dumps(body), {'Content-Type': 'application/json'})
        r = c.getresponse(); text = r.read(); c.close()
        if r.headers.get('Content-Type') != 'application/json': return r.status, text.decode()
        return r.status, json.loads(text)
    def api(self, method, url, body=None):
        status, v = self.call(method, url, body)
        self.assertEqual(status, 200, v)
        return v
    def refused(self, method, url, body, words):
        status, v = self.call(method, url, body)
        self.assertEqual(status, 409, v)
        self.assertIn(words, v['message'])
    def stream(self, url='/events', last=None):
        c = http.client.HTTPConnection('127.0.0.1', self.port, timeout=8)
        c.request('GET', url, headers={'Last-Event-ID': last} if last else {})
        r = c.getresponse(); self.clients.append(c); return r
    def next_event(self, r):
        """(id, data) of the next event; ('ready', None) at the end of the replay."""
        ident = None
        while True:
            line = r.readline().decode().strip()
            if line.startswith('id:'): ident = line[3:].strip()
            if line == 'event: ready': r.readline(); r.readline(); return 'ready', None
            if line.startswith('data:'): return ident, json.loads(line[5:])
    def until_ready(self, r):
        while self.next_event(r)[0] != 'ready': pass
    def wait(self, base, t, predicate, timeout=20):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            v = self.api('GET', f'{base}/tasks/{t["id"]}')
            if predicate(v): return v
            time.sleep(.05)
        self.fail('timed out: ' + json.dumps(v))

    def test_folder_of_projects(self):
        # Two projects with the same package name: lanes are keyed by folder.
        # a has no target, so the board reports Gleam's default.
        project(self.root/'a', target=None)
        for d in ['b', 'a/build/packages/dep']: project(self.root/d)
        self.migrate('--project', 'a', '--project', 'b')
        for d in ['a', 'b']:
            self.assertTrue((self.root/d/'.solace/store.db').exists())
            self.assertFalse((self.root.with_name('fixture.pre-solace')/d/'.solace').exists())
        self.assertFalse((self.root/'a/build/packages/dep/.solace').exists())

        self.start()
        self.api('DELETE', '/projects/b')
        v = self.api('GET', '/projects')
        self.assertTrue(v['workspace'])
        # The root folder itself is a project of plain files (D-60).
        self.assertEqual([(p['dir'], p['label'], p['name'], p['target'], p['base']) for p in v['projects']], [('.', 'fixture', '', '', ''), ('a', 'a', 'pilot', 'erlang', '/p/a')])
        self.assertEqual([(f['dir'], f['member']) for f in v['found']], [('a', True), ('b', False)])
        self.assertIsInstance(self.api('GET', '/models'), list)

        events = self.stream()
        self.until_ready(events)
        v = self.api('POST', '/projects', {'dir': 'b'})
        self.assertEqual([p['dir'] for p in v['projects']], ['.', 'a', 'b'])
        self.assertEqual(self.next_event(events)[1], {'projects': ['.', 'a', 'b']})
        # D-87: the root's records follow the list, before the project that came back.
        self.assertEqual(self.next_event(events)[1], {'milestones': []})
        self.assertEqual(self.next_event(events)[1], {'features': []})
        self.assertEqual(self.next_event(events)[1]['vision']['text'], '')
        self.assertEqual(self.next_event(events)[1], {'strategyRuns': []})
        # D-78: the project that came back follows with its workflow and tasks as they are.
        self.assertEqual(self.next_event(events)[1]['project'], 'b')
        ident, data = self.next_event(events)
        self.assertEqual((data['project'], data['tasks']), ('b', []))

        self.api('POST', '/p/a/tasks', {'title': 'In a'})
        self.assertEqual([x['title'] for x in self.api('GET', '/p/a/tasks')], ['In a'])
        self.assertEqual(self.api('GET', '/p/b/tasks'), [])
        self.refused('GET', '/p/nope/tasks', None, 'unknown project')
        self.assertEqual(self.api('GET', '/tasks'), [])
        ident, data = self.next_event(events)
        self.assertEqual((data['project'], data['task']['title']), ('a', 'In a'))
        self.assertEqual(set(json.loads(ident)), {'tasks', '.', 'a', 'b'})

        # A reconnect replays only what it missed, from every project.
        events.close()
        self.api('POST', '/p/b/tasks', {'title': 'In b'})
        events = self.stream(last=ident)
        ident, data = self.next_event(events)
        self.assertEqual((data['project'], data['task']['title']), ('b', 'In b'))
        self.assertEqual(self.next_event(events), ('ready', None))

        # A project's own stream and view, as a project on its own has them.
        self.api('POST', '/milestones', {'title': 'Root only'})
        single = self.stream('/p/a/events')
        ident, data = self.next_event(single)
        self.assertNotIn('project', data)
        self.assertEqual(set(json.loads(ident)), {'tasks', 'a'})
        # D-87: the root's records are not a project's; its stream and routes leave them out.
        keys = set(data)
        while (e := self.next_event(single))[0] != 'ready': keys |= set(e[1])
        self.assertFalse(keys & {'milestones', 'features', 'milestone', 'feature', 'vision', 'strategyRuns', 'strategyRun'}, keys)
        self.refused('GET', '/p/a/milestones', None, 'unknown route')
        self.refused('GET', '/p/a/vision', None, 'unknown route')
        self.refused('GET', '/p/a/strategy/runs', None, 'unknown route')
        self.refused('POST', '/p/a/features', {'title': 'F'}, 'unknown route')
        self.assertIn('<div id="app">', self.call('GET', '/p/a/contracts')[1])

        # Inside a project, the root is still the one root: a second server is refused.
        other = self.serve(self.root/'a/src')
        self.assertNotEqual(other.wait(timeout=10), 0)
        self.assertIn('already served', other.stdout.read() + other.stderr.read())
        other.stdout.close(); other.stderr.close()

        # Existing code must be migrated, never imported by create or POST /projects.
        project(self.root/'c', 'c')
        self.assertIn('migrating', self.cli('create', 'child', '--yes', cwd=self.root/'c', ok=False))
        self.refused('POST', '/projects', {'dir': 'c'}, 'migrating')
        self.assertFalse((self.root/'c/.solace').exists())
        self.assertFalse((self.root/'c/child').exists())

        v = self.api('POST', '/projects/new', {'name': 'fresh', 'target': 'javascript'})
        self.assertEqual([p['target'] for p in v['projects'] if p['dir'] == 'fresh'], ['javascript'])
        for f in ['gleam.toml', 'solace.toml', '.solace/store.db', 'src/fresh.gleam']:
            self.assertTrue((self.root/'fresh'/f).exists(), f)
        self.assertFalse((self.root/'fresh/.git').exists())

        v = self.api('DELETE', '/projects/b')
        self.assertEqual([p['dir'] for p in v['projects']], ['.', 'a', 'fresh'])
        self.assertTrue((self.root/'b/.solace/store.db').exists())
        self.api('POST', '/projects', {'dir': 'b'})
        self.assertEqual([t['title'] for t in self.api('GET', '/p/b/tasks')], ['In b'])

        # Removing with its files (D-74) needs the project's name; a wrong one changes nothing.
        self.refused('DELETE', '/projects/fresh', {'delete': True, 'name': 'other'}, 'type the project')
        self.refused('DELETE', '/projects/fresh', {'delete': True}, 'type the project')
        self.refused('DELETE', '/projects/.', {'delete': True, 'name': '.'}, 'can')
        self.assertTrue((self.root/'fresh/gleam.toml').exists())
        self.assertIn('fresh', [p['dir'] for p in self.api('GET', '/projects')['projects']])
        v = self.api('DELETE', '/projects/fresh', {'delete': True, 'name': 'fresh'})
        self.assertEqual([p['dir'] for p in v['projects']], ['.', 'a', 'b'])
        self.assertFalse((self.root/'fresh').exists())
        self.assertNotIn('fresh', (self.root/'solace.toml').read_text())
        # Without `delete`, the files stay.
        self.api('DELETE', '/projects/b')
        self.assertTrue((self.root/'b/.solace/store.db').exists())

    def test_mcp_serves_the_command_tool(self):
        project(self.root, body='pub fn one() { 1 }\n\npub fn two() { one() + 1 }\n')
        self.migrate()
        p = subprocess.Popen([BIN, 'mcp'], cwd=self.root, env=self.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.addCleanup(lambda: (p.stdin.close(), p.wait(timeout=10), p.stdout.close()))
        def rpc(id, method, params=None):
            p.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': id, 'method': method, 'params': params or {}}) + '\n')
            p.stdin.flush()
            reply = json.loads(p.stdout.readline())
            self.assertEqual(reply['id'], id)
            return reply
        def call(command, **args):
            r = rpc(9, 'tools/call', {'name': 'solace', 'arguments': {'command': command, **args}})['result']
            return r['content'][0]['text'], r['isError']
        self.assertEqual(rpc(1, 'initialize', {'protocolVersion': '2025-06-18'})['result']['serverInfo']['name'], 'solace')
        # A notification gets no reply; the next request's reply comes next.
        p.stdin.write(json.dumps({'jsonrpc': '2.0', 'method': 'notifications/initialized'}) + '\n')
        tools = rpc(2, 'tools/list')['result']['tools']
        self.assertEqual([t['name'] for t in tools], ['solace'])
        self.assertEqual(rpc(3, 'nope')['error']['code'], -32601)
        # Several commands run in order, each under a header.
        text, error = call('view app.two\nsolace refs app.one')
        self.assertFalse(error, text)
        self.assertIn('==== solace view app.two ====', text)
        self.assertIn('==== solace refs app.one ====', text)
        self.assertIn('app.two', text.split('==== solace refs app.one ====')[1])
        # D-109: a batch with a failed command names it on its last line.
        text, error = call('view app.two\nread nowhere.md\nrefs app.one')
        self.assertTrue(error)
        self.assertIn('app.two', text.split('==== solace refs app.one ====')[1])
        self.assertTrue(text.endswith('1 of 3 commands failed: read nowhere.md\n'), text)
        # D-119: a failed change stops the call, so the run after it never runs on unsaved work.
        settings = self.root/'solace.toml'
        self.assertIn('[commands]\n', settings.read_text())
        settings.write_text(settings.read_text().replace('[commands]\n', '[commands]\nmark = { run = ["touch", "marked"] }\n', 1))
        text, error = call('write notes.md\nrun mark', text='*** Begin Patch\n*** Add File: notes.md\n+x\n*** End Patch\n')
        self.assertTrue(error)
        self.assertNotIn('==== solace run mark ====', text)
        self.assertFalse((self.root/'marked').exists())
        self.assertTrue(text.endswith('1 of 2 commands failed: write notes.md; skipped: run mark\n'), text)
        text, error = call('run mark')
        self.assertFalse(error, text)
        self.assertTrue((self.root/'marked').exists())
        # Code in `command` runs nothing.
        text, error = call('let x = app.one()')
        self.assertTrue(error)
        self.assertTrue(text.startswith('Not a command: let x = app.one()'), text)
        text, error = call('')
        self.assertTrue(error)
        self.assertTrue(text.startswith('No command given.'), text)
        text, error = call('submit app.one', text='pub fn one() { 2 }')
        self.assertFalse(error, text)
        self.assertIn('Saved app.one', text)

    def test_a_wide_grep_shares_the_reply_budget_with_the_read_after_it(self):
        # D-119: grep stops at a whole file within what the call has left, so the read after it still fits.
        project(self.root, body='pub fn one() { 1 }\n')
        (self.root/'docs').mkdir()
        for n in range(12):
            (self.root/f'docs/wide{n:02}.md').write_text(f'needle {"w" * 150}\n' * 8)
        (self.root/'notes.md').write_text('first notes line\nsecond notes line\n')
        self.migrate()
        budget = 6000
        p = subprocess.Popen([BIN, 'mcp', '--budget', str(budget)], cwd=self.root, env=self.env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.addCleanup(lambda: (p.stdin.close(), p.wait(timeout=10), p.stdout.close()))
        def call(command):
            p.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call', 'params': {'name': 'solace', 'arguments': {'command': command}}}) + '\n')
            p.stdin.flush()
            r = json.loads(p.stdout.readline())['result']
            return r['content'][0]['text'], r['isError']
        text, error = call('grep needle\nread notes.md')
        self.assertFalse(error, text)
        grep, read = text.split('==== solace read notes.md ====\n')
        grep = grep.split('==== solace grep needle ====\n')[1]
        self.assertIn('first notes line\nsecond notes line\n', read)
        self.assertLessEqual(len(text), budget)
        shown = grep.count('.md\n')
        self.assertTrue(0 < shown < 12, grep)
        self.assertEqual(grep.count('needle'), shown * 8, 'every file shown is whole')
        self.assertIn(f'… {12 - shown} more files match, not shown; narrow with `--in <path>`.', grep)
        # Narrowed to one file, with context, the same call shows it all.
        text, error = call('grep needle --in docs/wide00.md --context 1\nread notes.md')
        self.assertFalse(error, text)
        self.assertIn('docs/wide00.md\n', text)
        self.assertNotIn('more files match', text)
        self.assertIn('first notes line', text)

    def test_create_requires_a_new_name_and_confirmation(self):
        self.assertIn('create <name>', self.cli('create', ok=False))
        for answer in ['', '\n', 'n\n']:
            out = self.cli('create', 'fresh', input=answer)
            self.assertIn('[y/N]', out)
            self.assertFalse((self.root/'fresh').exists())
            self.assertFalse((self.root/'solace.toml').exists())
        self.cli('create', 'fresh', input='y\n')
        for f in ['gleam.toml', 'solace.toml', '.solace/store.db', 'src/fresh.gleam']:
            self.assertTrue((self.root/'fresh'/f).exists(), f)
        self.assertFalse((self.root/'fresh/.git').exists())
        self.assertIn('already exists', self.cli('create', 'fresh', '--yes', ok=False))
        (self.root/'apps').mkdir()
        self.cli('create', 'web', '--javascript', '--yes', cwd=self.root/'apps')
        self.assertIn('target = "javascript"', (self.root/'apps/web/gleam.toml').read_text())
        self.assertIn('"apps/web"', (self.root/'solace.toml').read_text())
        self.assertFalse((self.root/'apps/solace.toml').exists())

    def test_serve_offers_migration_for_an_unmanaged_project(self):
        project(self.root)
        self.start()
        v = self.api('GET', '/migration')
        self.assertEqual(v['state'], 'available')
        self.assertEqual(v['candidates'], [{'dir': '.', 'name': 'pilot'}])
        self.assertFalse((self.root/'.solace/store.db').exists())

    def test_everything_stays_under_one_root(self):
        project(self.root/'a')
        self.migrate()
        outside = Path(tempfile.mkdtemp(prefix='solace-outside-'))
        try:
            project(outside/'ext')
            (self.root/'link').symlink_to(outside/'ext')
            self.start()
            self.api('DELETE', '/projects/a')
            self.assertEqual([p['dir'] for p in self.api('GET', '/projects')['projects']], ['.'])
            self.refused('POST', '/projects', {'dir': 'link'}, 'migrating')
            self.refused('POST', '/projects', {'dir': '../x'}, 'migrating')
            self.refused('POST', '/projects/new', {'name': 'ok', 'dir': '../out'}, 'below the root')
            self.refused('POST', '/projects/new', {'name': 'ok', 'dir': 'link/new'}, 'without symlinks')
            self.refused('POST', '/projects/new', {'name': 'Bad-Name'}, "isn't a Gleam package name")
            self.refused('POST', '/projects/new', {'name': 'a'}, 'already exists')
            self.assertFalse((outside/'ext/.solace').exists())
            self.assertFalse((outside/'new').exists())
            # A project that lists projects of its own would be a second root.
            (self.root/'a/solace.toml').write_text('projects = []\n')
            self.refused('POST', '/projects', {'dir': 'a'}, 'lists projects of its own')
            (self.root/'a/solace.toml').unlink()
            self.api('POST', '/projects', {'dir': 'a'})
            # So would a folder of projects inside the root, or inside a project.
            for nested in ['inner', 'a/docs']:
                (self.root/nested).mkdir(exist_ok=True)
                (self.root/nested/'solace.toml').write_text('projects = []\n')
                self.assertIn('is inside the root', self.cli('serve', cwd=self.root/nested, ok=False))
                (self.root/nested/'solace.toml').unlink()
        finally:
            subprocess.run(['rm', '-rf', str(outside)])

    def test_the_root_is_a_project_too(self):
        project(self.root, 'visualizer', 'pub fn root_fn() { 1 }\n')
        self.migrate('--project', '.')
        self.start()
        v = self.api('GET', '/projects')
        self.assertFalse(v['workspace'])
        self.assertEqual([(p['dir'], p['base']) for p in v['projects']], [('.', '')])
        self.assertEqual([(f['dir'], f['member']) for f in v['found']], [('.', True)])
        # A project on its own keeps its plain API and events.
        self.api('POST', '/tasks', {'title': 'Root task'})
        events = self.stream()
        ident, data = self.next_event(events)
        self.assertEqual(set(json.loads(ident)), {'tasks', '.'})
        self.assertNotIn('project', data)
        events.close()

        v = self.api('POST', '/projects/new', {'name': 'frontend'})
        self.assertTrue(v['workspace'])
        self.assertEqual([(p['dir'], p['base']) for p in v['projects']], [('.', ''), ('frontend', '/p/frontend')])
        settings = (self.root/'solace.toml').read_text()
        self.assertIn('projects = ["frontend"]', settings)
        self.assertIn('watch_limit', settings)
        self.assertEqual([t['title'] for t in self.api('GET', '/tasks')], ['Root task'])
        self.assertEqual(self.api('GET', '/p/frontend/tasks'), [])
        self.refused('DELETE', '/projects/.', None, 'always stays')
        # The board's stream now merges both projects.
        events = self.stream()
        projects = set()
        while True:
            ident, data = self.next_event(events)
            if ident == 'ready': break
            # D-87: the root's records come with the snapshot, with no project to name.
            if 'milestones' in data or 'features' in data or 'vision' in data or 'strategyRuns' in data: continue
            projects.add(data['project'])
        self.assertEqual(projects, {'.', 'frontend'})
        # The root project's agents can't reach into the project below it, only read it.
        self.assertIn('E6006', self.cli('read', 'frontend/gleam.toml', ok=False))
        self.assertIn('pub fn main()', self.cli('view', 'frontend.main', '--project', 'frontend'))

    def test_a_root_without_gleam_is_a_project_of_plain_files(self):
        project(self.root/'app')
        (self.root/'DECISIONS.md').write_text('# Decisions\n')
        (self.root/'tool.py').write_text('print("ran")\n')
        self.migrate('--project', 'app')
        settings = (self.root/'solace.toml').read_text()
        self.assertIn('projects = ["app"]', settings)
        self.assertIn('watch_limit', settings)
        self.assertTrue((self.root/'.solace/.gitignore').exists())
        with open(self.root/'solace.toml', 'a') as f: f.write('tool = { run = ["python3", "tool.py"] }\n')
        self.assertIn('plain files', self.cli('session', 'start'))
        self.assertIn('app (pilot)', self.cli('session', 'start'))
        self.assertIn('E6013', self.cli('check', ok=False))
        refused = self.cli('view', 'app.one', '--project', '.', cwd=self.root/'app', ok=False)
        self.assertIn('E6013', refused)
        self.assertIn('`view <file>::<name>`', refused)
        self.assertIn('# Decisions', self.cli('read', 'DECISIONS.md', '--project', '.', cwd=self.root/'app'))
        self.start()
        steps = [{'command': 'session start'}, {'command': 'read DECISIONS.md'},
                 {'command': 'write DECISIONS.md', 'text': '# Decisions\n\n## D-1 Root files are reachable\n'},
                 {'command': 'run tool'}, {'command': 'write app/src/app.gleam', 'text': 'pub fn one() { 2 }\n'},
                 {'command': 'check'}, {'command': 'task done recorded'}]
        t = self.api('POST', '/tasks', {'title': 'Record a decision', 'body': json.dumps(steps)})
        self.api('POST', f'/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        t = self.wait('', t, lambda x: x['column'] == 'needs_review')
        replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'.solace'/(t['runs'][-1]['id'] + '.responses')).read_text().splitlines()]
        self.assertIn('plain files', replies[0])
        self.assertIn('# Decisions', replies[1])
        self.assertIn('ran', replies[3])
        self.assertIn('E6006', replies[4])
        self.assertIn('E6013', replies[5])
        self.assertIn('D-1 Root files are reachable', (self.root/'DECISIONS.md').read_text())
        self.assertIn('{ 1 }', (self.root/'app/src/app.gleam').read_text())
        # A subproject's agent reads the root's files but can't change them.
        self.assertIn('E6006', self.cli('write', '../DECISIONS.md', cwd=self.root/'app', input='x', ok=False))
        # Undoing the task restores the root's files; there is no code to type-check.
        self.api('POST', f'/tasks/{t["id"]}/actions', {'action': 'cancel'})
        self.assertEqual((self.root/'DECISIONS.md').read_text(), '# Decisions\n')

    def test_a_file_patch_changes_several_files_and_cancel_undoes_them(self):
        # D-104: `write` with no path takes a patch of several files; the task records each, and cancel restores them.
        project(self.root/'app')
        (self.root/'DECISIONS.md').write_text('# Decisions\n')
        (self.root/'old.txt').write_text('old\n')
        self.migrate('--project', 'app')
        self.start()
        patch = ('*** Begin Patch\n*** Update File: DECISIONS.md\n@@ # Decisions\n+\n+## D-1 Patched\n'
                 '*** Add File: docs/new.md\n+new\n*** Delete File: old.txt\n*** End Patch\n')
        steps = [{'command': 'write', 'text': patch}, {'command': 'task done patched'}]
        t = self.api('POST', '/tasks', {'title': 'Patch files', 'body': json.dumps(steps)})
        self.api('POST', f'/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        t = self.wait('', t, lambda x: x['column'] == 'needs_review')
        replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'.solace'/(t['runs'][-1]['id'] + '.responses')).read_text().splitlines()]
        self.assertIn('Wrote DECISIONS.md\nAdded docs/new.md\nRemoved old.txt\n', replies[0])
        self.assertEqual((self.root/'DECISIONS.md').read_text(), '# Decisions\n\n## D-1 Patched\n')
        self.assertEqual((self.root/'docs/new.md').read_text(), 'new\n')
        self.assertFalse((self.root/'old.txt').exists())
        self.api('POST', f'/tasks/{t["id"]}/actions', {'action': 'cancel'})
        self.assertEqual((self.root/'DECISIONS.md').read_text(), '# Decisions\n')
        self.assertFalse((self.root/'docs/new.md').exists())
        self.assertEqual((self.root/'old.txt').read_text(), 'old\n')

    def test_a_refused_file_patch_is_fixed_in_its_draft_and_recorded(self):
        # D-120: a refused `write` patch is kept as a draft; a `--replace` on it retries the whole patch, recorded as the task's change.
        project(self.root/'app')
        (self.root/'DECISIONS.md').write_text('# Decisions\n')
        (self.root/'notes.md').write_text('A failed read does not stop the call (D-109).\n')
        self.migrate('--project', 'app')
        self.start()
        patch = ('*** Begin Patch\n*** Update File: DECISIONS.md\n@@ # Decisions\n+\n+## D-1 Patched\n'
                 '*** Update File: notes.md\n-does not stop the call (D-109).\n+does not stop the call (D-119).\n*** End Patch\n')
        steps = [{'command': 'write', 'text': patch}, {'command': 'drafts'},
                 {'command': 'write --draft {draft} --replace', 'old': '-does not stop the call (D-109).\n+does',
                  'text': '-A failed read does not stop the call (D-109).\n+A failed read does'},
                 {'command': 'task done patched'}]
        t = self.api('POST', '/tasks', {'title': 'Patch files', 'body': json.dumps(steps)})
        self.api('POST', f'/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        t = self.wait('', t, lambda x: x['column'] == 'needs_review')
        replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'.solace'/(t['runs'][-1]['id'] + '.responses')).read_text().splitlines()]
        self.assertIn('is only part of line 1: `A failed read', replies[0])
        self.assertIn('["write"] (current)', replies[1])
        self.assertIn('Wrote DECISIONS.md\nWrote notes.md\n', replies[2])
        self.assertEqual((self.root/'notes.md').read_text(), 'A failed read does not stop the call (D-119).\n')
        self.api('POST', f'/tasks/{t["id"]}/actions', {'action': 'cancel'})
        self.assertEqual((self.root/'DECISIONS.md').read_text(), '# Decisions\n')
        self.assertEqual((self.root/'notes.md').read_text(), 'A failed read does not stop the call (D-109).\n')

    def test_a_file_patch_draft_refused_for_its_form_is_recorded_once_fixed(self):
        # D-120: a draft kept for a line with no ` `, `-` or `+` doesn't parse; its fixed text names the files to record.
        project(self.root/'app')
        (self.root/'DECISIONS.md').write_text('# Decisions\n')
        (self.root/'notes.md').write_text('A failed read\n')
        self.migrate('--project', 'app')
        self.start()
        patch = ('*** Begin Patch\n*** Update File: DECISIONS.md\n@@ # Decisions\n+\n+## D-1 Patched\n'
                 '*** Update File: notes.md\nA failed read\n+more\n*** End Patch\n')
        steps = [{'command': 'write', 'text': patch},
                 {'command': 'write --draft {draft} --replace', 'old': '\nA failed read\n', 'text': '\n@@ A failed read\n'},
                 {'command': 'task done patched'}]
        t = self.api('POST', '/tasks', {'title': 'Patch files', 'body': json.dumps(steps)})
        self.api('POST', f'/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        t = self.wait('', t, lambda x: x['column'] == 'needs_review')
        replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'.solace'/(t['runs'][-1]['id'] + '.responses')).read_text().splitlines()]
        self.assertIn('each line starts with', replies[0])
        self.assertIn('Wrote DECISIONS.md\nWrote notes.md\n', replies[1])
        self.assertEqual((self.root/'notes.md').read_text(), 'A failed read\nmore\n')
        self.api('POST', f'/tasks/{t["id"]}/actions', {'action': 'cancel'})
        self.assertEqual((self.root/'DECISIONS.md').read_text(), '# Decisions\n')
        self.assertEqual((self.root/'notes.md').read_text(), 'A failed read\n')

    def test_every_root_gets_its_own_lane(self):
        # A root from before D-60: its solace.toml only lists projects, and it has no store.
        project(self.root/'app')
        self.migrate('--project', 'app')
        (self.root/'.solace/store.db').unlink()
        (self.root/'solace.toml').write_text('projects = ["app"]\n')
        self.start()
        self.assertEqual([p['dir'] for p in self.api('GET', '/projects')['projects']], ['.', 'app'])
        settings = (self.root/'solace.toml').read_text()
        self.assertIn('projects = ["app"]', settings)
        self.assertIn('watch_limit', settings)
        self.assertEqual(self.api('GET', '/tasks'), [])
        # A new project made in an empty folder makes that folder a root with its own lane.
        empty = Path(self.tmp.name)/'empty'
        empty.mkdir()
        self.start(empty)
        self.assertEqual(self.api('GET', '/projects')['projects'], [])
        self.assertFalse((empty/'.solace').exists())
        v = self.api('POST', '/projects/new', {'name': 'fresh'})
        self.assertEqual([p['dir'] for p in v['projects']], ['.', 'fresh'])
        self.assertTrue((empty/'.solace/store.db').exists())

    def test_agents_work_in_parallel_and_read_across_projects(self):
        project(self.root/'a', 'alpha', 'pub fn one() { 1 }\n')
        project(self.root/'b', 'beta', '/// The answer.\npub fn answer() { 42 }\n')
        self.migrate()
        # From a shell, reads reach another project; changes don't.
        self.assertIn('pub fn answer() { 42 }', self.cli('view', 'app.answer', '--project', 'b', cwd=self.root/'a'))
        self.assertIn('version="1.0.0"', self.cli('read', 'gleam.toml', '--project', 'b/', cwd=self.root/'a'))
        self.assertIn('E6007', self.cli('submit', 'app.answer', '--project', 'b', cwd=self.root/'a', input='pub fn answer() { 0 }', ok=False))
        self.assertIn('E6007', self.cli('view', 'app.one', '--project', 'nope', cwd=self.root/'a', ok=False))
        # A path out of the project says how to reach another one, not that it is Solace's own.
        out = self.cli('read', '../b/gleam.toml', cwd=self.root/'a', ok=False)
        self.assertIn('outside this project', out)
        self.assertIn('task ask', out)
        # Humans read any project from the root folder too.
        self.assertIn('pub fn one() { 1 }', self.cli('view', 'app.one', '--project', 'a'))
        self.start()
        tasks = {}
        for d in ['a', 'b']:
            steps = [{'command': 'session start'}, {'command': 'view app.answer --project b'}, {'command': 'find answer --project b'},
                     {'command': 'submit app.answer --project b', 'text': 'pub fn answer() { 0 }'},
                     {'sleep': 1.5}, {'command': 'task done work done'}]
            t = self.api('POST', f'/p/{d}/tasks', {'title': 'Read ' + d, 'body': json.dumps(steps)})
            self.api('POST', f'/p/{d}/tasks/{t["id"]}/actions', {'action': 'plan'})
            tasks[d] = self.api('POST', f'/p/{d}/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        # Both agents are working at the same time, one per project.
        busy = lambda x: any(r['state'] != 'ended' for r in x['runs'])
        self.assertTrue(busy(self.api('GET', f'/p/a/tasks/{tasks["a"]["id"]}')))
        self.assertTrue(busy(self.api('GET', f'/p/b/tasks/{tasks["b"]["id"]}')))
        for d in ['a', 'b']:
            t = self.wait(f'/p/{d}', tasks[d], lambda x: x['column'] == 'needs_review')
            rid = t['runs'][-1]['id']
            replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/d/'.solace'/(rid + '.responses')).read_text().splitlines()]
            if d == 'a':
                self.assertIn('You are in a (alpha).', replies[0])
                self.assertIn('Read only: b (beta), . (the root).', replies[0])
                self.assertIn('(`--project b`, not `../b`)', replies[0])
                self.assertIn('ask the human with `task ask` to add that project', replies[0])
            self.assertIn('pub fn answer() { 42 }', replies[1])
            self.assertIn('The answer', replies[2])
            # A task changes the projects it covers (D-61): a's doesn't cover b; b's own is b.
            self.assertIn('E6007' if d == 'a' else 'Saved app.answer', replies[3])
        self.assertNotIn('42', (self.root/'b/src/app.gleam').read_text())

    def test_a_task_across_projects_starts_where_its_symbols_are(self):  # D-123
        project(self.root/'a', 'alpha', 'pub fn ledger() { 1 }\n')
        project(self.root/'b', 'beta', 'pub fn two() { 2 }\n')
        (self.root/'b/src/plan.gleam').write_text('/// The plan.\npub fn answer() { 42 }\n')
        self.migrate()
        self.start()
        # The task is at home in a, but names a symbol only b has.
        steps = [{'command': 'session start --task', 'text': 'Change plan.answer; see ledger'},
                 {'command': 'view plan.answer'}, {'command': 'view ledger'}, {'command': 'task done looked'}]
        t = self.api('POST', '/p/a/tasks', {'title': 'Plan', 'body': json.dumps(steps), 'scope': ['a', 'b']})
        self.api('POST', f'/p/a/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/p/a/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        t = self.wait('/p/a', t, lambda x: x['column'] == 'needs_review')
        rid = t['runs'][-1]['id']
        self.assertFalse((self.root/'a/.solace'/(rid + '.prompt')).exists())
        self.assertIn('You work in b', (self.root/'b/.solace'/(rid + '.prompt')).read_text())
        replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'b/.solace'/(rid + '.responses')).read_text().splitlines()]
        # The session says where the agent is, what it may change, and how to reach the rest.
        self.assertIn('You are in b (beta).', replies[0])
        self.assertIn('This task can change: a (alpha), b (beta).', replies[0])
        self.assertIn('(`--project a`, not `../a`), for that line only', replies[0])
        self.assertIn('relative to the project the line runs in', replies[0])
        # Its symbols come from both projects, the other one's labelled.
        self.assertIn('-- plan.answer\n', replies[0])
        self.assertIn('-- app.ledger (a)\n', replies[0])
        self.assertIn('pub fn answer() { 42 }', replies[1])
        # A name from the other project says where it is.
        self.assertIn('E1001 unknown name: ledger\nfound in a: add --project a', replies[2])
        self.assertEqual(t['column'], 'needs_review')

    def test_a_strategy_run_in_a_folder_of_projects(self):  # D-93
        project(self.root/'a', 'alpha', 'pub fn one() { 1 }\n')
        project(self.root/'b', 'beta', 'pub fn answer() { 42 }\n')
        self.migrate()
        (self.root/'DECISIONS.md').write_text('# Decisions\n## D-1 First thing\nWhy: x\n')
        self.start()
        f = self.api('POST', '/features', {'title': 'Feature F1'})
        steps = [{'command': 'strategy list'}, {'command': 'read DECISIONS.md'}, {'command': 'view app.answer --project b'},
                 {'command': 'help'}, {'command': 'strategy note', 'text': 'Read it'}]
        run = self.api('POST', '/strategy/runs', {'model': 'fake', 'text': json.dumps(steps), 'context': {'kind': 'page'}})
        rid = run['id']
        end = time.monotonic() + 20
        while self.api('GET', f'/strategy/runs/{rid}')['state'] != 'waiting' and time.monotonic() < end: time.sleep(.05)
        replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'.solace'/(rid + '.responses')).read_text().splitlines()]
        self.assertIn(f"Features:\n{f['id']} proposed m:- Feature F1", replies[0])
        # Serving makes the root a project of plain files too (D-60), so the root's decisions can be read.
        self.assertIn('## D-1 First thing', replies[1])
        self.assertIn('pub fn answer() { 42 }', replies[2])
        self.assertIn('strategy note', replies[3]); self.assertNotIn('submit (a patch)', replies[3]); self.assertNotIn('task ask', replies[3])
        self.assertEqual(len(replies), 4)
        prompt = (self.root/'.solace'/(rid + '.prompt')).read_text()
        self.assertIn('- D-1 First thing', prompt); self.assertIn('`read DECISIONS.md --lines a..b`', prompt)
        self.assertNotIn('folder of projects', prompt)
        self.assertEqual(self.api('POST', f'/strategy/runs/{rid}/end')['outcome'], 'planned')
        end = time.monotonic() + 10
        while not (self.root/'.solace'/(rid + '.exited')).exists() and time.monotonic() < end: time.sleep(.05)
        self.assertTrue((self.root/'.solace'/(rid + '.exited')).exists())

    def test_a_question_is_alone_in_its_call_and_a_settings_reason_is_part_of_it(self):
        # A note beside a question is refused with nothing recorded; the why goes in `reason`.
        for d in 'ab': project(self.root/d, d, 'pub fn one() { 1 }\n')
        self.migrate()
        self.start()
        edit = {'old': '[commands]', 'text': '[commands]\ntool = { run = ["true"] }'}
        steps = [{'command': 'task note beside\nwrite solace.toml --project b', **edit},
                 {'command': 'task note beside\ntask ask', 'text': 'Which?', 'options': ['a', 'b']},
                 {'command': 'write solace.toml --project b', 'reason': 'Agents need the tool.', **edit},
                 {'command': 'task done proposed'}]
        t = self.api('POST', '/p/a/tasks', {'title': 'Settings', 'body': json.dumps(steps), 'scope': ['a', 'b']})
        self.api('POST', f'/p/a/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/p/a/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        t = self.wait('/p/a', t, lambda x: x['runs'][-1]['state'] == 'waiting')
        self.assertEqual(len(t['questions']), 1)
        self.assertEqual(t['questions'][0]['text'], f'Change {self.root.name}/b/solace.toml\n\nAgents need the tool.')
        self.assertFalse([n for n in t['notes'] if 'beside' in n.get('text', '')])
        rid = t['runs'][-1]['id']
        replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'a/.solace'/(rid + '.responses')).read_text().splitlines()]
        self.assertIn('E7017', replies[0])
        self.assertIn('E7017', replies[1])

    def test_an_agent_proposes_settings_for_a_project_of_its_task(self):
        # D-61: a settings proposal can name another project in the task; `old` alone is an exact edit.
        for d in 'ab': project(self.root/d, d, 'pub fn one() { 1 }\n')
        self.migrate()
        self.start()
        steps = [{'command': 'write solace.toml --project b', 'old': '[commands]', 'text': '[commands]\ntool = { run = ["true"] }'},
                 {'command': 'task done proposed'}]
        t = self.api('POST', '/p/a/tasks', {'title': 'Settings', 'body': json.dumps(steps), 'scope': ['a', 'b']})
        self.api('POST', f'/p/a/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/p/a/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        t = self.wait('/p/a', t, lambda x: x['runs'][-1]['state'] == 'waiting')
        q = t['questions'][0]
        self.assertEqual(q['text'], f'Change {self.root.name}/b/solace.toml')
        before = (self.root/'b/solace.toml').read_text()
        self.assertEqual(q['settings']['proposed'], before.replace('[commands]', '[commands]\ntool = { run = ["true"] }'))
        self.api('POST', f'/p/a/tasks/{t["id"]}/questions/{q["id"]}', {'option': 'Approve'})
        self.wait('/p/a', t, lambda x: x['column'] == 'needs_review')
        self.assertIn('tool = { run = ["true"] }', (self.root/'b/solace.toml').read_text())
        self.assertNotIn('tool =', (self.root/'a/solace.toml').read_text())

    def test_a_task_changes_every_project_it_covers(self):
        # D-61: one task across a and b; c stays out of its reach.
        for d in 'abc': project(self.root/d, d, 'pub fn one() { 1 }\n')
        self.migrate()
        (self.root/'b/solace.toml').write_text((self.root/'b/solace.toml').read_text() + '\n[board]\nevents = { asked = "x", finished = "x", failed = "x" }\n[[board.columns]]\nid = "x"\n')
        self.start()
        # One workflow for the board: the root's.
        self.assertEqual(self.api('GET', '/p/b/workflow'), self.api('GET', '/workflow'))
        self.refused('POST', '/p/a/tasks', {'title': 'T', 'scope': ['b', 'nope']}, 'not a project on this board')
        self.refused('POST', '/p/a/tasks', {'title': 'T', 'scope': []}, 'at least one project')
        self.refused('POST', '/p/a/tasks', {'title': 'T', 'scope': ['b']}, 'post it to one of b')
        steps = [{'command': 'submit app.one', 'text': 'pub fn one() { 2 }'},
                 {'command': 'submit app.one --project b', 'text': 'pub fn one() { 3 }'},
                 {'command': 'test --project b'},
                 {'command': 'submit app.one --project c', 'text': 'pub fn one() { 4 }'},
                 {'command': 'task diff'},
                 {'sleep': 1.5}, {'command': 'task done changed a and b'}]
        t = self.api('POST', '/p/a/tasks', {'title': 'Across', 'body': json.dumps(steps), 'scope': ['b', 'a']})
        self.assertEqual(t['scope'], ['a', 'b'])
        other = self.api('POST', '/p/b/tasks', {'title': 'Also in b', 'body': '[]'})
        for x, base in [(t, '/p/a'), (other, '/p/b')]:
            self.api('POST', f'{base}/tasks/{x["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/p/a/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        # The run holds both projects: b's other task can't start work, and the scope can't change mid-run.
        self.refused('POST', f'/p/b/tasks/{other["id"]}/actions', {'action': 'assign', 'model': 'fake'}, 'another task has a work run')
        self.refused('POST', f'/p/a/tasks/{t["id"]}/scope', {'scope': ['a', 'b', 'c']}, 'an agent is working')
        t = self.wait('/p/a', t, lambda x: x['column'] == 'needs_review')
        rid = t['runs'][-1]['id']
        replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'a/.solace'/(rid + '.responses')).read_text().splitlines()]
        self.assertIn('Saved app.one', replies[1])
        self.assertIn('exit code: 0', replies[2])
        self.assertIn("not one of this task's projects (a, b)", replies[3])
        self.assertIn('## a: app.one (changed)', replies[4])
        self.assertIn('## b: app.one (changed)', replies[4])
        self.assertEqual(sorted((c['project'], c['name']) for c in t['changes']), [('a', 'app.one'), ('b', 'app.one')])
        self.assertIn('3', (self.root/'b/src/app.gleam').read_text())
        self.assertIn('{ 1 }', (self.root/'c/src/app.gleam').read_text())
        # The Contract view in b finds the task and marks b's change.
        g = self.api('GET', f'/p/b/graph?task={t["id"]}')
        self.assertTrue(any(n.get('changed') for n in g['nodes']))
        # Review baselines distinguish identical names in separate projects.
        url = f'/p/a/tasks/{t["id"]}'
        def review(steps):
            self.api('PATCH', url, {'body':json.dumps(steps)})
            self.api('POST', url+'/actions', {'action':'review', 'model':'fake'})
            value = self.wait('/p/a', t, lambda x:x['runs'][-1]['state']=='ended')
            rid = value['runs'][-1]['id']
            prompt = (self.root/'a/.solace'/(rid+'.prompt')).read_text()
            replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'a/.solace'/(rid+'.responses')).read_text().splitlines()]
            return prompt, replies
        review([{'command':'task note baseline'}])
        self.api('PATCH', url, {'body':json.dumps([
            {'command':'submit app.one --project b','text':'pub fn one() { 30 }'},
            {'command':'task done repaired b'},
        ])})
        self.api('POST', url+'/actions', {'action':'reassign'})
        self.wait('/p/a', t, lambda x:x['column']=='needs_review')
        _, replies = review([{'command':'task diff --since-review app.one'}, {'command':'task note checked b'}])
        self.assertIn('## b: app.one (changed)', replies[0])
        self.assertNotIn('## a:', replies[0])
        self.assertIn('-   3', replies[0]); self.assertIn('+   30', replies[0])
        self.api('POST', url+'/scope', {'scope':['a', 'b', 'c']})
        prompt, replies = review([{'command':'task diff --since-review'}, {'command':'task note checked scope'}])
        self.assertIn('project scope changed since review', prompt)
        self.assertIn('Full task diff', replies[0])
        self.assertIn('## a: app.one', replies[0]); self.assertIn('## b: app.one', replies[0])
        # Between runs the human widens and narrows the scope, but keeps projects it changed.
        self.assertEqual(self.api('POST', f'/p/a/tasks/{t["id"]}/scope', {'scope': ['a', 'b', 'c']})['scope'], ['a', 'b', 'c'])
        self.assertEqual(self.api('POST', f'/p/a/tasks/{t["id"]}/scope', {'scope': ['a', 'b']})['scope'], ['a', 'b'])
        self.refused('POST', f'/p/a/tasks/{t["id"]}/scope', {'scope': ['a']}, 'this task has changed b')
        self.refused('DELETE', '/projects/b', None, 'also covers b')
        # Canceling undoes the changes in both projects.
        self.api('POST', f'/p/a/tasks/{t["id"]}/actions', {'action': 'cancel'})
        self.assertIn('{ 1 }', (self.root/'a/src/app.gleam').read_text())
        self.assertIn('{ 1 }', (self.root/'b/src/app.gleam').read_text())
        # A task whose home leaves its scope moves to the first project left.
        m = self.api('POST', '/p/a/tasks', {'title': 'Moves', 'scope': ['a', 'c']})
        self.assertEqual(self.api('POST', f'/p/a/tasks/{m["id"]}/scope', {'scope': ['c']})['scope'], ['c'])
        self.assertEqual(self.api('GET', f'/p/c/tasks/{m["id"]}')['title'], 'Moves')
        self.refused('GET', f'/p/a/tasks/{m["id"]}', None, 'unknown task')

    def test_a_move_between_the_projects_a_task_covers(self):
        # D-84: a move crosses projects only within the task's scope; each side is recorded in its own project.
        for d in 'abc': project(self.root/d, d, 'pub fn one() { 1 }\n')
        self.migrate()
        (self.root/'a/notes.txt').write_text('notes')
        self.start()
        steps = [{'command': 'move notes.txt docs/notes.txt --to-project b'},
                 {'command': 'move notes.txt docs/notes.txt --to-project b'},
                 {'command': 'move docs/notes.txt notes.txt --project b --to-project c'},
                 {'command': 'task diff'},
                 {'command': 'task done moved'}]
        t = self.api('POST', '/p/a/tasks', {'title': 'Move', 'body': json.dumps(steps), 'scope': ['a', 'b']})
        self.api('POST', f'/p/a/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/p/a/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        t = self.wait('/p/a', t, lambda x: x['column'] == 'needs_review')
        rid = t['runs'][-1]['id']
        replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'a/.solace'/(rid + '.responses')).read_text().splitlines()]
        self.assertIn('Moved a: notes.txt to b: docs/notes.txt', replies[0])
        self.assertIn('Already moved', replies[1])
        self.assertIn("not one of this task's projects (a, b)", replies[2])
        self.assertIn('a: file:notes.txt (deleted)', replies[3])
        self.assertIn('b: file:docs/notes.txt (added)', replies[3])
        self.assertEqual((self.root/'b/docs/notes.txt').read_text(), 'notes')
        self.assertFalse((self.root/'c/notes.txt').exists())
        self.api('POST', f'/p/a/tasks/{t["id"]}/actions', {'action': 'cancel'})
        self.assertEqual((self.root/'a/notes.txt').read_text(), 'notes')
        self.assertFalse((self.root/'b/docs/notes.txt').exists())

    def test_an_extract_into_another_project_of_the_task(self):
        # D-103: extract --to-project moves symbols with their identity into a project the task
        # covers; the source gains the dependency, a project that uses moved code is refused,
        # and cancel undoes both stores and the source's gleam.toml.
        board = ('pub type Task {\n  Task(id: Int, lane: Int)\n}\n\npub fn new_task(id: Int) -> Task {\n  Task(id, 0)\n}\n\n'
                 'pub fn lane(t: Task) -> Int {\n  t.lane\n}\n')
        files = {'web/src/board.gleam': board,
                 'web/test/board_test.gleam': 'import board\n\npub fn round_trip_test() {\n  let t = board.Task(1, 2)\n  assert t.id == 1\n}\n',
                 'components/src/solace_task.gleam': 'pub fn z() -> Int {\n  0\n}\n',
                 'front/src/front.gleam': 'import board\n\npub fn view(t) -> Int {\n  board.lane(t)\n}\n'}
        for d in ['web', 'components', 'front']:
            project(self.root/d, d)
            (self.root/d/'src/app.gleam').unlink()
        for path, text in files.items():
            (self.root/path).parent.mkdir(parents=True, exist_ok=True)
            (self.root/path).write_text(text)
        with open(self.root/'front/gleam.toml', 'a') as f: f.write('\n[dependencies]\nweb = { path = "../web" }\n')
        web_toml = (self.root/'web/gleam.toml').read_text()
        self.migrate()
        self.start()
        steps = [{'command': 'extract board.Task board.new_task board.lane --to solace_task --to-project components'},
                 {'command': 'extract board.Task board.new_task --to solace_task --to-project components'},
                 {'command': 'check'},
                 {'command': 'task diff'},
                 {'command': 'task done extracted'}]
        t = self.api('POST', '/p/web/tasks', {'title': 'Extract', 'body': json.dumps(steps), 'scope': ['web', 'components']})
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        t = self.wait('/p/web', t, lambda x: x['column'] == 'needs_review')
        rid = t['runs'][-1]['id']
        replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'web/.solace'/(rid + '.responses')).read_text().splitlines()]
        self.assertIn('E5003', replies[0])
        self.assertIn('front: front.view uses board.lane', replies[0])
        self.assertIn('Moved board.Task, board.new_task to solace_task in components', replies[1])
        self.assertIn('Dependency added: web -> components', replies[1])
        self.assertIn('Tests moved: board_test.round_trip_test -> solace_task_test', replies[1])
        self.assertIn('No errors', replies[2])
        # A moved symbol is shown once with its diff, where it landed; its source says where it went.
        self.assertIn('## components: solace_task.Task (added)', replies[3])
        self.assertIn('## web: board.Task (moved to components)', replies[3])
        self.assertNotIn('board.Task (deleted)', replies[3])
        self.assertIn('components = { path = "../components" }', (self.root/'web/gleam.toml').read_text())
        self.assertNotIn('[dependencies]', (self.root/'components/gleam.toml').read_text())
        self.assertIn('solace_task.Task', (self.root/'web/src/board.gleam').read_text())
        self.assertIn('pub type Task', (self.root/'components/src/solace_task.gleam').read_text())
        self.assertTrue((self.root/'components/test/solace_task_test.gleam').exists())
        live = lambda d: sqlite3.connect(self.root/d/'.solace/store.db').execute("SELECT id FROM symbols WHERE name='Task' AND live=1").fetchall()
        task_id = live('components')
        self.assertEqual(len(task_id), 1)
        self.assertEqual(live('web'), [])
        # Canceling undoes both stores and the source's gleam.toml.
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'cancel'})
        self.assertEqual((self.root/'web/gleam.toml').read_text(), web_toml)
        self.assertIn('pub type Task', (self.root/'web/src/board.gleam').read_text())
        self.assertNotIn('Task', (self.root/'components/src/solace_task.gleam').read_text())
        self.assertFalse((self.root/'components/test/solace_task_test.gleam').exists())
        self.assertEqual(live('web'), task_id)
        self.assertEqual(live('components'), [])

    def test_a_crash_between_an_extracts_projects_is_undone_by_cancel(self):
        # D-103: the destination and a dependent the task covers commit before the source;
        # a crash between them leaves their changes recorded, and cancel puts all three back.
        board = ('pub type Task {\n  Task(id: Int, lane: Int)\n}\n\npub fn new_task(id: Int) -> Task {\n  Task(id, 0)\n}\n\n'
                 'pub fn lane(t: Task) -> Int {\n  t.lane\n}\n')
        front = 'import board\n\npub fn view(t: board.Task) -> Int {\n  board.lane(t)\n}\n'
        components = 'pub fn z() -> Int {\n  0\n}\n'
        files = {'web/src/board.gleam': board, 'components/src/solace_task.gleam': components, 'front/src/front.gleam': front}
        for d in ['web', 'components', 'front']:
            project(self.root/d, d)
            (self.root/d/'src/app.gleam').unlink()
        for path, text in files.items():
            (self.root/path).parent.mkdir(parents=True, exist_ok=True)
            (self.root/path).write_text(text)
        with open(self.root/'front/gleam.toml', 'a') as f: f.write('\n[dependencies]\nweb = { path = "../web" }\n')
        tomls = {d: (self.root/d/'gleam.toml').read_text() for d in ['web', 'components', 'front']}
        self.migrate()
        self.env['SOLACE_FAIL_AFTER'] = 'others'
        self.start()
        db = lambda d: sqlite3.connect(self.root/d/'.solace/store.db')
        live = lambda d, name: db(d).execute("SELECT id FROM symbols WHERE name=? AND live=1", [name]).fetchall()
        task_id = live('web', 'Task')
        steps = [{'command': 'extract board.Task board.new_task board.lane --to solace_task --to-project components'}]
        t = self.api('POST', '/p/web/tasks', {'title': 'Extract', 'body': json.dumps(steps), 'scope': ['web', 'components', 'front']})
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        self.wait('/p/web', t, lambda x: x['runs'] and x['runs'][-1]['state'] == 'ended')
        # The destination and the dependent committed; the source did not.
        self.assertEqual(live('components', 'Task'), task_id)
        self.assertIn('solace_task', db('front').execute("SELECT text FROM symbols WHERE name='view' AND live=1").fetchone()[0])
        self.assertEqual(live('web', 'Task'), task_id)
        # Cancel puts every project back: code, files and gleam.toml.
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'cancel'})
        for path, text in files.items():
            self.assertEqual((self.root/path).read_text(), text, path)
        for d, text in tomls.items():
            self.assertEqual((self.root/d/'gleam.toml').read_text(), text, d)
        self.assertEqual(live('web', 'Task'), task_id)
        self.assertEqual(live('components', 'Task'), [])
        self.assertFalse((self.root/'components/test/solace_task_test.gleam').exists())

    def test_a_merge_folds_a_duplicate_into_another_project(self):
        # D-117: extract refuses a name the target has and names merge; merge redirects every use,
        # in the source and in a dependent the task covers, to the survivor in another project,
        # retires the duplicate, and cancel undoes all three projects.
        task_type = 'pub type Task {\n  Task(id: Int, lane: Int)\n}\n'
        board = task_type + '\npub fn new_task(id: Int) -> Task {\n  Task(id, 0)\n}\n\npub fn lane(t: Task) -> Int {\n  t.lane\n}\n'
        front = 'import board.{type Task}\n\npub fn view(t: Task) -> Int {\n  board.lane(t)\n}\n'
        files = {'web/src/board.gleam': board,
                 'web/test/board_test.gleam': 'import board\n\npub fn round_trip_test() {\n  let t = board.Task(1, 2)\n  assert t.id == 1\n}\n',
                 'components/src/solace_task.gleam': task_type,
                 'front/src/front.gleam': front}
        for d in ['web', 'components', 'front']:
            project(self.root/d, d)
            (self.root/d/'src/app.gleam').unlink()
        for path, text in files.items():
            (self.root/path).parent.mkdir(parents=True, exist_ok=True)
            (self.root/path).write_text(text)
        with open(self.root/'web/gleam.toml', 'a') as f: f.write('\n[dependencies]\ncomponents = { path = "../components" }\n')
        with open(self.root/'front/gleam.toml', 'a') as f: f.write('\n[dependencies]\nweb = { path = "../web" }\ncomponents = { path = "../components" }\n')
        self.migrate()
        self.start()
        live = lambda d: sqlite3.connect(self.root/d/'.solace/store.db').execute("SELECT id FROM symbols WHERE name='Task' AND live=1").fetchall()
        web_task, components_task = live('web'), live('components')
        steps = [{'command': 'extract board.Task --to solace_task --to-project components'},
                 {'command': 'merge solace_task.Task board.Task --to-project components'},
                 {'command': 'check'},
                 {'command': 'task done merged'}]
        t = self.api('POST', '/p/web/tasks', {'title': 'Merge', 'body': json.dumps(steps), 'scope': ['web', 'components', 'front']})
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        t = self.wait('/p/web', t, lambda x: x['column'] == 'needs_review')
        rid = t['runs'][-1]['id']
        replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'web/.solace'/(rid + '.responses')).read_text().splitlines()]
        self.assertIn('E1004 solace_task.Task already exists', replies[0])
        self.assertIn('to fold board.Task into it, use merge solace_task.Task board.Task --to-project components', replies[0])
        self.assertIn('Merged board.Task into solace_task.Task (components)', replies[1])
        self.assertIn('(2 projects)', replies[1])
        self.assertIn('No errors', replies[2])
        self.assertIn('solace_task.Task', (self.root/'web/src/board.gleam').read_text())
        self.assertNotIn('pub type Task', (self.root/'web/src/board.gleam').read_text())
        self.assertIn('import solace_task.{type Task}', (self.root/'front/src/front.gleam').read_text())
        self.assertIn('solace_task.Task(1, 2)', (self.root/'web/test/board_test.gleam').read_text())
        self.assertEqual(live('web'), [])
        self.assertEqual(live('components'), components_task)
        # Canceling undoes the source and the dependent; the survivor was never changed.
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'cancel'})
        self.assertEqual((self.root/'web/src/board.gleam').read_text(), board)
        self.assertEqual((self.root/'front/src/front.gleam').read_text(), front)
        self.assertEqual(live('web'), web_task)
        self.assertEqual(live('components'), components_task)

    def test_an_extract_into_another_project_carries_its_ffi_file(self):
        # D-103, D-84: a JavaScript file the moved code uses goes along, its prelude import
        # rewritten for its new folder; cancel puts it back as it was.
        board = ('@external(erlang, "erlang", "abs")\n@external(javascript, "./board_ffi.mjs", "abs")\npub fn abs(x: Int) -> Int\n\n'
                 'pub fn keep() -> Int {\n  1\n}\n')
        ffi = 'import { Ok } from "./gleam.mjs";\nexport function abs(x) { return Math.abs(x) }\n'
        files = {'web/src/board.gleam': board, 'web/src/board_ffi.mjs': ffi,
                 'components/src/solace_task.gleam': 'pub fn z() -> Int {\n  0\n}\n'}
        for d in ['web', 'components']:
            project(self.root/d, d)
            (self.root/d/'src/app.gleam').unlink()
        for path, text in files.items():
            (self.root/path).parent.mkdir(parents=True, exist_ok=True)
            (self.root/path).write_text(text)
        self.migrate()
        self.start()
        steps = [{'command': 'extract board.abs --to ui/task --to-project components'},
                 {'command': 'task done extracted'}]
        t = self.api('POST', '/p/web/tasks', {'title': 'Extract', 'body': json.dumps(steps), 'scope': ['web', 'components']})
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        t = self.wait('/p/web', t, lambda x: x['column'] == 'needs_review')
        rid = t['runs'][-1]['id']
        replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'web/.solace'/(rid + '.responses')).read_text().splitlines()]
        self.assertIn('Moved file src/board_ffi.mjs to components: src/ui/board_ffi.mjs (its imports rewritten)', replies[0])
        self.assertIn('no declared suite reaches the moved code', replies[0])
        self.assertFalse((self.root/'web/src/board_ffi.mjs').exists())
        self.assertEqual((self.root/'components/src/ui/board_ffi.mjs').read_text(), ffi.replace('"./gleam.mjs"', '"../gleam.mjs"'))
        self.assertIn('"./board_ffi.mjs"', (self.root/'components/src/ui/task.gleam').read_text())
        # Canceling puts the file back in web, unchanged, and takes it out of components.
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'cancel'})
        self.assertEqual((self.root/'web/src/board_ffi.mjs').read_text(), ffi)
        self.assertFalse((self.root/'components/src/ui/board_ffi.mjs').exists())
        self.assertIn('pub fn abs', (self.root/'web/src/board.gleam').read_text())

    def test_an_extract_rewrites_a_dependent_project_of_the_task(self):
        # D-103: a project that depends on the source and that the task covers has its
        # references rewritten and gains the destination dependency; cancel restores
        # its store and its gleam.toml.
        board = ('pub type Task {\n  Task(id: Int, lane: Int)\n}\n\npub fn new_task(id: Int) -> Task {\n  Task(id, 0)\n}\n\n'
                 'pub fn lane(t: Task) -> Int {\n  t.lane\n}\n\npub fn keep(t: Task) -> Int {\n  t.id\n}\n')
        front = 'import board\n\npub fn view(t) -> Int {\n  board.lane(t) + board.keep(t)\n}\n'
        files = {'web/src/board.gleam': board,
                 'components/src/solace_task.gleam': 'pub fn z() -> Int {\n  0\n}\n',
                 'front/src/front.gleam': front}
        for d in ['web', 'components', 'front']:
            project(self.root/d, d)
            (self.root/d/'src/app.gleam').unlink()
        for path, text in files.items():
            (self.root/path).parent.mkdir(parents=True, exist_ok=True)
            (self.root/path).write_text(text)
        with open(self.root/'front/gleam.toml', 'a') as f: f.write('\n[dependencies]\nweb = { path = "../web" }\n')
        front_toml = (self.root/'front/gleam.toml').read_text()
        self.migrate()
        self.start()
        steps = [{'command': 'extract board.Task board.new_task board.lane --to solace_task --to-project components'},
                 {'command': 'check'},
                 {'command': 'check --project front'},
                 {'command': 'task done extracted'}]
        t = self.api('POST', '/p/web/tasks', {'title': 'Extract', 'body': json.dumps(steps), 'scope': ['web', 'components', 'front']})
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        t = self.wait('/p/web', t, lambda x: x['column'] == 'needs_review')
        rid = t['runs'][-1]['id']
        replies = [json.loads(l)['result']['content'][0]['text'] for l in (self.root/'web/.solace'/(rid + '.responses')).read_text().splitlines()]
        self.assertIn('Moved board.Task, board.new_task, board.lane to solace_task in components', replies[0])
        self.assertIn('Dependency added: web, front -> components', replies[0])
        self.assertIn('Rewrote 1 reference in 1 module in front', replies[0])
        self.assertIn('No errors', replies[1])
        self.assertIn('No errors', replies[2])
        self.assertIn('components = { path = "../components" }', (self.root/'front/gleam.toml').read_text())
        text = (self.root/'front/src/front.gleam').read_text()
        self.assertIn('import solace_task', text)
        self.assertIn('solace_task.lane(t) + board.keep(t)', text)
        # Canceling restores the dependent's store and its gleam.toml.
        self.api('POST', f'/p/web/tasks/{t["id"]}/actions', {'action': 'cancel'})
        self.assertEqual((self.root/'front/gleam.toml').read_text(), front_toml)
        self.assertEqual((self.root/'front/src/front.gleam').read_text(), front)
        self.assertIn('pub fn lane', (self.root/'web/src/board.gleam').read_text())

    def test_old_tasks_keep_scope_conversation_and_changes_after_the_move(self):
        # D-77 point 4: a task saved in a project's old board folder keeps its scope,
        # conversation and changes when serve moves it into the root's task store; the
        # changes are computed from the projects' stores, and cancel undoes them.
        for d in 'ab': project(self.root/d, d, 'pub fn one() { 1 }\n')
        self.migrate()
        self.start()
        steps = [{'command': 'submit app.one', 'text': 'pub fn one() { 2 }'},
                 {'command': 'submit app.one --project b', 'text': 'pub fn one() { 3 }'},
                 {'command': 'task done changed a and b'}]
        t = self.api('POST', '/p/a/tasks', {'title': 'Move', 'body': json.dumps(steps), 'scope': ['a', 'b']})
        self.api('POST', f'/p/a/tasks/{t["id"]}/actions', {'action': 'plan'})
        self.api('POST', f'/p/a/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        t = self.wait('/p/a', t, lambda x: x['column'] == 'needs_review')
        tid = t['id']
        changes = sorted((c['project'], c['name'], c['entity']) for c in t['changes'])
        conversation = t['conversation']
        # The old format: changes in the task file, the file in the project's board
        # folder, and rows in the projects' stores without run or name.
        self.servers[0].terminate(); self.servers[0].wait(timeout=3)
        for d in 'ab':
            db = sqlite3.connect(self.root/d/'.solace/store.db')
            db.execute('UPDATE board_touches SET run=NULL, name=NULL WHERE task=?', [tid]); db.commit(); db.close()
        (self.root/'a/.solace/board'/f'{tid}.json').write_text(json.dumps(t))
        (self.root/'.solace/tasks'/f'{tid}.json').unlink()
        self.start()
        v = self.api('GET', f'/p/a/tasks/{tid}')
        self.assertEqual(v['scope'], ['a', 'b'])
        self.assertEqual(v['conversation'], conversation)
        self.assertEqual(sorted((c['project'], c['name'], c['entity']) for c in v['changes']), changes)
        self.assertNotIn('changes', json.loads((self.root/'.solace/tasks'/f'{tid}.json').read_text()))
        # Undo works: cancel puts both projects back.
        self.api('POST', f'/p/a/tasks/{tid}/actions', {'action': 'cancel'})
        self.assertIn('{ 1 }', (self.root/'a/src/app.gleam').read_text())
        self.assertIn('{ 1 }', (self.root/'b/src/app.gleam').read_text())

    def test_changes_made_outside_solace_wait_for_review(self):
        # D-58: the board's poll reads an outside edit in, in its own project only, and never writes over it.
        body = 'pub fn one() {\n  1\n}\n'
        for d in ['a', 'b']: project(self.root/d, body=body)
        self.migrate()
        self.assertIn('approved', (self.root/'b/.solace/.gitignore').read_text())
        self.start()
        edited = body.replace('  1', '  2')
        (self.root/'b/src/app.gleam').write_text(edited)
        end = time.monotonic() + 20
        while not (found := [t for t in self.api('GET', '/p/b/tasks') if t.get('unexpected')]) and time.monotonic() < end:
            time.sleep(.1)
        self.assertEqual([t['title'] for t in found], ['Unexpected Changes'])
        self.assertEqual(self.api('GET', '/p/a/tasks'), [])
        # The note follows once the compile check ends.
        t = self.wait('/p/b', found[0], lambda v: v['notes'])
        self.assertEqual([c['name'] for c in t['changes']], ['app.one'])
        self.assertIn('Changed outside Solace: app.one', t['notes'][0]['text'])
        self.assertEqual((self.root/'b/src/app.gleam').read_text(), edited)
        other = self.api('POST', '/p/b/tasks', {'title': 'Other'})
        self.api('POST', f'/p/b/tasks/{other["id"]}/actions', {'action': 'plan'})
        self.refused('POST', f'/p/b/tasks/{other["id"]}/actions', {'action': 'assign', 'model': 'fake'}, 'Unexpected Changes')
        self.api('POST', f'/p/b/tasks/{t["id"]}/actions', {'action': 'cancel'})
        self.assertEqual((self.root/'b/src/app.gleam').read_text(), body)
        # A command in the project sees nothing new.
        self.assertNotIn('outside Solace', self.cli('check', cwd=self.root/'b'))

if __name__ == '__main__':
    unittest.main()
