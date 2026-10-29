"""Migrating an existing project (D-59, Spec 17): Solace works in a copy, and the original is only read until it's adopted."""
import fcntl
import hashlib
import http.client
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import time
import unittest

BIN = os.environ['SOLACE_TEST_BIN']

APP_TOML = '''name = "app"
version = "1.0.0"
[dependencies]
gleam_stdlib = ">= 0.44.0 and < 2.0.0"
[dev-dependencies]
gleeunit = ">= 1.0.0 and < 2.0.0"
'''

# Tests whose app has packages and a test module; the rest use a plain app.
WITH_PACKAGES = {'test_m1_m3_m7_the_original_is_only_read', 'test_board_migrates_and_restarts_where_the_project_is'}

BWRAP = shutil.which('bwrap') is not None and subprocess.run(['bwrap', '--dev-bind', '/', '/', 'true']).returncode == 0

def snapshot(root):
    """Every entry below root: kind and mode, and a file's bytes hash and modified time."""
    out = {}
    for dirpath, dirs, files in os.walk(root, followlinks=False):
        for name in dirs + files:
            p = Path(dirpath, name)
            rel = str(p.relative_to(root))
            m = os.lstat(p)
            if stat.S_ISLNK(m.st_mode): out[rel] = 'link ' + os.readlink(p)
            elif stat.S_ISDIR(m.st_mode): out[rel] = f'folder {m.st_mode:o}'
            elif stat.S_ISREG(m.st_mode): out[rel] = f'{hashlib.sha256(p.read_bytes()).hexdigest()} {m.st_mode:o} {m.st_mtime_ns}'
            else: out[rel] = f'other {m.st_mode:o}'
    return out

class Migrate(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='solace-migrate-')
        self.parent = Path(self.tmp.name)
        self.repo = self.parent/'repo'
        self.copy = self.parent/'solace-migration'
        app = self.repo/'app'
        (app/'src').mkdir(parents=True)
        # Extra blank lines and no final newline: Solace lays it out differently.
        (app/'src/app.gleam').write_text('pub fn double(x: Int) -> Int {\n  x * 2\n}\n\n\n\npub fn half(x: Int) -> Int { x / 2 }')
        if self._testMethodName in WITH_PACKAGES:
            # Packages and tests cost a stdlib compile in each of the migration's builds.
            (app/'gleam.toml').write_text(APP_TOML)
            (app/'test').mkdir()
            (app/'test/app_test.gleam').write_text(
                'import app\nimport gleeunit\n\npub fn main() { gleeunit.main() }\n\n'
                'pub fn double_test() {\n  assert app.double(2) == 4\n}\n\npub fn broken_test() {\n  assert app.double(2) == 5\n}\n')
        else:
            # JavaScript builds skip the Erlang compiler, which takes most of a build's time.
            (app/'gleam.toml').write_text('name = "app"\nversion = "1.0.0"\ntarget = "javascript"\n')
        lib = self.repo/'lib'
        (lib/'src').mkdir(parents=True)
        (lib/'gleam.toml').write_text('name = "lib"\nversion = "1.0.0"\ntarget = "javascript"\n')
        (lib/'src/lib.gleam').write_text('pub fn one() { 1 }\n')
        (self.repo/'.git').mkdir()
        (self.repo/'.git/HEAD').write_text('ref: refs/heads/main\n')
        (self.repo/'docs').mkdir()
        (self.repo/'.gitignore').write_text('data.bin\n')
        (self.repo/'data.bin').write_bytes(b'\x00\xff data')
        (self.repo/'run.sh').write_text(f'#!/bin/sh\ncd {self.repo}/app && gleam run\n')
        os.symlink('app/src', self.repo/'linked')
        self.servers = []
    def tearDown(self):
        for s in self.servers:
            s.terminate(); s.wait(timeout=5)
            s.stdout.close(); s.stderr.close()
        self.tmp.cleanup()
    def cli(self, *args, cwd=None, input=None, ok=True):
        p = subprocess.run([BIN, *args], cwd=cwd or self.repo, input=input, text=True, capture_output=True)
        if ok is not None: self.assertEqual(p.returncode == 0, ok, p.stdout + p.stderr)
        return p.stdout + p.stderr
    def migrate(self, *args):
        return self.cli('migrate', '--yes', '--confirm-paths', '--smoke', 'test -f data.bin', *args)

    def test_m1_m3_m7_the_original_is_only_read(self):
        report = self.cli('migrate', '--report')
        self.assertIn('Not migrated yet. Projects you can migrate:', report)
        self.assertIn('  app (app)', report)
        before = snapshot(self.repo)
        out = self.migrate()
        self.assertEqual(snapshot(self.repo), before)
        # The copy holds every file of the original.
        copied = snapshot(self.copy)
        for rel, v in before.items():
            self.assertIn(rel, copied)
            if v.startswith('link') or v == 'folder' or rel.startswith(('app/src', 'app/test')): continue
            if rel == '.gitignore':
                ignore = (self.copy/rel).read_text()
                self.assertIn('data.bin\n', ignore)
                self.assertIn('**/.solace/*', ignore)
                continue
            self.assertEqual(copied[rel].split()[0], v.split()[0], rel)
        self.assertIn('! run.sh contains your original folder\'s path', out)
        self.assertIn('Your original folder was read-only to every command Solace ran' if BWRAP else 'Solace couldn\'t make your original folder read-only', out)
        self.assertIn('✓ Your original folder\n  Nothing in it has changed since the copy was made', out)
        # M3: a test that failed before isn't blamed; a layout change is shown and explained.
        self.assertIn('! Test app_test.broken_test fails, as it did before the migration', out)
        self.assertIn('✓ app/src/app.gleam differs only in spacing: the newline at the end of the file, blank lines between definitions', out)
        self.assertIn('✓ Your check command `test -f data.bin` works, as before', out)
        # M7: the code builds without Solace.
        recovery = out[out.index('Your code without Solace'):]
        self.assertIn('✓ The build of app works, as before', recovery)
        self.assertIn('✓ The build of lib works, as before', recovery)
        self.assertIn('Ready to switch', out)
        status = json.loads((self.copy/'.solace/migration.json').read_text())
        diff = next(i for c in status['report']['checks'] if c['name'] == 'Your code, written back by Solace' for i in c['items'] if 'app.gleam' in i['text'])
        self.assertIn('-\n', diff['detail'])
        # M8: one migration at a time.
        self.assertIn('there\'s already a copy', self.cli('migrate', '--yes', ok=False))

    def test_m2_a_path_dependency_outside_blocks_adopt(self):
        (self.parent/'shared').mkdir()
        (self.repo/'lib/gleam.toml').write_text('name = "lib"\nversion = "1.0.0"\n[dependencies]\nshared = { path = "../../shared" }\n')
        out = self.migrate('--project', 'lib')
        self.assertIn('✗ lib/gleam.toml uses shared from ../../shared, outside the copy', out)
        self.assertIn('Anything that could reach your original folder: Something in the copy reaches outside it', self.cli('migrate', '--adopt', '--yes', ok=False))
        self.assertFalse((self.parent/'repo.pre-solace').exists())

    def test_m4_m6_adopt_undo_and_abandon(self):
        before = snapshot(self.repo)
        self.migrate()
        out = self.cli('migrate', '--adopt', '--yes')
        self.assertIn('now uses Solace', out)
        self.assertIn(f'run `cd {self.repo}`', out)
        self.assertEqual(snapshot(self.parent/'repo.pre-solace'), before)
        self.assertFalse(self.copy.exists())
        self.assertTrue((self.repo/'app/.solace/store.db').exists())
        self.assertIn('Your project uses Solace', self.cli('migrate', '--report'))
        self.cli('migrate', '--undo', '--yes')
        self.assertEqual(snapshot(self.repo), before)
        self.assertTrue((self.copy/'app/.solace/store.db').exists())
        self.cli('migrate', '--abandon', '--yes')
        self.assertFalse(self.copy.exists())
        self.assertEqual(snapshot(self.repo), before)

    def test_m5_adopt_refuses_changes_and_swaps_back_on_failure(self):
        self.migrate()
        # A change in the copy puts the checks out of date.
        (self.copy/'docs/note.md').write_text('new\n')
        self.assertIn('changed after the checks ran', self.cli('migrate', '--adopt', '--yes', ok=False))
        self.cli('migrate', '--recheck', cwd=self.copy)
        # An edit to the original since the copy was taken.
        (self.repo/'docs/later.md').write_text('mine\n')
        self.assertIn('docs/later.md was added', self.cli('migrate', '--adopt', '--yes', ok=False))
        self.cli('migrate', '--abandon', '--yes')
        (self.repo/'docs/later.md').unlink()
        # Passes in the baseline and the copy, fails at the original path: both folders go back.
        before = snapshot(self.repo)
        self.cli('migrate', '--yes', '--confirm-paths', '--smoke', 'test "$(basename "$PWD")" != repo')
        out = self.cli('migrate', '--adopt', '--yes', ok=False)
        self.assertIn('both folders were put back', out)
        self.assertEqual(snapshot(self.repo), before)
        self.assertTrue((self.copy/'.solace/migration.json').exists())

    def test_m8_create_only_makes_new_projects(self):
        self.assertIn('migrating', self.cli('create', 'x', cwd=self.repo/'app', ok=False))
        empty = self.parent/'empty'; empty.mkdir()
        self.assertIn('Nothing was created', self.cli('create', 'web', cwd=empty, input='n\n'))
        self.assertFalse((empty/'web').exists())
        self.cli('create', 'web', cwd=empty, input='y\n')
        self.assertTrue((empty/'web/.solace/store.db').exists())
        self.assertIn('already exists', self.cli('create', 'web', cwd=empty, input='y\n', ok=False))

    def test_board_migrates_and_restarts_where_the_project_is(self):
        self.serve(self.repo)
        v = self.get()
        self.assertEqual(v['state'], 'available')
        self.assertEqual(sorted(c['dir'] for c in v['candidates']), ['app', 'lib'])
        self.assertEqual(self.post('/migration/start', {'projects': ['app'], 'smoke': ['true']})['state'], 'running')
        # run.sh names the original: nothing runs until the user has looked.
        v = self.until(lambda v: v['state'] == 'confirm_paths')
        self.assertIn('run.sh contains your original folder\'s path', [i['text'] for i in v['flagged']][0])
        self.post('/migration/confirm', {})
        v = self.until(lambda v: v['state'] == 'in_progress')
        self.assertEqual(v['top'], str(self.copy))
        self.assertEqual(v['original'], str(self.repo))
        self.assertEqual(v['blockers'], [])
        self.assertTrue(v['report']['current'])
        self.assertEqual([c['name'] for c in v['report']['checks']], ['Copying your folder', 'Your code, written back by Solace', 'Builds and tests', 'Anything that could reach your original folder', 'Your original folder', 'Your code without Solace'])
        self.post('/migration/adopt', {})
        v = self.until(lambda v: v['state'] == 'adopted')
        self.assertEqual(v['top'], str(self.repo))
        self.assertTrue(v['undo'])
        self.assertTrue((self.parent/'repo.pre-solace').exists())

    def test_a_served_copy_reads_in_an_edit_made_outside_solace(self):
        self.serve(self.repo)
        self.post('/migration/start', {'projects': ['app'], 'smoke': ['true']})
        self.until(lambda v: v['state'] == 'confirm_paths')
        self.post('/migration/confirm', {})
        self.until(lambda v: v['state'] == 'in_progress')
        source = self.copy/'app/src/app.gleam'
        source.write_text(source.read_text() + '\npub fn added_outside() { 5 }\n')
        self.poll(
            lambda: self.call('GET', '/p/app/tasks'),
            lambda result: result[0] == 200 and any(t['title'] == 'Unexpected Changes' for t in result[1]),
            timeout=15,
        )
        self.post('/migration/abandon', {})
        self.until(lambda v: v['state'] == 'available')

    # Fixes from Astra's review (handoff.md).

    def test_links_into_the_original_point_into_the_copy(self):
        (self.repo/'vendor/shared').mkdir(parents=True)
        (self.repo/'vendor/shared/x.gleam').write_text('pub fn x() { 1 }\n')
        (self.repo/'stash').mkdir()
        (self.repo/'stash/keep.txt').write_text('mine\n')
        os.symlink(self.repo/'vendor/shared', self.repo/'app/src/shared')
        os.symlink(self.repo/'stash', self.repo/'app/build')
        before = snapshot(self.repo)
        out = self.migrate('--project', 'app')
        self.assertEqual(snapshot(self.repo), before)
        self.assertIn('app/src/shared linked into your original folder; in the copy it links to the same place in the copy', out)
        self.assertIn('app/build linked into your original folder', out)
        self.assertEqual(os.readlink(self.copy/'app/build'), '../stash')

    def test_compiled_output_linking_outside_is_rebuilt_not_followed(self):
        (self.parent/'elsewhere').mkdir()
        (self.repo/'app/build/dev/javascript/app').mkdir(parents=True)
        os.symlink(self.parent/'elsewhere', self.repo/'app/build/dev/javascript/app/priv')
        out = self.migrate('--project', 'app')
        self.assertIn('Ready to switch', out)
        self.assertNotIn('build/dev', out)

    def test_a_link_out_of_a_projects_source_stops_the_migration(self):
        (self.parent/'elsewhere').mkdir()
        (self.parent/'elsewhere/y.gleam').write_text('pub fn y() { 1 }\n')
        os.symlink(self.parent/'elsewhere', self.repo/'app/src/elsewhere')
        out = self.cli('migrate', '--yes', '--confirm-paths', '--project', 'app', ok=False)
        self.assertIn('app/src/elsewhere', out)
        self.assertIn('is kept so you can see what happened', out)
        self.assertTrue((self.parent/'elsewhere/y.gleam').exists())
        self.assertIn('The migration stopped.', self.cli('migrate', '--report'))
        self.cli('migrate', '--abandon', '--yes')
        self.assertFalse(self.copy.exists())

    def test_commands_cant_change_the_original(self):
        before = snapshot(self.repo)
        smoke = f'touch {self.repo}/pwned'
        out = self.cli('migrate', '--yes', '--smoke', smoke)
        self.assertIn('Waiting for you to look at the files below', out)
        self.assertIn(f'Your check command `{smoke}` uses your original folder\'s path', out)
        self.assertFalse((self.copy/'app/.solace/store.db').exists())
        self.assertEqual(snapshot(self.repo), before)
        if not BWRAP:
            self.skipTest('bubblewrap is not installed')
        out = self.cli('migrate', '--confirm-paths', cwd=self.copy)
        self.assertIn(f'Your check command `{smoke}` fails, as it did before the migration', out)
        self.assertEqual(snapshot(self.repo), before)

    def fail_after(self, point, *args, cwd):
        env = dict(os.environ, SOLACE_MIGRATE_FAIL_AFTER=point)
        p = subprocess.run([BIN, 'migrate', *args, '--yes'], cwd=cwd, env=env, text=True, capture_output=True)
        self.assertNotEqual(p.returncode, 0, p.stdout + p.stderr)

    def test_an_interrupted_swap_is_put_back(self):
        before = snapshot(self.repo)
        self.migrate()
        pre = self.parent/'repo.pre-solace'
        for point, cwd in [('adopt:1', self.copy), ('adopt:2', self.repo)]:
            self.fail_after(point, '--adopt', cwd=self.repo if point == 'adopt:1' else self.repo)
            self.assertTrue((self.parent/'.solace-migration-swap.json').exists())
            self.assertIn('was undone', self.cli('migrate', '--report', cwd=cwd))
            self.assertFalse((self.parent/'.solace-migration-swap.json').exists())
            self.assertFalse(pre.exists())
            self.assertEqual(snapshot(self.repo), before)
            self.assertIn('You\'re working in a copy', self.cli('migrate', '--report'))
        self.cli('migrate', '--adopt', '--yes')
        self.fail_after('undo:1', '--undo', cwd=self.repo)
        self.assertIn('was undone', self.cli('migrate', '--report', cwd=pre))
        self.assertIn('Your project uses Solace', self.cli('migrate', '--report'))
        self.assertEqual(snapshot(pre), before)
        self.cli('migrate', '--undo', '--yes')
        self.assertEqual(snapshot(self.repo), before)

    def test_git_refreshing_its_index_doesnt_block_adoption(self):
        shutil.rmtree(self.repo/'.git')
        git = lambda *a: subprocess.run(['git', '-c', 'user.email=t@t', '-c', 'user.name=t', *a], cwd=self.repo, check=True, capture_output=True)
        git('init', '-q'); git('add', '-A'); git('commit', '-qm', 'start')
        self.migrate()
        # Rewrites the index with the same entries, as any git command may when it refreshes cached file times.
        index = hashlib.sha256((self.repo/'.git/index').read_bytes()).hexdigest()
        git('update-index', '--index-version', '4')
        git('status', '--short')
        self.assertNotEqual(hashlib.sha256((self.repo/'.git/index').read_bytes()).hexdigest(), index)
        self.assertIn('now uses Solace', self.cli('migrate', '--adopt', '--yes'))
        self.cli('migrate', '--undo', '--yes')
        self.cli('migrate', '--abandon', '--yes')
        # Staging something is a change: adoption refuses.
        self.migrate()
        git('add', '-f', 'data.bin')
        self.assertIn('.git/index changed', self.cli('migrate', '--adopt', '--yes', ok=False))

    def test_one_operation_at_a_time_and_not_while_served(self):
        self.migrate()
        with open(self.copy/'app/.solace/lock', 'w') as f:
            fcntl.flock(f, fcntl.LOCK_EX)
            self.assertIn('busy', self.cli('migrate', '--recheck', cwd=self.copy, ok=False))
        with open(self.parent/'.solace-migration.lock', 'w') as f:
            fcntl.flock(f, fcntl.LOCK_EX)
            self.assertIn('another migration step is running', self.cli('migrate', '--abandon', '--yes', ok=False))
        server = subprocess.Popen([BIN, 'serve', '--port', '0'], cwd=self.copy, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.servers.append(server)
        self.assertIn('listening', server.stdout.readline())
        out = self.cli('migrate', '--adopt', '--yes', ok=False)
        self.assertIn('use the board', out)
        self.assertTrue(self.copy.exists())

    def test_errors_in_the_original_carry_over(self):
        (self.repo/'lib/src/lib.gleam').write_text('pub fn one() { 1 + "a" }\n')
        out = self.migrate('--project', 'lib')
        self.assertIn('lib doesn\'t build, before or after the migration. The copy has the same errors as your original.', out)
        self.assertIn('Ready to switch', out)
        out = self.cli('migrate', '--adopt', '--yes')
        self.assertIn('the copy has the same ones', out)
        self.assertIn('Builds and tests: lib doesn\'t build, before or after the migration', out)
        self.assertIn('now uses Solace', out)

    def test_a_change_during_the_checks_leaves_them_out_of_date(self):
        out = self.cli('migrate', '--yes', '--confirm-paths', '--smoke', 'echo x >> docs/edited.md')
        self.assertIn('The copy changed after the checks ran', out)
        self.assertIn('changed after the checks ran', self.cli('migrate', '--adopt', '--yes', ok=False))

    def test_paths_resolve_links_and_git_pointers(self):
        (self.parent/'outside').mkdir()
        (self.parent/'outside/gleam.toml').write_text('name = "outside"\nversion = "1.0.0"\n')
        (self.repo/'vendor').mkdir()
        os.symlink(self.parent/'outside', self.repo/'vendor/shared')
        (self.repo/'lib/gleam.toml').write_text('name = "lib"\nversion = "1.0.0"\n[dependencies]\nshared = { path = "../vendor/shared" }\n')
        (self.repo/'sub').mkdir()
        (self.repo/'sub/.git').write_text(f'gitdir: {self.repo}/.git/modules/sub\n')
        out = self.migrate('--project', 'app')
        self.assertIn('✗ lib/gleam.toml uses shared from ../vendor/shared, outside the copy', out)
        self.assertEqual(out.count('! vendor/shared links to'), 1)
        self.assertIn(f'✗ sub/.git points git at {self.repo}/.git/modules/sub', out)

    def test_work_in_the_copy_doesnt_block_adoption(self):
        self.migrate()
        f = self.copy/'app/src/app.gleam'
        f.write_text(f.read_text().replace('x * 2', 'x + x'))
        out = self.cli('migrate', '--recheck', cwd=self.copy)
        self.assertIn('✓ Your code, written back by Solace', out)
        self.assertIn('Ready to switch', out)

    def test_permissions_and_entries_that_cant_be_copied(self):
        (self.repo/'private').mkdir(mode=0o700)
        (self.repo/'private').chmod(0o700)
        (self.repo/'private/key').write_text('k\n')
        os.mkfifo(self.repo/'pipe')
        before = snapshot(self.repo)
        out = self.migrate()
        self.assertEqual(snapshot(self.repo), before)
        self.assertEqual(stat.S_IMODE(os.stat(self.copy/'private').st_mode), 0o700)
        self.assertIn('1 special file (a socket, FIFO or device) wasn\'t copied', out)
        self.assertFalse((self.copy/'pipe').exists())

    def test_an_unreadable_file_leaves_no_copy(self):
        if os.geteuid() == 0:
            self.skipTest('root reads everything')
        secret = self.repo/'docs/secret.txt'
        secret.write_text('s\n'); secret.chmod(0)
        out = self.cli('migrate', '--yes', '--confirm-paths', ok=False)
        self.assertIn('docs/secret.txt', out)
        self.assertFalse(self.copy.exists())
        secret.chmod(0o644)
        self.migrate()

    def test_source_that_isnt_utf8_is_reported(self):
        (self.repo/'app/src/bad.gleam').write_bytes(b'pub fn bad() { "\xff" }\n')
        out = self.cli('migrate', '--yes', '--confirm-paths', '--project', 'app', ok=False)
        self.assertIn('is kept so you can see what happened', out)
        out = self.cli('migrate', '--report')
        self.assertIn('The migration stopped.', out)
        self.assertIn('✗ app/src/bad.gleam isn\'t UTF-8 text', out)
        self.cli('migrate', '--abandon', '--yes')
        self.assertFalse(self.copy.exists())

    def serve(self, cwd):
        server = subprocess.Popen([BIN, 'serve', '--port', '0'], cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.servers.append(server)
        line = server.stdout.readline()
        self.assertIn('listening', line, line or server.stderr.read())
        self.port = int(line.strip().rsplit(':', 1)[1])

    def call(self, method, url, body=None):
        c = http.client.HTTPConnection('127.0.0.1', self.port, timeout=30)
        c.request(method, url, None if body is None else json.dumps(body), {'Content-Type': 'application/json'})
        r = c.getresponse(); v = json.loads(r.read()); c.close()
        return r.status, v
    def get(self):
        status, v = self.call('GET', '/migration')
        self.assertEqual(status, 200, v)
        return v
    def post(self, url, body):
        status, v = self.call('POST', url, body)
        self.assertEqual(status, 200, v)
        return v

    def poll(self, fetch, predicate, timeout=120):
        last = None
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            try:
                last = fetch()
                if predicate(last): return last
            except (ConnectionError, http.client.HTTPException):
                pass  # the server is restarting
            time.sleep(.2)
        self.fail(f'timed out; last {last}')

    def until(self, predicate, timeout=120):
        def check():
            v = self.get()
            error = v.get('error')
            self.assertIsNone(error, error)
            return v
        return self.poll(check, predicate, timeout)

if __name__ == '__main__':
    unittest.main()
