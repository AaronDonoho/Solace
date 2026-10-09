import base64
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

class Board(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='solace-board-')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)/'fixture'
        self.root.mkdir()
        (self.root/'src').mkdir()
        (self.root/'src/app.gleam').write_text('pub fn one() { 1 }\n\npub fn two() { 2 }\n\npub fn removed() { 3 }\n')
        (self.root/'gleam.toml').write_text('name="fixture"\nversion="1.0.0"\ntarget="javascript"\n')
        (self.root/'solace.toml').write_text('unwatched=["priv/gen/"]\n')
        (self.root/'priv').mkdir()
        (self.root/'priv/a').write_text('original')
        config=self.root/'system.toml'
        config.write_text('task_timeout=2\n[[models]]\nid="fake"\nlabel="Fake agent"\nagent="codex"\nrun=["python3",'+json.dumps(FAKE)+']\n[[models]]\nid="missing"\nlabel="Missing agent"\nagent="codex"\nrun=["solace-no-such-program"]\n')
        self.env={**os.environ,'SOLACE_CONFIG':str(config)}
        # Exercise native CLI launch/resume flags without contacting a provider.
        programs = self.root/'fake-bin'
        programs.mkdir()
        for name in ['codex', 'claude']:
            program = programs/name
            program.write_text('#!/usr/bin/env python3\n' + Path(FAKE).read_text())
            program.chmod(0o755)
        self.env['PATH'] = str(programs) + os.pathsep + os.environ['PATH']
        with config.open('a') as f:
            for name, agent, model in [('native', 'codex', 'fixture'), ('native-other', 'codex', 'other'), ('native-claude', 'claude', 'fixture')]:
                kind = 'openai' if agent == 'codex' else 'anthropic'
                f.write(f'\n[[models]]\nid="{name}"\nlabel="Fixture"\nagent="{agent}"\nmodel="{model}"\nendpoint={{kind="{kind}",base_url="http://localhost:1"}}\n')
        self.cli('migrate', '--yes')
        self.cli('migrate', '--adopt', '--yes')
        self.serve()
        self.clients=[]
    def serve(self):
        self.server=subprocess.Popen([BIN,'serve','--port','0'],cwd=self.root,env=self.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        line=self.server.stdout.readline()
        self.assertIn('listening',line)
        self.port=int(line.strip().rsplit(':',1)[1])
    def tearDown(self):
        # End all fake processes before terminating their supervisor.
        try:
            for t in ([] if self.server.poll() is not None else self.api('GET','/tasks')):
                full=self.api('GET','/tasks/'+t['id'])
                for r in full['runs']:
                    p=self.root/'.solace'/(r['id']+'.pid')
                    if p.exists():
                        try: os.killpg(int(p.read_text()),9)
                        except ProcessLookupError: pass
        finally:
            for c in self.clients: c.close()
            self.server.terminate();self.server.wait(timeout=3)
            self.server.stdout.close();self.server.stderr.close()
            self.tmp.cleanup()
    def cli(self,*args):
        p=subprocess.run([BIN,*args],cwd=self.root,env=self.env,text=True,capture_output=True)
        self.assertEqual(p.returncode,0,p.stdout+p.stderr)
        return p.stdout
    def api(self,method,url,body=None,code=None):
        c=http.client.HTTPConnection('127.0.0.1',self.port,timeout=8)
        c.request(method,url,None if body is None else json.dumps(body),{'Content-Type':'application/json'})
        r=c.getresponse();v=json.loads(r.read());c.close()
        if code:self.assertEqual(v.get('code'),code,v);self.assertEqual(r.status,409)
        else:self.assertEqual(r.status,200,v)
        return v
    def create(self,steps=None):
        return self.api('POST','/tasks',{'title':'Fixture','body':json.dumps(steps or [])})
    def action(self,t,name,code=None,**kw):
        return self.api('POST','/tasks/'+t['id']+'/actions',{'action':name,**kw},code)
    def get(self,t):return self.api('GET','/tasks/'+t['id'])
    def bare(self,method,url,headers,body=None):
        c=http.client.HTTPConnection('127.0.0.1',self.port,timeout=8)
        c.putrequest(method,url,skip_host='Host' in headers)
        for k,v in headers.items():c.putheader(k,v)
        c.putheader('Content-Length',str(len(body or '')));c.endheaders(body.encode() if body else None)
        r=c.getresponse();v=r.read();c.close();return r.status,v
    def test_only_the_servers_own_address_may_ask(self):  # D-80
        own=f'127.0.0.1:{self.port}'
        body=json.dumps({'title':'Cross','body':'[]'})
        status,v=self.bare('POST','/tasks',{'Origin':'http://evil.example','Content-Type':'text/plain'},body)
        self.assertEqual(status,403,v);self.assertEqual(json.loads(v)['code'],'E7016')
        self.assertEqual(self.bare('POST','/tasks',{'Origin':f'http://{own}','Content-Type':'text/plain'},body)[0],200)
        self.assertEqual(self.bare('GET','/tasks',{'Origin':'http://evil.example'})[0],403)
        self.assertEqual(self.bare('GET','/tasks',{'Host':'evil.example'})[0],403)
        self.assertEqual(self.bare('GET','/tasks',{'Host':f'localhost:{self.port}'})[0],200)
        self.assertEqual(self.bare('GET','/tasks',{'Host':'127.0.0.1:1'})[0],403)
        self.assertEqual([t['title'] for t in self.api('GET','/tasks')],['Cross'])
    def test_the_lock_file_holds_the_port_while_served(self):  # D-80
        lock=json.loads((self.root/'.solace/serve.lock').read_text())
        self.assertEqual(lock['port'],self.port);self.assertEqual(lock['pid'],self.server.pid)
    def wait(self,t,predicate,timeout=10):
        end=time.monotonic()+timeout
        while time.monotonic()<end:
            value=self.get(t)
            if predicate(value):return value
            time.sleep(.03)
        self.fail('timed out: '+json.dumps(value))
    def assign(self,steps):
        t=self.create(steps);self.action(t,'plan');return self.action(t,'assign',model='fake')
    def completed(self,steps):
        t=self.assign([*steps,{'command':'task done done'}]);return self.wait(t,lambda t:t['column']=='needs_review')
    def stream(self,seq=0):
        c=http.client.HTTPConnection('127.0.0.1',self.port,timeout=8);c.request('GET','/events',headers={'Last-Event-ID':str(seq)})
        r=c.getresponse();self.clients.append(c);return r
    def next_event(self,r):
        seq=None;ready=False
        while True:
            line=r.readline().decode().strip()
            if line.startswith('id:'):seq=line[3:].strip()
            if line=='event: ready':ready=True
            elif line.startswith('data:') and ready:ready=False
            elif line.startswith('data:'):return seq,json.loads(line[5:])

    def test_author_conversations_reviews_handoffs_and_cleanup(self):
        for model in ['native', 'native-claude']:
            t = self.create([{'command':'submit app.one','text':'pub fn one() { 99 }'}, {'command':'check'}, {'command':'task done investigation plan'}])
            self.action(t, 'plan')
            self.action(t, 'investigate', model=model)
            t = self.wait(t, lambda x: x.get('conversation', {}).get('session') and x['runs'][-1]['state']=='ended')
            original = t['conversation'].copy()
            first = t['runs'][-1]['id']
            self.assertTrue((self.root/'.solace'/(first+'.exited')).exists())
            self.assertIn('E7010', (self.root/'.solace'/(first+'.responses')).read_text())
            self.api('PATCH', '/tasks/'+t['id'], {'body':json.dumps([{'command':'submit app.one','text':'pub fn one() { 10 }'}, {'command':'check'}, {'command':'task done work complete'}])})
            t = self.action(t, 'assign', model=model)
            work = t['runs'][-1]['id']
            t = self.wait(t, lambda x: x['column']=='needs_review' and x.get('conversation', {}).get('notes_seen'))
            launch = json.loads((self.root/'.solace'/(work+'.launch')).read_text())
            self.assertEqual(launch['session'], original['session'])
            self.assertEqual(launch['home'], str(self.root/'.solace/board'/t['id']/original['home']))
            prompt = (self.root/'.solace'/(work+'.prompt')).read_text()
            self.assertIn("Continue the author's conversation", prompt)
            self.assertNotIn('Start with `session start', prompt)
            self.assertIn('10', self.cli('view', 'app.one'))

            self.api('PATCH', '/tasks/'+t['id'], {'body':json.dumps([{'command':'submit app.one','text':'pub fn one() { 99 }'}, {'command':'task note please fix rounding'}])})
            t = self.action(t, 'review', model=model)
            reviewer = t['runs'][-1]['id']
            review_home = self.root/'.solace/board'/t['id']/reviewer
            t = self.wait(t, lambda x: (self.root/'.solace'/(reviewer+'.exited')).exists() and not review_home.exists())
            launch = json.loads((self.root/'.solace'/(reviewer+'.launch')).read_text())
            self.assertEqual(launch['session'], reviewer)
            self.assertIn('E7010', (self.root/'.solace'/(reviewer+'.responses')).read_text())
            self.assertEqual(t['conversation']['session'], original['session'])

            # Removing earlier notes must not hide subsequently added feedback.
            self.api('DELETE', '/tasks/'+t['id']+'/notes/0')

            steps = [{'command':'check'}, {'command':'submit app.one','text':'pub fn one() { 20 }'}, {'command':'task done fixed rounding'}]
            self.api('PATCH', '/tasks/'+t['id'], {'body':json.dumps(steps)})
            t = self.action(t, 'reassign', model=model, text='Please address review')
            repair = t['runs'][-1]['id']
            t = self.wait(t, lambda x: x['column']=='needs_review' and x.get('conversation', {}).get('notes_seen'))
            self.assertEqual(t['conversation']['session'], original['session'])
            prompt = (self.root/'.solace'/(repair+'.prompt')).read_text()
            self.assertIn('please fix rounding', prompt)
            self.assertIn('Please address review', prompt)
            self.assertNotIn('- agent: investigation plan', prompt)

            self.api('PATCH', '/tasks/'+t['id'], {'body':json.dumps([
                {'command':'task diff --since-review'}, {'command':'task note **Assessment: Looks good** repaired'}])})
            t = self.action(t, 'review', model=model)
            followup = t['runs'][-1]['id']
            t = self.wait(t, lambda x:(self.root/'.solace'/(followup+'.exited')).exists())
            launch = json.loads((self.root/'.solace'/(followup+'.launch')).read_text())
            self.assertEqual(launch['session'], followup)
            self.assertEqual(t['conversation']['session'], original['session'])
            prompt = (self.root/'.solace'/(followup+'.prompt')).read_text()
            self.assertIn('Start with `task diff --since-review`', prompt)
            self.assertIn('please fix rounding', prompt)
            self.assertIn('fixed rounding', prompt)
            delta = json.loads((self.root/'.solace'/(followup+'.responses')).read_text().splitlines()[0])['result']['content'][0]['text']
            self.assertIn(reviewer, delta)
            self.assertIn('-   10', delta)
            self.assertIn('+   20', delta)

            # A lost provider session transparently falls back once to a fresh handoff.
            home = self.root/'.solace/board'/t['id']/t['conversation']['home']
            (home/('session-'+original['session'])).unlink()
            self.api('PATCH', '/tasks/'+t['id'], {'body':json.dumps([{'command':'task done recovered'}])})
            t = self.action(t, 'reassign', model=model)
            fallback = t['runs'][-1]['id']
            t = self.wait(t, lambda x: x['column']=='needs_review' and x.get('conversation', {}).get('notes_seen'))
            self.assertNotEqual(t['conversation']['session'], original['session'])
            self.assertFalse(home.exists())
            prompt = (self.root/'.solace'/(fallback+'.prompt')).read_text()
            self.assertIn('Fresh conversation', prompt)
            self.assertIn('please fix rounding', prompt)
            self.assertIn('app.one', prompt)
            self.assertIn('stale: project changed', prompt)
            home = self.root/'.solace/board'/t['id']/t['conversation']['home']
            self.action(t, 'approve')
            self.action(t, 'complete')
            self.wait(t, lambda x: x.get('conversation') is None and not home.exists())

    def review(self, t, steps=None):
        if steps is not None:
            self.api('PATCH', '/tasks/'+t['id'], {'body':json.dumps(steps)})
        t = self.action(t, 'review', model='fake')
        rid = t['runs'][-1]['id']
        t = self.wait(t, lambda x:x['runs'][-1]['state']=='ended')
        prompt = (self.root/'.solace'/(rid+'.prompt')).read_text()
        responses = self.root/'.solace'/(rid+'.responses')
        replies = [json.loads(l)['result']['content'][0]['text'] for l in responses.read_text().splitlines()] if responses.exists() else []
        return t, prompt, replies

    def test_review_checkpoints_unchanged_failure_deleted_feedback_and_legacy(self):
        t = self.completed([{'command':'submit app.one','text':'pub fn one() { 10 }'}])
        steps = [{'command':'task diff --since-review'}, {'command':'task note **Assessment: Needs work** unique finding'}]
        t, prompt, replies = self.review(t, steps)
        self.assertIn('assess the whole task: no completed review', prompt)
        self.assertIn('falling back', replies[0])
        first = t['runs'][-1]['id']
        t, prompt, replies = self.review(t)
        self.assertIn('Code and relevant context are unchanged', prompt)
        self.assertIn('must not turn Needs work into Looks good', prompt)
        self.assertIn(first, replies[0])
        self.assertIn('No task entity changes', replies[0])
        baseline = t['runs'][-1]['id']
        t, _, _ = self.review(t, [{'exit':7}])
        t, prompt, replies = self.review(t, steps)
        self.assertIn(baseline, replies[0])
        self.assertIn('Code and relevant context are unchanged', prompt)
        # Delete feedback by its stored run identity; logs must not bring it back.
        for i in reversed(range(len(t['notes']))):
            if t['notes'][i]['author'] == 'agent' and 'unique finding' in t['notes'][i]['text']:
                self.api('DELETE', '/tasks/'+t['id']+'/notes/'+str(i))
        t, prompt, replies = self.review(t, [{'command':'task diff --since-review'}, {'command':'task note fine'}])
        self.assertIn('Previous feedback was deleted', prompt)
        self.assertNotIn('unique finding', prompt)
        checkpoint = self.root/'.solace/board'/t['id']/'reviews'/(t['runs'][-1]['id']+'.json')
        checkpoint.unlink()
        t, prompt, replies = self.review(t)
        self.assertIn('missing or legacy review checkpoint', prompt)
        self.assertIn('Full task diff', replies[0])
        self.action(t, 'approve'); self.action(t, 'complete')
        self.assertFalse(checkpoint.parent.exists())

    def test_review_delta_add_remove_revert_and_context(self):
        t = self.completed([
            {'command':'read priv/a'}, {'command':'write priv/a','text':'reviewed'},
            {'command':'write priv/new','text':'new'},
            {'command':'submit app.one','text':'pub fn one() { 10 }'},
        ])
        t, _, _ = self.review(t, [{'command':'task note **Assessment: Needs work** repair'}])
        self.api('PATCH', '/tasks/'+t['id'], {'body':json.dumps([
            {'command':'read priv/a'}, {'command':'write priv/a','text':'original'},
            {'command':'read priv/new'}, {'command':'remove priv/new'},
            {'command':'write priv/later','text':'added later'},
            {'command':'submit app.two','text':'pub fn two() { 20 }'},
            {'command':'delete app.removed'},
            {'command':'submit --new app.added','text':'pub fn added() { 30 }'},
            {'command':'task done repaired files'},
        ])})
        t = self.action(t, 'reassign', text='Please fix the findings')
        t = self.wait(t, lambda x:x['column']=='needs_review')
        steps = [{'command':'task diff --since-review'}, {'command':'task diff --since-review app.two'},
                 {'command':'task diff'}, {'command':'task note **Assessment: Looks good**'}]
        t, prompt, replies = self.review(t, steps)
        delta, filtered, whole = replies[:3]
        self.assertIn('Subsequent work', prompt)
        self.assertIn('repaired files', prompt)
        self.assertIn('Human feedback: Please fix the findings', prompt)
        self.assertIn('- reviewed', delta); self.assertIn('+ original', delta)
        self.assertIn('file:priv/new (deleted)', delta)
        self.assertIn('file:priv/later (added)', delta)
        self.assertIn('app.two (changed)', delta)
        self.assertIn('app.removed (deleted)', delta)
        self.assertIn('app.added (added)', delta)
        self.assertNotIn('app.one', delta)
        self.assertIn('app.two', filtered); self.assertNotIn('priv/', filtered)
        self.assertIn('app.one (changed)', whole)
        self.assertIn('back to its state before the task', whole)
        # Same task entities, different surrounding dependency and instructions.
        (self.root/'manifest.toml').write_text((self.root/'manifest.toml').read_text()+'\n# dependency lock changed\n')
        t, prompt, replies = self.review(t)
        self.assertIn('No task entity changes', replies[0])
        self.assertIn('Code or relevant context changed', prompt)
        self.api('PATCH', '/tasks/'+t['id'], {'title':'Reconsider requirements'})
        t, prompt, replies = self.review(t)
        self.assertIn('No task entity changes', replies[0])
        self.assertIn('Code or relevant context changed', prompt)
        self.action(t, 'start_over')
        self.assertFalse((self.root/'.solace/board'/t['id']/'reviews').exists())

    def test_review_detects_changes_while_running(self):
        t = self.completed([])
        self.api('PATCH', '/tasks/'+t['id'], {'body':json.dumps([{'sleep':.6}, {'command':'task note fine'}])})
        t = self.action(t, 'review', model='fake')
        rid = t['runs'][-1]['id']
        self.wait(t, lambda x:(self.root/'.solace'/(rid+'.prompt')).exists())
        (self.root/'manifest.toml').write_text((self.root/'manifest.toml').read_text()+'\n# changed during review\n')
        t = self.wait(t, lambda x:x['runs'][-1]['state']=='ended')
        checkpoint = json.loads((self.root/'.solace/board'/t['id']/'reviews'/(rid+'.json')).read_text())
        self.assertTrue(checkpoint['changed_during_review'])
        t, prompt, _ = self.review(t, [{'command':'task note fine'}])
        self.assertIn('changed during the previous review', prompt)

    def raw(self,method,url,data=b''):
        c=http.client.HTTPConnection('127.0.0.1',self.port,timeout=8)
        c.request(method,url,data)
        r=c.getresponse();out=(r.status,r.getheader('Content-Type'),r.read());c.close()
        return out

    def test_a16_effort_finished_and_note_images(self):
        models={m['id']:m for m in self.api('GET','/models')}
        self.assertEqual(models['native-claude']['efforts'],['low','medium','high','xhigh','max'])
        self.assertEqual(models['native']['efforts'],['low','medium','high','xhigh'])
        self.assertEqual(models['fake']['efforts'],[])
        t=self.create([{'command':'task done done'}])
        png=b'\x89PNG\r\n\x1a\n'+bytes(range(40))
        status,_,body=self.raw('POST','/tasks/'+t['id']+'/files',png)
        self.assertEqual(status,200,body)
        url=json.loads(body)['url']
        self.assertEqual(self.raw('GET',url),(200,'image/png',png))
        for bad in [b'<svg onload="alert(1)"/>', b'\x89PNG\r\n\x1a\n'+bytes(5*1024*1024)]:
            status,_,body=self.raw('POST','/tasks/'+t['id']+'/files',bad)
            self.assertEqual((status,json.loads(body)['code']),(409,'E7001'))
        self.assertEqual(self.raw('GET','/tasks/'+t['id']+'/files/../../solace.toml')[0],404)
        self.api('POST','/tasks/'+t['id']+'/notes',{'text':'Look: ![shot]('+url+')'})
        self.action(t,'plan')
        self.action(t,'assign',code='E7001',model='native-claude',effort='turbo')
        t=self.action(t,'assign',model='native-claude',effort='high')
        self.assertEqual((t['runs'][-1]['effort'],t['effort']),('high','high'))
        rid=t['runs'][-1]['id']
        t=self.wait(t,lambda x:x['column']=='needs_review' and x.get('finished'))
        self.assertEqual(t['finished']['outcome'],'finished')
        argv=json.loads((self.root/'.solace'/(rid+'.launch')).read_text())['argv']
        self.assertEqual(argv[argv.index('--effort')+1],'high')
        self.assertIn('stream-json',argv[argv.index('--input-format')+1])
        message=json.loads((self.root/'.solace'/(rid+'.prompt')).read_text())['message']['content']
        self.assertIn('Look: [image 1]',message[0]['text'])
        self.assertEqual((message[1]['source']['media_type'],message[1]['source']['data']),('image/png',base64.b64encode(png).decode()))
        # Changing model and effort starts a fresh Codex conversation with the image attached.
        t=self.wait(t,lambda x:(self.root/'.solace'/(rid+'.exited')).exists())
        t=self.action(t,'reassign',model='native',effort='low')
        self.assertIsNone(t['finished'])
        rid=t['runs'][-1]['id']
        self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
        argv=json.loads((self.root/'.solace'/(rid+'.launch')).read_text())['argv']
        self.assertTrue(argv[argv.index('--image')+1].endswith(url.rsplit('/',1)[1]))
        self.assertIn('model_reasoning_effort="low"',argv)
        self.assertLess(argv.index('--image'),argv.index('--json'))
        # Custom runners get the image's path in the text.
        t=self.create([{'command':'task done done'}])
        url=json.loads(self.raw('POST','/tasks/'+t['id']+'/files',png)[2])['url']
        self.api('POST','/tasks/'+t['id']+'/notes',{'text':'![a]('+url+')'})
        self.action(t,'plan')
        t=self.wait(self.action(t,'assign',model='fake'),lambda x:x['column']=='needs_review')
        self.assertIn('[image 1: ',(self.root/'.solace'/(t['runs'][-1]['id']+'.prompt')).read_text())

    def test_model_and_effort_change_keep_session_and_start_over_gets_fresh(self):
        t = self.create([{'command':'task done done'}])
        self.action(t, 'plan'); self.action(t, 'assign', model='native')
        t = self.wait(t, lambda x: x.get('conversation', {}).get('notes_seen'))
        original = t['conversation']['session']
        n = len(t['runs'])
        t = self.action(t, 'reassign', model='native-other')
        t = self.wait(t, lambda x: len(x['runs']) > n and x['column']=='needs_review' and x.get('conversation', {}).get('session'))
        self.assertEqual(t['conversation']['session'], original)
        n = len(t['runs'])
        t = self.action(t, 'reassign', model='native', effort='low')
        t = self.wait(t, lambda x: len(x['runs']) > n and x['column']=='needs_review' and x.get('conversation', {}).get('session'))
        self.assertEqual(t['conversation']['session'], original)
        t = self.action(t, 'start_over')
        t = self.wait(t, lambda x: x.get('conversation', {}).get('notes_seen') and x['conversation']['session'] != original)
        home = self.root/'.solace/board'/t['id']/t['conversation']['home']
        self.action(t, 'cancel')
        self.wait(t, lambda x: x.get('conversation') is None and not home.exists())

    def test_resume_does_not_retry_provider_failure(self):
        t = self.create([{'command':'task done done'}])
        self.action(t, 'plan'); self.action(t, 'assign', model='native')
        t = self.wait(t, lambda x: x.get('conversation', {}).get('notes_seen'))
        original = t['conversation']['session']
        self.api('PATCH', '/tasks/'+t['id'], {'body':json.dumps([{'exit':1, 'error':'usage limit reached'}])})
        t = self.action(t, 'reassign')
        rid = t['runs'][-1]['id']
        t = self.wait(t, lambda x: (x['attention'] or {}).get('hold'))
        self.assertEqual(t['column'], 'assigned')
        launch = json.loads((self.root/'.solace'/(rid+'.launch')).read_text())
        self.assertEqual(launch['session'], original)
        self.assertIn('usage limit reached', t['attention']['reason'])
        self.assertFalse((self.root/'.solace/board'/t['id']/(rid+'-fallback')).exists())

    def test_done_allows_graceful_exit(self):
        # The 30-second bound on hung shutdown is board::tests::hung_shutdown_is_killed_after_the_grace_period.
        t = self.completed([])
        rid = t['runs'][-1]['id']
        self.wait(t, lambda x: (self.root/'.solace'/(rid+'.exited')).exists())
        t = self.assign([{'command':'task done done'}, {'sleep':2}])
        t = self.wait(t, lambda x: x['column']=='needs_review')
        rid = t['runs'][-1]['id']
        home = self.root/'.solace/board'/t['id']/rid
        self.assertTrue(home.exists())
        self.wait(t, lambda x: not home.exists())
        self.assertTrue((self.root/'.solace'/(rid+'.exited')).exists())
    def test_a1_default_action_matrix(self):
        w=self.api('GET','/workflow')
        # Seed column positions through the human-owned JSON fixture, then use only HTTP actions.
        for col in w['columns']:
            for name in [*w['actions'],'pause','resume']:
                t=self.create([{'sleep':.2},{'command':'task done done'}])
                t['column']=col['id'];t['home']='.'
                (self.root/'.solace/tasks'/f'{t["id"]}.json').write_text(json.dumps(t))
                allowed=[] if col['final'] else [*col['actions'],'pause','resume','discuss','stop','end_discussion']
                if name not in allowed:
                    e=self.action(t,name,'E7001',model='fake');self.assertEqual(e['allowed'],allowed)
                elif name=='complete' and col.get('requires'):self.action(t,name,'E7006')
                else:
                    result=self.action(t,name,model='fake')
                    if result['runs'] and result['runs'][-1]['state']!='ended':
                        # Clean up the run independently so the next pair doesn't hit E7005.
                        self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
    def test_a2_events_a4_gate(self):
        t=self.completed([])
        self.action(t,'complete','E7006')
        t=self.action(t,'approve');self.assertEqual(t['stamps']['approved']['who'],'human');self.assertIsInstance(t['stamps']['approved']['time'],int)
        self.assertEqual(self.action(t,'complete')['column'],'complete')
        self.api('PATCH','/tasks/'+t['id'],{'title':'no'},'E7004')
        t=self.completed([]);self.action(t,'approve');t=self.action(t,'start_over');self.assertEqual(t['stamps'],{})
        self.wait(t,lambda x:x['column']=='needs_review');self.action(t,'approve');self.assertEqual(self.action(t,'cancel')['column'],'cancelled')
        crashed=self.assign([{'exit':7}]);crashed=self.wait(crashed,lambda x:(x['attention'] or {}).get('hold'));self.assertEqual(crashed['column'],'assigned');self.assertIn('without finishing',crashed['notes'][-1]['text'])
        self.assertIn('without finishing',crashed['attention']['reason'])
        self.assertIsNone(self.action(crashed,'cancel')['attention'])
        # A failed review is flagged but leaves the task where it was.
        reviewed=self.completed([]);self.api('PATCH','/tasks/'+reviewed['id'],{'body':json.dumps([{'exit':7}])})
        self.action(reviewed,'review',model='fake');reviewed=self.wait(reviewed,lambda x:x['runs'][-1]['state']=='ended')
        self.assertEqual(reviewed['column'],'needs_review');self.assertIn('without finishing',reviewed['attention']['reason'])
        timed=self.assign([{'sleep':3}]);timed=self.wait(timed,lambda x:(x['attention'] or {}).get('hold'));self.assertEqual(timed['column'],'assigned');self.assertIn('timeout',timed['notes'][-1]['text'])
    def test_a18_review_reads_task_diff(self):
        (self.root/'priv/b').write_text('one\ntwo\nthree\nfour\nfive\nsix\n')
        t=self.completed([
            {'command':'submit app.one','text':'pub fn one() { 10 }'},
            {'command':'delete app.removed'},
            {'command':'read priv/a'},{'command':'write priv/a','text':'changed'},
            {'command':'read priv/a'},{'command':'write priv/a','text':'original'},
            {'command':'read priv/b'},{'command':'write priv/b','text':'one\ntwo\nthree\nfour\nfive\nSIX\n'},
        ])
        self.api('PATCH','/tasks/'+t['id'],{'body':json.dumps([
            {'command':'task diff'},{'command':'task diff app.one'},{'command':'probe','text':'echo app.one() * 3'},{'command':'task note fine'}])})
        self.action(t,'review',model='fake');t=self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
        rid=t['runs'][-1]['id']
        self.assertIn('task diff',(self.root/'.solace'/(rid+'.prompt')).read_text())
        whole,one,probe=[json.loads(l)['result']['content'][0]['text'] for l in (self.root/'.solace'/(rid+'.responses')).read_text().splitlines()[:3]]
        # A read-only review can probe; nothing it runs is saved or counted as a change.
        self.assertIn('30',probe);self.assertFalse((self.root/'test/solace_probe.gleam').exists())
        self.assertFalse([c for c in t['changes'] if 'probe' in c['entity']])
        self.assertIn('## app.one (changed)\n- pub fn one() { 1 }\n+ pub fn one() {\n+   10\n+ }\n',whole)
        self.assertIn('## app.removed (deleted)\n- pub fn removed() { 3 }\n',whole)
        self.assertIn('## file:priv/a (back to its state before the task)\n',whole)
        self.assertIn('## file:priv/b (changed)\n  …\n  four\n  five\n- six\n+ SIX\n',whole)
        self.assertTrue(one.startswith('## app.one'),one);self.assertNotIn('app.removed',one)
    def test_reassign_preserves_work_and_review_feedback(self):
        original=self.cli('view','app.one')
        t=self.completed([
            {'command':'submit app.one','text':'pub fn one() { 10 }'},
            {'command':'read priv/a'}, {'command':'write priv/a','text':'keep this'},
        ])
        changes=t['changes']
        self.api('PATCH','/tasks/'+t['id'],{'body':json.dumps([{'command':'task note','text':'Review: handle the edge case.'}])})
        self.action(t,'review',model='fake');t=self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
        review=t['runs'][-1]['id']
        self.api('PATCH','/tasks/'+t['id'],{'body':json.dumps([
            {'command':'view app.one'},
            {'command':'submit app.one','text':'pub fn one() { 20 }'},
            {'command':'task done fixed'},
        ])})
        self.action(t,'approve')
        t=self.action(t,'reassign',text='Please address the review.')
        self.assertEqual(t['column'],'assigned');self.assertEqual(t['stamps'],{})
        # The task's changes are computed from the store (D-77 point 4), so read them from the task's GET.
        self.assertEqual(self.get(t)['changes'],changes);self.assertNotEqual(t['runs'][-1]['id'],review)
        rid=t['runs'][-1]['id'];self.assertEqual(t['runs'][-1]['kind'],'work')
        self.assertEqual(t['runs'][-1]['model'],'fake')
        t=self.wait(t,lambda x:x['column']=='needs_review')
        prompt=(self.root/'.solace'/(rid+'.prompt')).read_text()
        self.assertIn('Review: handle the edge case.',prompt);self.assertIn('Please address the review.',prompt)
        first=json.loads((self.root/'.solace'/(rid+'.responses')).read_text().splitlines()[0])
        self.assertIn('10',first['result']['content'][0]['text'])
        self.assertIn('20',self.cli('view','app.one'))
        self.assertEqual((self.root/'priv/a').read_text(),'keep this')
        self.action(t,'complete','E7006')
        self.action(t,'cancel')
        self.assertEqual(self.cli('view','app.one'),original)
        self.assertEqual((self.root/'priv/a').read_text(),'original')

    def test_a3_custom_workflow_and_invalid(self):
        original=(self.root/'solace.toml').read_text()
        default=Path(__file__).parents[1].joinpath('src/default-board.toml').read_text()
        custom=default.replace('actions = ["plan", "cancel"]','actions = ["triage", "plan", "cancel"]')+'\ntriage = {to="triage"}\n\n[[board.columns]]\nid="triage"\nlabel="Triage"\nactions=["plan"]\n'
        (self.root/'solace.toml').write_text(original+custom)
        t=self.create();self.assertEqual(self.action(t,'triage')['column'],'triage');self.assertEqual(self.action(t,'plan')['column'],'planning')
        for broken in [default.replace('to = "planning"','to = "missing"'),default+'\nunused={to="backlog"}\n',default.replace('finished = "needs_review"',''),default.replace('events = {','events = { asked = "missing",'),default.replace('next = "complete"','next = "missing"'),default.replace('id = "complete"','id = "complete"\nactions=["plan"]')]:
            (self.root/'solace.toml').write_text(original+broken);self.api('GET','/workflow',code='E6020')
        (self.root/'solace.toml').write_text(original)
    def test_stop_ends_running_and_paused_agents(self):
        t=self.assign([{'command':'submit app.one','text':'pub fn one() { 7 }'},{'sleep':30}])
        t=self.wait(t,lambda x:x['changes'])
        rid=t['runs'][-1]['id'];pid=int((self.root/'.solace'/(rid+'.pid')).read_text())
        t=self.action(t,'stop')
        self.assertEqual(t['runs'][-1]['outcome'],'stopped');self.assertEqual(t['column'],'assigned')
        self.wait(t,lambda x:(self.root/'.solace'/(rid+'.exited')).exists() or not os.path.exists(f'/proc/{pid}'))
        self.assertIn('7',self.cli('view','app.one'))  # Changes are kept.
        self.action(t,'stop','E7003')
        # A paused agent is held inside its next Solace call; stopping ends that call too.
        t=self.create([{'sleep':.3},{'command':'check'},{'command':'task done done'}]);self.action(t,'plan');t=self.action(t,'investigate',model='fake')
        self.action(t,'pause');time.sleep(.6)
        t=self.action(t,'stop');self.assertIsNone(t['paused_from'])
        self.wait(t,lambda x:x['runs'][-1]['outcome']=='stopped' and x['finished'] is None)
    def test_discussion_turns_and_transcript(self):
        t=self.create([{'command':'task note first reply'},{'command':'task note second reply'}]);self.action(t,'plan')
        self.action(t,'discuss','E7001',model='fake')
        t=self.action(t,'discuss',model='fake',text='What about X?')
        rid=t['runs'][-1]['id'];self.assertEqual(t['runs'][-1]['kind'],'discuss')
        self.action(t,'investigate','E7003',model='fake')
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting')
        self.api('POST','/tasks/'+t['id']+'/notes',{'text':'And Y?'})
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting' and len(x['notes'])==4)
        t=self.action(t,'end_discussion')
        self.assertEqual(t['runs'][-1]['outcome'],'discussed')
        self.assertEqual(len(t['notes']),1);n=t['notes'][0]
        self.assertEqual((n['author'],n['run']),('solace',rid))
        self.assertEqual(n['text'],'### Discussion\n\n**Human:**\n\nWhat about X?\n\n**Agent (fake):**\n\nfirst reply\n\n**Human:**\n\nAnd Y?\n\n**Agent (fake):**\n\nsecond reply')
        self.assertEqual(t['column'],'planning')
        self.action(t,'end_discussion','E7003')
    def test_discussion_plain_reply_is_a_turn(self):
        # An agent that replies in plain text and exits cleanly has taken its turn.
        t=self.create([{'say':'plain reply'}]);self.action(t,'plan')
        t=self.action(t,'discuss',model='fake',text='What about X?');rid=t['runs'][-1]['id']
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting' and len(x['notes'])==2)
        self.assertEqual((t['notes'][1]['author'],t['notes'][1]['text']),('agent','plain reply'))
        self.assertIsNone(t['attention'])
        self.api('POST','/tasks/'+t['id']+'/notes',{'text':'And Y?'})
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting' and len(x['notes'])==4)
        t=self.action(t,'end_discussion')
        self.assertEqual(t['notes'][0]['text'],'### Discussion\n\n**Human:**\n\nWhat about X?\n\n**Agent (fake):**\n\nplain reply\n\n**Human:**\n\nAnd Y?\n\n**Agent (fake):**\n\nplain reply')
        # Exiting with an error still fails the run.
        t=self.create([{'say':'half a reply'},{'exit':1,'error':'boom'}]);self.action(t,'plan')
        t=self.action(t,'discuss',model='fake',text='What about Z?')
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
        self.assertIn('agent exited without finishing',t['notes'][-1]['text']);self.assertIsNotNone(t['attention'])
    def test_a5_question_wait_and_new_agent(self):
        t=self.assign([{'command':'task ask','text':'Which?','options':['a','b']},{'command':'task done answered'}])
        t=self.wait(t,lambda x:(x['attention'] or {}).get('hold'));self.assertEqual(t['column'],'assigned');rid=t['runs'][-1]['id'];pid=int((self.root/'.solace'/(rid+'.pid')).read_text());os.kill(pid,0)
        self.assertFalse((self.root/'.solace'/(rid+'.responses')).exists())
        time.sleep(2.2)  # Waiting does not consume the two-second task timeout.
        self.assertEqual(self.get(t)['runs'][-1]['state'],'waiting')
        events=self.stream()
        while True:
            _,event=self.next_event(events)
            asked=[q for x in event.get('tasks',[]) for q in x.get('questions',[])]+event.get('task',{}).get('questions',[])
            if asked:
                self.assertEqual(asked[0]['text'],'Which?');break
        other=self.create();self.action(other,'plan');self.action(other,'assign','E7005',model='fake')
        self.action(t,'review','E7003',model='fake')
        self.api('PATCH','/tasks/'+t['id'],{'title':'blocked edit'},'E7003')
        self.action(t,'unblock','E7002')
        self.api('POST',f'/tasks/{t["id"]}/questions/{t["questions"][0]["id"]}',{'option':'a'})
        t=self.wait(t,lambda x:x['column']=='needs_review');self.assertEqual(t['runs'][-1]['id'],rid)
        self.assertIsNone(t['attention'])
        self.api('POST',f'/tasks/{t["id"]}/questions/{t["questions"][0]["id"]}',{'option':'a'})
        self.assertEqual(len(self.get(t)['runs']),1)  # Retried answers never launch a new agent.
        response=json.loads((self.root/'.solace'/(rid+'.responses')).read_text().splitlines()[0]);self.assertEqual(json.loads(response['result']['content'][0]['text'])['questions'][0]['answer'],'a')
        t=self.assign([{'command':'task ask','text':'Again?','options':['a','b']},{'command':'task done answered'}]);t=self.wait(t,lambda x:(x['attention'] or {}).get('hold'));old=t['runs'][-1]['id']
        t=self.action(t,'new_agent',text='b');new=t['runs'][-1]['id'];self.assertNotEqual(old,new)
        self.wait(t,lambda x:len(x['questions'])==2)
        prompt=(self.root/'.solace'/(new+'.prompt')).read_text();self.assertIn("Answer: b",prompt);self.assertNotIn('"runs"',prompt)
        self.action(t,'unblock',text='b');self.wait(t,lambda x:x['column']=='needs_review')
    def test_a17_agents_md_proposal(self):  # D-45: AGENTS.md is human-owned like solace.toml
        agents=self.root/'AGENTS.md'
        t=self.assign([{'command':'write AGENTS.md','text':'Prices are in cents.\n'},{'command':'task done done'}])
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting');q=t['questions'][0]
        self.assertEqual(q['text'],f'Change {self.root.name}/AGENTS.md');self.assertFalse(agents.exists())
        self.api('POST',f'/tasks/{t["id"]}/questions/{q["id"]}',{'option':'Approve'})
        self.wait(t,lambda x:x['column']=='needs_review');self.assertEqual(agents.read_text(),'Prices are in cents.\n')
        agents.unlink()
    def test_a17_settings_proposals(self):
        settings=self.root/'solace.toml';original=settings.read_text()
        add={'command':'write solace.toml --replace','old':'unwatched=["priv/gen/"]','text':'unwatched=["priv/gen/"]\n[commands]\nbuild={run=["true"]}'}
        t=self.assign([{'command':'write solace.toml','text':'watch_limit=-1'},add,add,{'command':'run build'},
                       {'command':'write solace.toml --replace','old':'unwatched','text':'watch_limit=1\nunwatched'},{'command':'task done done'}])
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting');rid=t['runs'][-1]['id']
        self.assertEqual(len(t['questions']),1);q=t['questions'][0]  # the invalid proposal added none
        self.assertEqual(q['options'],['Approve','Reject']);self.assertEqual(q['settings']['base'],original);self.assertIn('build',q['settings']['proposed'])
        self.assertIn('settings change',t['attention']['reason'])
        self.api('POST',f'/tasks/{t["id"]}/questions/{q["id"]}',{'option':'Reject'})
        t=self.wait(t,lambda x:len(x['questions'])==2 and x['runs'][-1]['state']=='waiting');self.assertEqual(settings.read_text(),original)
        self.api('POST',f'/tasks/{t["id"]}/questions/{t["questions"][1]["id"]}',{'option':'Approve'})
        t=self.wait(t,lambda x:len(x['questions'])==3 and x['runs'][-1]['state']=='waiting');approved=settings.read_text();self.assertIn('build',approved)
        settings.write_text(approved+'# human edit\n')
        self.api('POST',f'/tasks/{t["id"]}/questions/{t["questions"][2]["id"]}',{'option':'Approve'})
        t=self.wait(t,lambda x:x['column']=='needs_review');self.assertNotIn('watch_limit',settings.read_text())
        self.assertTrue(t['questions'][2]['answer'].startswith('Not applied'));self.assertEqual(t['runs'][-1]['id'],rid)
        responses=[json.loads(l)['result']['content'][0]['text'] for l in (self.root/'.solace'/(rid+'.responses')).read_text().splitlines()]
        self.assertIn('E6020',responses[0]);self.assertIn('Reject',responses[1]);self.assertIn('Approved',responses[2]);self.assertIn('exit code: 0',responses[3])
        review=self.completed([]);self.api('PATCH','/tasks/'+review['id'],{'body':json.dumps([add,{'command':'task note r'}])})
        self.action(review,'review',model='fake');review=self.wait(review,lambda x:x['runs'][-1]['state']=='ended')
        self.assertIn('E7010',(self.root/'.solace'/(review['runs'][-1]['id']+'.responses')).read_text());self.assertFalse(review['questions'])
    def test_answers_respect_pause_and_all_questions(self):
        config=self.root/'solace.toml'
        workflow=Path(__file__).parents[1].joinpath('src/default-board.toml').read_text()
        config.write_text(config.read_text()+workflow.replace('unblock','continue_work'))
        t=self.assign([{'command':'task ask','text':'First?','options':['a','b']},{'command':'task done answered'}])
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting');rid=t['runs'][-1]['id']
        # Seed a second pending question to exercise the all-answers condition.
        t['questions'].append({'id':'second','text':'Second?','options':['a','b']});t['home']='.'
        (self.root/'.solace/tasks'/f'{t["id"]}.json').write_text(json.dumps(t))
        t=self.api('POST',f'/tasks/{t["id"]}/questions/{t["questions"][0]["id"]}',{'option':'a'})
        self.assertEqual(t['runs'][-1]['state'],'waiting')
        self.action(t,'pause')
        t=self.api('POST',f'/tasks/{t["id"]}/questions/second',{'text':'custom answer'})
        self.assertEqual(t['runs'][-1]['state'],'waiting');self.assertEqual(t['paused_from'],'assigned')
        self.assertFalse((self.root/'.solace'/(rid+'.responses')).exists())
        t=self.action(t,'resume')
        self.assertEqual(t['column'],'assigned');self.assertEqual(t['runs'][-1]['state'],'running')
        t=self.wait(t,lambda x:x['column']=='needs_review');self.assertEqual(t['runs'][-1]['id'],rid)
        response=json.loads((self.root/'.solace'/(rid+'.responses')).read_text().splitlines()[0])
        self.assertEqual(json.loads(response['result']['content'][0]['text'])['questions'][1]['answer'],'custom answer')

    def test_a6_revert_a7b_invisible(self):
        self.cli('build') # canonical projection before comparing byte for byte
        before=(self.root/'src/app.gleam').read_bytes()
        (self.root/'README.md').write_text('original docs')
        (self.root/'obsolete.txt').write_text('restore me')
        t=self.completed([
            {'command':'submit app.one','text':'pub fn one() { 10 }'},
            {'command':'submit app.two','text':'pub fn two() { 20 }'},
            {'command':'submit --new app.new','text':'pub fn new() { 30 }'},
            {'command':'delete app.removed'},
            {'command':'read priv/a'}, {'command':'write priv/a','text':'edited'},
            {'command':'write priv/new','text':'created'},
            {'command':'read README.md'}, {'command':'write README.md','text':'updated docs'},
            {'command':'write bin/coolcalc','text':'#!/bin/sh\ngleam run -- "$@"\n'},
            {'command':'remove obsolete.txt'},
        ])
        self.assertEqual(len(t['changes']),9)
        for command in [('find','removed'),('view','app','--bodies'),('session','start','--task','removed')]:
            text=self.cli(*command);self.assertNotIn('pub fn removed()',text);self.assertNotIn('pub fn one() {\n  1\n}',text)
        # Every semantic read is identical with and without the private before rows.
        commands=[('find','removed'),('inspect','app.one'),('view','app','--bodies'),('refs','app.one'),('session','start','--task','removed one')]
        with_before=[self.cli(*args) for args in commands]
        db=sqlite3.connect(self.root/'.solace/store.db')
        rows=db.execute('SELECT * FROM board_before').fetchall()
        db.execute('DELETE FROM board_before');db.commit()
        self.assertEqual(with_before,[self.cli(*args) for args in commands])
        db.executemany('INSERT INTO board_before VALUES (?,?,?)',rows);db.commit();db.close()
        self.action(t,'cancel')
        self.assertEqual((self.root/'src/app.gleam').read_bytes(),before)
        self.assertEqual((self.root/'priv/a').read_bytes(),b'original');self.assertFalse((self.root/'priv/new').exists())
        self.assertEqual((self.root/'README.md').read_text(),'original docs')
        self.assertEqual((self.root/'obsolete.txt').read_text(),'restore me')
        self.assertFalse((self.root/'bin/coolcalc').exists())
    def test_a7_overlap_and_a7c_dependent_refusal(self):
        a=self.completed([{'command':'submit app.one','text':'pub fn one() { 10 }'}]);b=self.completed([{'command':'submit app.one','text':'pub fn one() { 20 }'}])
        e=self.action(a,'cancel','E7007');self.assertIn(b['id'],e['message']);self.assertIn('app.one',e['message'])
        self.action(b,'cancel');self.action(a,'cancel')
        a=self.completed([{'command':'submit --new app.added','text':'pub fn added() { 99 }'}]);b=self.completed([{'command':'submit --new app.caller','text':'pub fn caller() { added() }'}])
        before=(self.root/'src/app.gleam').read_bytes();e=self.action(a,'cancel','E7008');self.assertIn(b['id'],e['message']);self.assertIn('caller',e['message']);self.assertEqual((self.root/'src/app.gleam').read_bytes(),before)
        self.action(b,'cancel');self.action(a,'cancel')
    def test_a8_pause_and_a9_readonly(self):
        t=self.assign([{'sleep':.4},{'command':'task note alive'},{'command':'task done done'}]);self.action(t,'pause');time.sleep(2.3)
        current=self.get(t);self.assertEqual(current['runs'][-1]['state'],'running');self.assertFalse(current['notes']);self.assertEqual(current['paused_from'],'assigned')
        self.action(t,'resume');self.wait(t,lambda x:x['column']=='needs_review')
        before=self.cli('view','app.one')
        steps=[{'command':'write priv/new','text':'no'},{'command':'submit app.one','text':'pub fn one() { 100 }'}]
        t=self.create([*steps,{'command':'task ask','text':'Which?','options':['a','b']},{'command':'task note --text "### Cut'},{'command':'task done report'}]);self.action(t,'plan')
        self.action(t,'investigate',model='fake');t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting')
        self.assertEqual(t['column'],'planning');self.assertIn('question',t['attention']['reason']);rid=t['runs'][-1]['id']
        self.action(t,'investigate','E7002')
        self.api('POST',f'/tasks/{t["id"]}/questions/{t["questions"][0]["id"]}',{'text':'a'})
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='ended');self.assertEqual(t['runs'][-1]['id'],rid);self.assertEqual(t['runs'][-1]['outcome'],'reported')
        self.assertEqual(t['column'],'planning');self.assertEqual(t['notes'][-1]['text'],'report');self.assertFalse(t['changes'])
        prompt=(self.root/'.solace'/(rid+'.prompt')).read_text()
        self.assertIn('Investigate this task',prompt)
        self.assertIn('This run is read-only',prompt)
        self.assertEqual(before,self.cli('view','app.one'));self.assertFalse((self.root/'priv/new').exists())
        responses=(self.root/'.solace'/(rid+'.responses')).read_text();self.assertEqual(responses.count('E7010'),2)
        self.assertIn('E7015',responses);self.assertFalse([n for n in t['notes'] if 'Cut' in n['text']])  # D-79
        self.assertEqual(self.action(t,'complete')['column'],'complete')  # an answered question ends after planning
        review=self.completed([]);self.api('PATCH','/tasks/'+review['id'],{'body':json.dumps([*steps,{'command':'task note review report'}])})
        # Starting a run on a paused task ends the pause instead of leaving the run hung.
        self.action(review,'pause');self.assertIsNone(self.action(review,'review',model='fake')['paused_from'])
        review=self.wait(review,lambda x:x['runs'][-1]['state']=='ended');self.assertEqual(review['column'],'needs_review')
        self.assertEqual(review['notes'][-1]['text'],'review report')
    def test_board_reads_while_agent_command_runs(self):
        config=self.root/'solace.toml'
        config.write_text(config.read_text()+'\n[commands]\nslow={run=["sleep","3"]}\n')
        t=self.assign([{'command':'run slow'},{'command':'task done done'}])
        time.sleep(1)  # the agent's command now holds the project lock
        start=time.monotonic();self.api('GET','/tasks');self.get(t)
        self.assertLess(time.monotonic()-start,1)
        self.wait(t,lambda x:x['runs'][-1]['state']=='ended')  # the fixture's 2-second timeout ends it
    def test_a_task_command_leaves_a_live_runs_records(self):
        # D-77 point 5: a task command takes no project lock, so it must not
        # recover a live run's records as a crashed one's.
        config=self.root/'solace.toml'
        script="import time; from pathlib import Path; Path('priv/a').write_text('changed'); Path('priv/gen').mkdir(exist_ok=True); Path('priv/gen/marker').write_text('x'); time.sleep(1.5)"
        config.write_text(config.read_text()+'\n[commands]\nslow={run=["python3","-c",'+json.dumps(script)+']}\n')
        t=self.assign([{'command':'run slow'}])
        rid=t['runs'][-1]['id'];pending=self.root/'.solace/pending'
        end=time.monotonic()+5
        while not (self.root/'priv/gen/marker').exists():
            self.assertLess(time.monotonic(),end);time.sleep(.02)
        start=time.monotonic()
        p=subprocess.run([BIN,'task','note','x'],cwd=self.root,env={**self.env,'SOLACE_TASK':t['id'],'SOLACE_RUN':rid},text=True,capture_output=True)
        self.assertEqual(p.returncode,0,p.stdout+p.stderr)
        self.assertLess(time.monotonic()-start,1)
        self.assertTrue((pending/f'run-{rid}.json').exists());self.assertTrue((pending/f'command-{rid}.json').exists())
        self.assertEqual(self.rows(t['id']),[])
        self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
        self.assertEqual(self.rows(t['id']),['file:priv/a'])
        db=sqlite3.connect(self.root/'.solace/store.db')
        self.assertEqual(db.execute("SELECT COUNT(*) FROM board_touches WHERE task=?",[t['id']]).fetchone()[0],1)
        self.action(t,'cancel')
        self.assertEqual((self.root/'priv/a').read_text(),'original')
    def test_task_only_changes_wait_for_no_agent_command(self):
        # D-77 point 5: create, a note and stop take no project lock, so they
        # finish while an agent's command holds it.
        config=self.root/'solace.toml'
        script="import time; from pathlib import Path; Path('priv/gen').mkdir(exist_ok=True); Path('priv/gen/marker').write_text('x'); time.sleep(1.5)"
        config.write_text(config.read_text()+'\n[commands]\nslow={run=["python3","-c",'+json.dumps(script)+']}\n')
        t=self.assign([{'command':'run slow'}])
        end=time.monotonic()+5
        while not (self.root/'priv/gen/marker').exists():
            self.assertLess(time.monotonic(),end);time.sleep(.02)
        for method,path,body in [('POST','/tasks',{'title':'meanwhile'}),('POST',f'/tasks/{t["id"]}/notes',{'text':'meanwhile'}),('POST',f'/tasks/{t["id"]}/actions',{'action':'stop'})]:
            start=time.monotonic();self.api(method,path,body)
            self.assertLess(time.monotonic()-start,1,path)
        self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
    def test_declared_run_watched_files_revert(self):
        config=self.root/'solace.toml'
        config.write_text(config.read_text()+'\n[commands]\nedit={run=["python3","-c",'+json.dumps("from pathlib import Path; Path('priv/a').write_text('changed'); Path('priv/created').write_text('new')")+']}\n')
        t=self.completed([{'command':'run edit'}])
        self.assertEqual(len(t['changes']),2)
        self.assertEqual((self.root/'priv/a').read_text(),'changed')
        self.action(t,'cancel')
        self.assertEqual((self.root/'priv/a').read_text(),'original');self.assertFalse((self.root/'priv/created').exists())

    def restart(self,env):
        # Restarts the server with the test's env, so the agent's Solace process gets it.
        self.server.terminate();self.server.wait(timeout=3)
        self.env={**self.env,**env}
        self.serve()
    def rows(self,tid):
        db=sqlite3.connect(self.root/'.solace/store.db')
        return sorted(r[0] for r in db.execute("SELECT entity FROM board_touches WHERE task=?",[tid]))
    def test_a_crash_between_a_symbol_change_and_its_undo_leaves_neither(self):
        self.restart({'SOLACE_FAIL_AFTER':'command'})
        t=self.assign([{'command':'submit app.one','text':'pub fn one() { 10 }'}])
        self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
        self.cli('view','app.one')  # recovery runs at the start of this command
        self.assertEqual((self.root/'src/app.gleam').read_text(),'pub fn one() { 1 }\n\npub fn two() { 2 }\n\npub fn removed() { 3 }\n')
        self.assertEqual(self.rows(t['id']),[])
    def test_a_crash_after_a_file_write_keeps_its_undo(self):
        self.restart({'SOLACE_FAIL_AFTER':'command'})
        t=self.assign([{'command':'read priv/a'},{'command':'write priv/a','text':'changed'}])
        self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
        self.cli('read','priv/a')
        self.assertIn('file:priv/a',self.rows(t['id']))
        self.assertEqual((self.root/'priv/a').read_text(),'changed')
        self.action(t,'cancel')
        self.assertEqual((self.root/'priv/a').read_text(),'original')
    def test_a_crash_after_a_declared_run_adds_its_rows(self):
        config=self.root/'solace.toml'
        config.write_text(config.read_text()+'\n[commands]\nedit={run=["python3","-c",'+json.dumps("from pathlib import Path; Path('priv/a').write_text('changed'); Path('priv/created').write_text('new')")+']}\n')
        self.restart({'SOLACE_FAIL_AFTER':'command'})
        t=self.assign([{'command':'run edit'}])
        self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
        self.cli('read','priv/a')
        self.assertEqual(self.rows(t['id']),['file:priv/a','file:priv/created'])
        self.action(t,'cancel')
        self.assertEqual((self.root/'priv/a').read_text(),'original')
        self.assertFalse((self.root/'priv/created').exists())
    def test_a_crash_after_commit_adds_its_undo_once(self):
        self.restart({'SOLACE_FAIL_AFTER':'commit'})
        t=self.assign([{'command':'read priv/a'},{'command':'write priv/a','text':'changed'}])
        self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
        self.cli('read','priv/a')
        db=sqlite3.connect(self.root/'.solace/store.db')
        self.assertEqual(db.execute("SELECT COUNT(*) FROM board_touches WHERE task=? AND entity='file:priv/a'",[t['id']]).fetchone()[0],1)
        self.action(t,'cancel')
        self.assertEqual((self.root/'priv/a').read_text(),'original')

    def test_a11_embedded_client_and_internal_files(self):
        for path,expected in [('/',b'Solace tasks'),('/migrate',b'Solace tasks'),('/app.js',b'/workflow'),('/guide',b'Using Solace')]:
            c=http.client.HTTPConnection('127.0.0.1',self.port);c.request('GET',path);r=c.getresponse();self.assertEqual(r.status,200);self.assertIn(expected,r.read());c.close()
        for command in ['files','read','write','remove']:
            p=subprocess.run([BIN,command,'.solace/board'],cwd=self.root,env=self.env,input='x',text=True,capture_output=True)
            self.assertIn('E6006',p.stdout)

    def test_models_availability_and_attention(self):
        models={m['id']:m for m in self.api('GET','/models')}
        self.assertTrue(models['fake']['available']);self.assertIsNone(models['fake']['reason'])
        self.assertFalse(models['missing']['available']);self.assertIn('not installed',models['missing']['reason'])
        self.assertIn('codex-gpt-6-astra',models)  # built-in presets are listed too
        t=self.create();self.action(t,'plan')
        e=self.action(t,'assign','E7011',model='missing');self.assertIn('not installed',e['message'])
        self.assertEqual(self.get(t)['column'],'planning')
        t=self.action(t,'assign',model='fake')
        t=self.wait(t,lambda x:(x['attention'] or {}).get('hold'))  # the fake agent has no steps and exits
        self.assertEqual(t['column'],'assigned')
        self.assertTrue(t['attention']['reason'])
        asked=self.assign([{'command':'task ask','text':'Which?','options':['a','b']}])
        asked=self.wait(asked,lambda x:(x['attention'] or {}).get('hold'));self.assertIn('question',asked['attention']['reason'])
        self.assertEqual(asked['column'],'assigned')
        # Held attention survives an unrelated action while the question is open (D-82).
        asked=self.action(asked,'pause');self.assertTrue(asked['attention']['hold']);asked=self.action(asked,'resume');self.assertTrue(asked['attention']['hold'])
        self.assertIsNone(self.action(asked,'cancel')['attention'])

    def test_a10_events_reconnect(self):
        stream=self.stream()
        while stream.readline()!=b'event: ready\n':pass  # after the replay, once
        self.assertEqual(stream.readline(),b'data: {}\n')
        # D-78: a new stream starts from the project's workflow and tasks, then follows the changes.
        stream=self.stream();seq,e=self.next_event(stream);self.assertIn('workflow',e)
        seq,e=self.next_event(stream);self.assertIsInstance(e['tasks'],list)
        t=self.create();seq,e=self.next_event(stream);self.assertEqual(e['task']['id'],t['id'])
        self.api('POST','/tasks/'+t['id']+'/notes',{'text':'first'});self.api('POST','/tasks/'+t['id']+'/notes',{'text':'second'})
        resumed=self.stream(seq);s1,e1=self.next_event(resumed);s2,e2=self.next_event(resumed)
        self.assertEqual(json.loads(s2)['tasks'],json.loads(s1)['tasks']+1);self.assertEqual(e1['task']['notes'][-1]['text'],'first');self.assertEqual(e2['task']['notes'][-1]['text'],'second')

    def test_a_new_stream_starts_from_the_task_files(self):
        # D-78: a task file changed without an event shows in a new stream's snapshot.
        t=self.create();path=self.root/'.solace/tasks'/(t['id']+'.json')
        v=json.loads(path.read_text());v['title']='Edited on disk';path.write_text(json.dumps(v))
        stream=self.stream()
        while True:
            _,e=self.next_event(stream)
            if 'tasks' in e:break
        self.assertEqual([x['title'] for x in e['tasks'] if x['id']==t['id']],['Edited on disk'])
        self.assertNotIn('notes',e['tasks'][0])

    def test_delete_note(self):
        t=self.create()
        for text in ['a','b','c']:self.api('POST','/tasks/'+t['id']+'/notes',{'text':text})
        t=self.api('DELETE','/tasks/'+t['id']+'/notes/1');self.assertEqual([n['text'] for n in t['notes']],['a','c'])
        self.api('DELETE','/tasks/'+t['id']+'/notes/2',code='E7001')

    def test_a_request_creates_one_task(self):
        # D-77: a repeated create, in turn or at once, returns the first one's task.
        body={'title':'Once','request':'r-1'}
        first=self.api('POST','/tasks',body);again=self.api('POST','/tasks',body)
        self.assertEqual(again['id'],first['id']);self.assertEqual(first['request'],'r-1')
        import concurrent.futures
        with concurrent.futures.ThreadPoolExecutor(8) as pool:
            made=list(pool.map(lambda _:self.api('POST','/tasks',{'title':'Burst','request':'r-2'})['id'],range(8)))
        self.assertEqual(len(set(made)),1)
        self.assertEqual(sorted(t['title'] for t in self.api('GET','/tasks')),['Burst','Once'])
        self.api('POST','/tasks',{'title':'Bad','request':7},code='E7001')

    def test_inbox_is_applied_after_a_restart(self):
        # D-77: a message waits in the inbox while no server runs, and is applied when one starts.
        t=self.create()
        self.server.terminate();self.server.wait(timeout=3);self.server.stdout.close();self.server.stderr.close()
        inbox=self.root/'.solace/tasks/inbox';inbox.mkdir(exist_ok=True)
        name=f'{time.time_ns():024}-x'
        (inbox/(name+'.json')).write_text(json.dumps({'id':name,'kind':'note','task':t['id'],'author':'solace','text':'queued'}))
        self.serve()
        self.assertEqual([n['text'] for n in self.wait(t,lambda x:x['notes'])['notes']],['queued'])
        self.assertEqual(list(inbox.iterdir()),[])

    def test_a_stopped_server_queues_a_note(self):
        # applied() waits for the server, not forever: with no server, the note
        # stays in the inbox for the next serve, and the agent's call returns.
        t=self.create([{'sleep':.5},{'command':'task note reply'}]);self.action(t,'plan')
        t=self.action(t,'discuss',model='fake',text='What about X?')
        rid=t['runs'][-1]['id']
        prompt=self.root/'.solace'/(rid+'.prompt')
        end=time.monotonic()+8
        while time.monotonic()<end and not prompt.exists():time.sleep(.05)
        self.assertTrue(prompt.exists(),'the discussion agent never started')
        self.server.terminate();self.server.wait(timeout=3);self.server.stdout.close();self.server.stderr.close()
        responses=self.root/'.solace'/(rid+'.responses')
        end=time.monotonic()+4
        while time.monotonic()<end and not responses.exists():time.sleep(.1)
        self.assertTrue(responses.exists(),'the note call never returned')
        self.assertIn('reaches the board',responses.read_text())
        inbox=self.root/'.solace/tasks/inbox'
        self.assertEqual(len(list(inbox.iterdir())),1)
        self.serve()
        t=self.wait(t,lambda x:len(x['notes'])==2)
        self.assertEqual([n['text'] for n in t['notes']],['What about X?','reply'])
        self.assertEqual(list(inbox.iterdir()),[])

    def test_a_stopped_server_queues_an_ask(self):
        # The same for a question: the agent is told it is queued, not left waiting.
        t=self.assign([{'sleep':.5},{'command':'task ask','text':'Which?','options':['a','b']}])
        rid=t['runs'][-1]['id']
        prompt=self.root/'.solace'/(rid+'.prompt')
        end=time.monotonic()+8
        while time.monotonic()<end and not prompt.exists():time.sleep(.05)
        self.assertTrue(prompt.exists(),'the work agent never started')
        self.server.terminate();self.server.wait(timeout=3);self.server.stdout.close();self.server.stderr.close()
        responses=self.root/'.solace'/(rid+'.responses')
        end=time.monotonic()+4
        while time.monotonic()<end and not responses.exists():time.sleep(.1)
        self.assertTrue(responses.exists(),'the ask call never returned')
        self.assertIn('reaches the board',responses.read_text())
        self.serve()
        # The restart ended the run before the question was applied: the question
        # is kept as a note, and does not wake the dead run or ask anyone.
        t=self.wait(t,lambda x:x['notes'])
        self.assertEqual(t['notes'][0]['text'],'Which?')
        self.assertEqual(t['questions'],[])
        self.assertEqual(t['runs'][-1]['state'],'ended')
        self.assertEqual(list((self.root/'.solace/tasks/inbox').iterdir()),[])

if __name__=='__main__':unittest.main()
