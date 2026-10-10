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
        self.assertFalse(keys & {'milestones', 'features', 'milestone', 'feature'}, keys)
        self.refused('GET', '/p/a/milestones', None, 'unknown route')
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
            if 'milestones' in data or 'features' in data: continue
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
        self.assertIn('E6013', self.cli('view', 'app.one', '--project', '.', cwd=self.root/'app', ok=False))
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
                self.assertIn('Other projects under this Solace root', replies[0])
                self.assertIn('b (beta)', replies[0])
                self.assertIn('ask the human with `task ask` to add that project', replies[0])
            self.assertIn('pub fn answer() { 42 }', replies[1])
            self.assertIn('The answer', replies[2])
            # A task changes the projects it covers (D-61): a's doesn't cover b; b's own is b.
            self.assertIn('E6007' if d == 'a' else 'Saved app.answer', replies[3])
        self.assertNotIn('42', (self.root/'b/src/app.gleam').read_text())

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
