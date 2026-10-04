"""One Solace root (D-57): its projects are the root itself and folders below it, on one board."""
import http.client
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

BIN = os.environ['SOLACE_TEST_BIN']
FAKE = str(Path(__file__).with_name('fake_agent.py'))

def project(path, name='pilot', body='pub fn one() { 1 }\n'):
    (path/'src').mkdir(parents=True, exist_ok=True)
    (path/'gleam.toml').write_text(f'name="{name}"\nversion="1.0.0"\n')
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
        for d in ['a', 'b', 'a/build/packages/dep']: project(self.root/d)
        self.migrate('--project', 'a', '--project', 'b')
        for d in ['a', 'b']:
            self.assertTrue((self.root/d/'.solace/store.db').exists())
            self.assertFalse((self.root.with_name('fixture.pre-solace')/d/'.solace').exists())
        self.assertFalse((self.root/'a/build/packages/dep/.solace').exists())

        self.start()
        self.api('DELETE', '/projects/b')
        v = self.api('GET', '/projects')
        self.assertTrue(v['workspace'])
        self.assertEqual([(p['dir'], p['label'], p['name'], p['base']) for p in v['projects']], [('a', 'a', 'pilot', '/p/a')])
        self.assertEqual([(f['dir'], f['member']) for f in v['found']], [('a', True), ('b', False)])
        self.assertIsInstance(self.api('GET', '/models'), list)

        events = self.stream()
        self.until_ready(events)
        v = self.api('POST', '/projects', {'dir': 'b'})
        self.assertEqual([p['dir'] for p in v['projects']], ['a', 'b'])
        self.assertEqual(self.next_event(events)[1], {'projects': ['a', 'b']})

        self.api('POST', '/p/a/tasks', {'title': 'In a'})
        self.assertEqual([x['title'] for x in self.api('GET', '/p/a/tasks')], ['In a'])
        self.assertEqual(self.api('GET', '/p/b/tasks'), [])
        self.refused('GET', '/p/nope/tasks', None, 'unknown project')
        # No project lives at the root itself, so its routes are the root's own.
        self.refused('GET', '/tasks', None, 'unknown route')
        ident, data = self.next_event(events)
        self.assertEqual((data['project'], data['task']['title']), ('a', 'In a'))
        self.assertEqual(set(json.loads(ident)), {'a', 'b'})

        # A reconnect replays only what it missed, from every project.
        events.close()
        self.api('POST', '/p/b/tasks', {'title': 'In b'})
        events = self.stream(last=ident)
        ident, data = self.next_event(events)
        self.assertEqual((data['project'], data['task']['title']), ('b', 'In b'))
        self.assertEqual(self.next_event(events), ('ready', None))

        # A project's own stream and view, as a project on its own has them.
        single = self.stream('/p/a/events')
        ident, data = self.next_event(single)
        self.assertNotIn('project', data)
        int(ident)
        self.assertIn('<div id="app">', self.call('GET', '/p/a/view')[1])

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

        v = self.api('POST', '/projects/new', {'name': 'fresh'})
        self.assertIn('fresh', [p['dir'] for p in v['projects']])
        for f in ['gleam.toml', 'solace.toml', '.solace/store.db', 'src/fresh.gleam']:
            self.assertTrue((self.root/'fresh'/f).exists(), f)
        self.assertFalse((self.root/'fresh/.git').exists())

        v = self.api('DELETE', '/projects/b')
        self.assertEqual([p['dir'] for p in v['projects']], ['a', 'fresh'])
        self.assertTrue((self.root/'b/.solace/store.db').exists())
        self.api('POST', '/projects', {'dir': 'b'})
        self.assertEqual([t['title'] for t in self.api('GET', '/p/b/tasks')], ['In b'])

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
            self.assertEqual(self.api('GET', '/projects')['projects'], [])
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
        int(ident)
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
            projects.add(data.get('project'))
        self.assertEqual(projects, {'.', 'frontend'})
        # The root project's agents can't reach into the project below it, only read it.
        self.assertIn('E6006', self.cli('read', 'frontend/gleam.toml', ok=False))
        self.assertIn('pub fn main()', self.cli('view', 'frontend.main', '--project', 'frontend'))

    def test_agents_work_in_parallel_and_read_across_projects(self):
        project(self.root/'a', 'alpha', 'pub fn one() { 1 }\n')
        project(self.root/'b', 'beta', '/// The answer.\npub fn answer() { 42 }\n')
        self.migrate()
        # From a shell, reads reach another project; changes don't.
        self.assertIn('pub fn answer() { 42 }', self.cli('view', 'app.answer', '--project', 'b', cwd=self.root/'a'))
        self.assertIn('version="1.0.0"', self.cli('read', 'gleam.toml', '--project', 'b/', cwd=self.root/'a'))
        self.assertIn('E6007', self.cli('submit', 'app.answer', '--project', 'b', cwd=self.root/'a', input='pub fn answer() { 0 }', ok=False))
        self.assertIn('E6007', self.cli('view', 'app.one', '--project', 'nope', cwd=self.root/'a', ok=False))
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
            self.assertIn('pub fn answer() { 42 }', replies[1])
            self.assertIn('The answer', replies[2])
            self.assertIn('E6007', replies[3])
        self.assertIn('{ 42 }', (self.root/'b/src/app.gleam').read_text())

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
