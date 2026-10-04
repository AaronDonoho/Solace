import http.client
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

BIN = os.environ['SOLACE_TEST_BIN']
FAKE = Path(__file__).with_name('fake_agent.py')
PILOT = Path(__file__).parents[2] / 'bench' / 'pilot-large'


class View(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='solace-view-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'fixture'
        self.root.mkdir()
        (self.root / 'src').mkdir()
        (self.root / 'test').mkdir()
        (self.root / 'src/app.gleam').write_text('''import gleam/io

@external(erlang, "file", "write_file")
fn write(path: String, text: String) -> Nil

@external(erlang, "init", "get_plain_arguments")
fn args() -> List(String)

pub fn main() {
  let _ = args()
  io.println("hello")
  write("out", "hi")
}
''')
        (self.root / 'test/app_test.gleam').write_text('''import app

pub fn main_test() {
  assert app.main() == Nil
}
''')
        shutil.copy(PILOT / 'test/pilot_test.gleam', self.root / 'test/pilot_test.gleam')
        for name in ['gleam.toml', 'manifest.toml']:
            shutil.copy(PILOT / name, self.root / name)
        (self.root / 'gleam.toml').write_text((PILOT / 'gleam.toml').read_text().replace('pilot_large', 'fixture'))
        shutil.copytree(PILOT / 'build/packages', self.root / 'build/packages')
        shutil.copytree(PILOT / 'build/dev', self.root / 'build/dev')
        self.env = {**os.environ, 'SOLACE_CONFIG': str(self.root / 'system.toml')}
        (self.root / 'system.toml').write_text('task_timeout=5\n[[models]]\nid="fake"\nlabel="Fake"\nagent="codex"\nrun=["python3",' + json.dumps(str(FAKE)) + ']\n')
        self.cli('migrate', '--yes')
        self.cli('migrate', '--adopt', '--yes')
        self.server = subprocess.Popen([BIN, 'serve', '--port', '0'], cwd=self.root, env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        line = self.server.stdout.readline()
        self.assertIn('listening', line)
        self.port = int(line.rsplit(':', 1)[1])

    def tearDown(self):
        self.server.terminate()
        self.server.wait(timeout=3)
        self.server.stdout.close()
        self.server.stderr.close()
        self.tmp.cleanup()

    def cli(self, *args, input_text=None):
        p = subprocess.run([BIN, *args], cwd=self.root, env=self.env, input=input_text, capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        return p.stdout

    def api(self, method, path, body=None):
        c = http.client.HTTPConnection('127.0.0.1', self.port, timeout=8)
        c.request(method, path, None if body is None else json.dumps(body))
        response = c.getresponse()
        content = response.read()
        c.close()
        self.assertEqual(response.status, 200, content)
        return json.loads(content)

    def test_c1_c4_graph_and_files(self):
        graph = self.api('GET', '/graph')
        nodes = {n['id']: n for n in graph['nodes']}
        main = next(n for n in nodes.values() if n['name'] == 'main')
        self.assertEqual(main['kind'], 'entry')
        effects = {e['to'] for e in graph['edges'] if e['from'] == main['id'] and e['kind'] == 'uses'}
        self.assertTrue({'effect:prints', 'effect:writes files', 'effect:environment'} <= effects)
        self.assertTrue(any(e['kind'] == 'tests' and e['to'] == main['id'] for e in graph['edges']))
        files = {n['name']: n['tier'] for n in nodes.values() if n['kind'] == 'file'}
        listed = dict(line.rsplit(' [', 1) for line in self.cli('files').splitlines() if line.endswith(']') and not line.endswith('[internal]') and not line.endswith('[graph]') and not line.startswith('src/') and not line.startswith('test/'))
        listed = {k: v[:-1] for k, v in listed.items()}
        self.assertEqual(files, listed)

    def test_c3_task_review(self):
        self.cli('submit', 'app.main', input_text='pub fn main() { let _ = args()\n write("out", "hi") }')
        steps = [{'command': 'submit app.main', 'text': 'pub fn main() {\n  let _ = args()\n  io.println("changed")\n  write("out", "hi")\n}'}, {'command': 'task done done'}]
        task = self.api('POST', '/tasks', {'title': 'Print', 'body': json.dumps(steps)})
        self.api('POST', '/tasks/' + task['id'] + '/actions', {'action': 'plan'})
        self.api('POST', '/tasks/' + task['id'] + '/actions', {'action': 'assign', 'model': 'fake'})
        for _ in range(200):
            task = self.api('GET', '/tasks/' + task['id'])
            if task['column'] == 'needs_review':
                break
            time.sleep(.05)
        self.assertEqual(task['column'], 'needs_review')
        review = self.api('GET', '/graph?task=' + task['id'])['review']
        self.assertTrue(any(e['id'].startswith('fn:') and e['effects_added'] == ['effect:prints'] for e in review['entries']))
        self.assertIn('1 effects added', review['summary'])
        self.assertIn('untested uses', review['summary'])
        self.assertIn('untested', review)


if __name__ == '__main__':
    unittest.main()
