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

from acceptance_wait import wait_for

BIN = os.environ['SOLACE_TEST_BIN']
FAKE = str(Path(__file__).with_name('fake_agent.py'))

class Board(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='solace-board-', ignore_cleanup_errors=True)
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
        # Discovery: the fake Codex answers `model/list` from this fixture; the
        # fake agents' start events name this model as the one their run used;
        # an empty key keeps Claude's aliases without a network call.
        self.env['SOLACE_FAKE_MODELS']=str(Path(FAKE).with_name('fake_models.json'))
        self.env['SOLACE_FAKE_USED_MODEL']='claude-opus-5-5'
        self.env['OPENAI_API_KEY']='fixture-key'
        self.env['ANTHROPIC_API_KEY']=''
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
    def stop(self):
        self.server.terminate();self.server.wait(timeout=3)
        self.server.stdout.close();self.server.stderr.close()
    def restart(self,env=None):
        # Restarts the server with the test's env, so the agent's Solace process gets it.
        self.stop()
        self.env={**self.env,**(env or {})}
        self.serve()
    def tearDown(self):
        # End all fake processes before terminating their supervisor.
        try:
            alive=self.server.poll() is None
            runs=[r for t in (self.api('GET','/tasks') if alive else []) for r in self.api('GET','/tasks/'+t['id'])['runs']]
            for r in runs+(self.api('GET','/strategy/runs') if alive else []):
                p=self.root/'.solace'/(r['id']+'.pid')
                if p.exists():
                    try: os.killpg(int(p.read_text()),9)
                    except ProcessLookupError: pass
        finally:
            for c in self.clients: c.close()
            self.stop()
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
            # D-151: the fresh run's brief holds the session start reply.
            fresh = (self.root/'.solace'/(first+'.prompt')).read_text()
            self.assertIn('Session start (the reply to `session start --task`)', fresh)
            self.assertIn('\nNext: read what the change needs in one call', fresh)
            self.assertNotIn('Start with `session start', fresh)
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
            self.assertNotIn('Session start (the reply', prompt)
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
            self.assertIn("The task's changes are below: the delta since the last review", prompt)
            self.assertIn('Since completed review', prompt)
            self.assertNotIn('Start with', prompt)
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

    def test_a_director_brief_reaches_the_agent(self):
        # The fake agent can't judge wording, but the prompt it receives must carry the Director section.
        config = self.root/'system.toml'
        config.write_text('involvement = "director"\n' + config.read_text())
        t = self.completed([])
        prompt = (self.root/'.solace'/(t['runs'][-1]['id']+'.prompt')).read_text()
        self.assertIn('How to talk to the human (Director)', prompt)
        self.assertIn('`### Calls made` heading', prompt)
        self.assertIn('`### Summary` heading', prompt)
        self.assertNotIn('How to talk to the human (Collaborator)', prompt)

    def review(self, t, steps=None, model='fake'):
        if steps is not None:
            self.api('PATCH', '/tasks/'+t['id'], {'body':json.dumps(steps)})
        t = self.action(t, 'review', model=model)
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
        self.assertIn('No usable delta baseline: no completed review', prompt)
        self.assertIn('## app.one (changed)', prompt)
        self.assertIn('falling back', replies[0])
        first = t['runs'][-1]['id']
        t, prompt, replies = self.review(t)
        self.assertIn('Code and relevant context are unchanged', prompt)
        self.assertIn('must not turn Needs work into Looks good', prompt)
        self.assertIn('A prior finding matters only if it still blocks under the Needs work bar', prompt)
        self.assertIn('a new edge case that a fix exposes is mentioned', prompt)
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
        self.wait(t, lambda x: not checkpoint.parent.exists())

    def test_a_review_without_a_note_records_its_assessment(self):
        assessment = '**Assessment: Looks good**\n\n### Findings\n\nFine.'
        t = self.completed([])
        t, _, _ = self.review(t, [{'say': assessment}])
        rid = t['runs'][-1]['id']
        self.assertEqual(t['runs'][-1]['outcome'], 'reviewed')
        self.assertEqual(t['finished']['outcome'], 'reviewed')
        self.assertEqual(t['column'], 'needs_review')
        note = t['notes'][-1]
        self.assertEqual((note['author'], note['run'], note['text']), ('agent', rid, assessment))
        self.assertIsNone(t['attention'])
        # A clean review with no assessment still fails.
        t = self.completed([])
        t, _, _ = self.review(t, [{'say': 'No assessment.'}])
        self.assertNotEqual(t['runs'][-1]['outcome'], 'reviewed')
        self.assertEqual(t['column'], 'needs_review')
        self.assertIn('without finishing', t['attention']['reason'])

    def test_a_review_that_recovered_from_a_transient_notice_records_its_assessment(self):
        assessment = '**Assessment: Looks good**\n\n### Findings\n\nFine.'
        t = self.completed([])
        t, _, _ = self.review(t, [{'notice': 'Reconnecting... 1/5'}, {'say': assessment}])
        rid = t['runs'][-1]['id']
        self.assertEqual(t['runs'][-1]['outcome'], 'reviewed')
        self.assertEqual((t['notes'][-1]['author'], t['notes'][-1]['run'], t['notes'][-1]['text']), ('agent', rid, assessment))
        self.assertIsNone(t['attention'])
        # An error after the last message still fails.
        t = self.completed([])
        t, _, _ = self.review(t, [{'say': assessment}, {'notice': 'reconnect failed'}])
        self.assertEqual(t['column'], 'needs_review')
        self.assertEqual(
            t['runs'][-1]['outcome'],
            f'The agent ended its run without finishing it (exit status: 0):\n\n{assessment}'
        )
        self.assertIn('without finishing', t['attention']['reason'])

    def test_a_claude_reply_split_into_blocks_records_the_whole_assessment(self):
        assessment = '**Assessment: Looks good**\n\n### Findings\n\nFine.'
        t = self.completed([])
        t, _, _ = self.review(t, [{'say': assessment}, {'say': 'Done.'}], model='native-claude')
        rid = t['runs'][-1]['id']
        self.assertEqual(t['runs'][-1]['outcome'], 'reviewed')
        self.assertEqual((t['notes'][-1]['run'], t['notes'][-1]['text']), (rid, assessment+'\n\nDone.'))
        log = [json.loads(line) for line in Path(t['runs'][-1]['log']).read_text().splitlines()]
        self.assertEqual([e['message']['content'][0]['text'] for e in log if e.get('type')=='assistant'], [assessment, 'Done.'])

    def test_completed_reviews_file_tech_debt(self):  # D-112
        f = self.api('POST', '/features', {'title':'Search'})
        def reviewed(feature, finding):
            t = self.completed([])
            if feature: self.api('PATCH', '/tasks/'+t['id'], {'feature':f['id']})
            note = '**Assessment: Looks good**\n\n### Tech debt\n\n- `app.one`: '+finding+'\n\n### Verification\n\n- ok\n'
            t, prompt, _ = self.review(t, [{'command':'task note','text':note}])
            # Every review is asked for tech debt.
            self.assertIn('Put minor, non-blocking findings', prompt)
            self.assertIn("Choose **Needs work** only when the change doesn't meet the task's goal or acceptance criteria", prompt)
            self.assertIn('file an item only when fixing it clearly pays off', prompt)
            # D-138: the prompt names the task's feature.
            self.assertEqual('\nFeature (the feature this task is part of; its state and body, then its latest direction):\n- Search\n- state: proposed\n' in prompt, feature)
            self.action(t, 'approve'); return self.action(t, 'complete')
        debts = lambda: [t for t in self.api('GET', '/tasks') if t.get('techDebt')]
        # A review's tech debt waits for completion, then makes the feature's Tech Debt task once.
        a = reviewed(True, 'first finding')
        [debt] = debts()
        self.assertEqual((debt['title'], debt['column'], debt['feature']), ('Search: Tech Debt', 'planning', f['id']))
        notes = self.get(debt)['notes']
        self.assertEqual(len(notes), 1)
        self.assertIn('task:'+a['id'], notes[0]['text']); self.assertIn('first finding', notes[0]['text'])
        self.assertIn('task:'+debt['id'], a['notes'][-1]['text'])
        # A second review in the feature appends to it.
        b = reviewed(True, 'second finding')
        [again] = debts()
        self.assertEqual(again['id'], debt['id'])
        self.assertEqual([('task:'+x) in n['text'] for n, x in zip(self.get(debt)['notes'], [a['id'], b['id']])], [True, True])
        # A review of a task with no feature makes a Tech Debt task of its own.
        c = reviewed(False, 'own finding')
        own = next(d for d in debts() if d['title'] == c['title'])
        self.assertEqual((own['column'], own.get('feature'), own.get('milestone')), ('planning', None, None))
        self.assertEqual((own['scope'], own['home']), (c['scope'], c['home']))
        own_notes = self.get(own)['notes']
        self.assertEqual(len(own_notes), 1)
        self.assertIn('task:'+c['id'], own_notes[0]['text']); self.assertIn('own finding', own_notes[0]['text'])
        self.assertIn('task:'+own['id'], c['notes'][-1]['text'])
        self.assertEqual(len(self.get(debt)['notes']), 2); self.assertEqual(len(debts()), 2)

    def test_a_work_prompt_carries_the_features_assignment_note(self):  # D-138
        f = self.api('POST', '/features', {'title':'Search'})
        t = self.create([{'command':'task done done'}])
        self.api('PATCH', '/tasks/'+t['id'], {'feature':f['id']})
        self.action(t, 'plan'); t = self.action(t, 'assign', model='fake', text='keep it without a shell')
        t = self.wait(t, lambda x: x['column']=='needs_review')
        prompt = (self.root/'.solace'/(t['runs'][-1]['id']+'.prompt')).read_text()
        self.assertIn('\nFeature (the feature this task is part of; its state and body, then its latest direction):\n- Search\n- state: proposed\n\n- assignment: keep it without a shell\n', prompt)

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

    def test_a_work_run_with_no_effort_takes_medium(self):
        # D-149: no effort chosen, so the run records and passes the model's default.
        t=self.create([{'command':'task done done'}])
        self.action(t,'plan')
        t=self.action(t,'assign',model='native-claude')
        self.assertEqual((t['runs'][-1]['effort'],t['effort']),('medium','medium'))
        rid=t['runs'][-1]['id']
        self.wait(t,lambda x:(self.root/'.solace'/(rid+'.launch')).exists())
        argv=json.loads((self.root/'.solace'/(rid+'.launch')).read_text())['argv']
        self.assertEqual(argv[argv.index('--effort')+1],'medium')

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
                # A final column offers only what it lists (D-108): cancelled's to_planning (D-133), nothing in complete.
                allowed=col['actions'] if col['final'] else [*col['actions'],'pause','resume','discuss','stop','end_discussion']
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

    def test_session_remember_gives_back_the_run_context(self):
        # D-157 point 7: after a compaction the agent asks for its task and window.
        t=self.assign([{'command':'view app.one'},{'command':'session remember'},{'command':'session remember'},{'exit':0}])
        rid=t['runs'][-1]['id']
        self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
        replies=[json.loads(l)['result']['content'][0]['text'] for l in (self.root/'.solace'/(rid+'.responses')).read_text().splitlines()]
        self.assertIn('What this run read or changed since its last change, as it is now:',replies[1])
        self.assertIn('pub fn one() { 1 }',replies[1])
        self.assertEqual(replies[1],replies[2])

    def test_session_remember_after_a_change_in_the_same_call(self):
        # D-157 point 7: each command updates the window, so an ask in the
        # same call sees the change it just made.
        t=self.assign([{'command':'view app.one'},
            {'command':'submit app.one\nsession remember','text':'pub fn one() { 10 }'},
            {'command':'session remember'},{'exit':0}])
        rid=t['runs'][-1]['id']
        self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
        replies=[json.loads(l)['result']['content'][0]['text'] for l in (self.root/'.solace'/(rid+'.responses')).read_text().splitlines()]
        self.assertIn('pub fn one() { 10 }',replies[1])
        self.assertIn('pub fn one() { 10 }',replies[2])

    def test_a_batched_task_done_waits_for_a_passing_test(self):
        # D-152: one call submits, tests and finishes; the test builds and runs, so allow it time.
        config=self.root/'system.toml';config.write_text(config.read_text().replace('task_timeout=2\n','task_timeout=20\n'))
        patch=lambda expected:('*** Begin Patch\n*** Update: app.one\n@@\n-pub fn one() { 1 }\n+pub fn one() { 10 }\n'
            '*** Add: fixture_test\n+//// Tests for app.\n+\n+pub fn main() { one_test() }\n+\n'
            f'+pub fn one_test() {{\n+  let assert {expected} = app.one()\n+}}\n*** End Patch')
        def ran(steps):
            t=self.assign(steps);rid=t['runs'][-1]['id']
            self.wait(t,lambda x:(self.root/'.solace'/(rid+'.exited')).exists())
            reply=json.loads((self.root/'.solace'/(rid+'.responses')).read_text().splitlines()[0])['result']['content'][0]['text']
            return self.get(t),reply
        # A failing test holds back the task done.
        t,reply=ran([{'command':'submit\ntest\ntask done done','text':patch(1)}])
        self.assertIn('1 of 3 commands failed: test; skipped: task done done (the test failed)\n',reply)
        self.assertNotIn('Next:',reply)
        self.assertNotEqual(t['column'],'needs_review');self.action(t,'cancel')
        # A pass that leaves a changed symbol no test reaches holds it back too.
        t,reply=ran([{'command':'submit app.two\ntest\ntask done done','text':'pub fn two() { 20 }'}])
        self.assertIn('No test reaches app.two.\nNext: add a test for app.two; then, if every part of the task is changed, finish with task done.\n',reply)
        self.assertIn('Skipped: task done done (no test reaches app.two)\n',reply)
        self.assertNotEqual(t['column'],'needs_review');self.action(t,'cancel')
        # A passing test alone ends with the next step.
        t,reply=ran([{'command':'submit\ntest','text':patch(10)}])
        self.assertTrue(reply.endswith('Next: if every part of the task is changed, finish with task done.\n'),reply)
        self.action(t,'cancel')
        # A clean pass finishes the task in the same call.
        t=self.assign([{'command':'submit\ntest\ntask done done','text':patch(10)}])
        self.wait(t,lambda x:x['column']=='needs_review')

    def test_a3_custom_workflow_and_invalid(self):
        original=(self.root/'solace.toml').read_text()
        default=Path(__file__).parents[1].joinpath('src/default-board.toml').read_text()
        custom=default.replace('actions = ["plan", "cancel"]','actions = ["triage", "plan", "cancel"]')+'\ntriage = {to="triage"}\n\n[[board.columns]]\nid="triage"\nlabel="Triage"\nactions=["plan"]\n'
        (self.root/'solace.toml').write_text(original+custom)
        t=self.create();self.assertEqual(self.action(t,'triage')['column'],'triage');self.assertEqual(self.action(t,'plan')['column'],'planning')
        for broken in [default.replace('to = "planning"','to = "missing"'),default+'\nunused={to="backlog"}\n',default.replace('finished = "needs_review"',''),default.replace('events = {','events = { asked = "missing",'),default.replace('next = "complete"','next = "missing"'),default.replace('label = "Back to backlog"','label = 1')]:
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
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting' and len(x['notes'])>=4 and len(self.replies(rid))>=1)
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
    def test_discussion_mid_work_notes_arrive_once_in_order(self):
        steps=[{'sleep':.4},{'command':'task note first reply'},{'sleep':.4},
               {'command':'task note second reply'},{'sleep':.4},{'command':'task note third reply'}]
        t=self.create(steps);self.action(t,'plan')
        t=self.action(t,'discuss',model='fake',text='Start');rid=t['runs'][-1]['id']
        self.wait(t,lambda x:(self.root/'.solace'/(rid+'.prompt')).exists())
        for text in ['mid one','mid two']:self.api('POST','/tasks/'+t['id']+'/notes',{'text':text})
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting' and len(x['notes'])>=4 and len(self.replies(rid))>=1)
        self.assertEqual(self.replies(rid)[0],'The human replied:\n\nmid one\n\nmid two\n')
        self.api('POST','/tasks/'+t['id']+'/notes',{'text':'mid three'})
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting' and len(x['notes'])>=7 and len(self.replies(rid))>=2)
        self.assertEqual(self.replies(rid)[1],'The human replied:\n\nmid three\n')
        self.action(t,'end_discussion')
    def test_discussion_note_survives_a_plain_reply(self):
        t=self.create([{'sleep':.6},{'say':'plain reply'}]);self.action(t,'plan')
        t=self.action(t,'discuss',model='fake',text='Start');rid=t['runs'][-1]['id']
        self.wait(t,lambda x:(self.root/'.solace'/(rid+'.prompt')).exists())
        self.api('POST','/tasks/'+t['id']+'/notes',{'text':'mid note'})
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting' and len(x['notes'])==4)
        self.assertIn('mid note',(self.root/'.solace'/(rid+'.prompt')).read_text())
        time.sleep(.4)
        self.assertEqual(len(self.get(t)['notes']),4)  # The fresh brief consumed it once.
        self.action(self.get(t),'end_discussion')
    def test_wait_again_takes_a_wait_only_call(self):
        # A wait that reaches the tool call's limit sends nothing and returns
        # `WAIT_AGAIN`; the wait-only command then takes the human's next note.
        self.restart({'SOLACE_WAIT_LIMIT_SECS':'1'})
        t=self.create([{'command':'task note first reply'},{'command':'task wait'},{'say':'after wait'}])
        self.action(t,'plan')
        t=self.action(t,'discuss',model='fake',text='Start');rid=t['runs'][-1]['id']
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting' and len(self.replies(rid))>=1)
        self.assertIn('`task wait` (a discussion)',self.replies(rid)[0])
        self.api('POST','/tasks/'+t['id']+'/notes',{'text':'after the limit'})
        t=self.wait(t,lambda x:len(self.replies(rid))>=2)
        self.assertEqual(self.replies(rid)[1],'The human replied:\n\nafter the limit\n')
        self.assertEqual(len(t['notes']),3)  # The wait sent no note of its own.
        self.action(t,'end_discussion')
        steps=[{'command':'strategy note','text':'first plan'},{'command':'strategy wait'},{'say':'after wait'}]
        rid=self.api('POST','/strategy/runs',{'model':'fake','text':json.dumps(steps),'context':{'kind':'page'}})['id']
        self.wait_run(rid,lambda r:r['state']=='waiting' and len(self.replies(rid))>=1)
        self.assertIn('`strategy wait` (a strategy run)',self.replies(rid)[0])
        self.api('POST',f'/strategy/runs/{rid}/notes',{'text':'after the limit'})
        run=self.wait_run(rid,lambda r:len(self.replies(rid))>=2)
        self.assertEqual(self.replies(rid)[1],'The human replied:\n\nafter the limit\n')
        self.assertEqual((len(run['notes']),run['questions']),(3,[]))  # No note or proposal from the wait.
        self.api('POST',f'/strategy/runs/{rid}/end')
    def test_a_read_after_wait_again_names_the_wait(self):
        self.restart({'SOLACE_WAIT_LIMIT_SECS':'1'})
        steps=[{'command':'task note first reply'},{'sleep':1.5},{'command':'view app'},
               {'command':'task wait'},{'command':'task bad x'},{'say':'after wait'}]
        t=self.create(steps);self.action(t,'plan')
        t=self.action(t,'discuss',model='fake',text='Start');rid=t['runs'][-1]['id']
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting' and len(self.replies(rid))>=1)
        self.assertIn('`task wait` (a discussion)',self.replies(rid)[0])
        self.api('POST','/tasks/'+t['id']+'/notes',{'text':'after the limit'})
        t=self.wait(t,lambda x:len(self.replies(rid))>=2)
        self.assertIn('the human replied; call `task wait`',self.replies(rid)[1])
        t=self.wait(t,lambda x:len(self.replies(rid))>=3)
        self.assertEqual(self.replies(rid)[2],'The human replied:\n\nafter the limit\n')
        t=self.wait(t,lambda x:len(self.replies(rid))>=4)
        self.assertIn('task wait',self.replies(rid)[3])
        self.action(t,'end_discussion')
        steps=[{'command':'strategy note','text':'first plan'},{'sleep':1.5},{'command':'view app'},
               {'command':'strategy wait'},{'say':'after wait'}]
        rid=self.api('POST','/strategy/runs',{'model':'fake','text':json.dumps(steps),'context':{'kind':'page'}})['id']
        self.wait_run(rid,lambda r:r['state']=='waiting' and len(self.replies(rid))>=1)
        self.assertIn('`strategy wait` (a strategy run)',self.replies(rid)[0])
        self.api('POST',f'/strategy/runs/{rid}/notes',{'text':'after the limit'})
        self.wait_run(rid,lambda r:len(self.replies(rid))>=2)
        self.assertIn('the human replied; call `strategy wait`',self.replies(rid)[1])
        self.wait_run(rid,lambda r:len(self.replies(rid))>=3)
        self.assertEqual(self.replies(rid)[2],'The human replied:\n\nafter the limit\n')
        self.api('POST',f'/strategy/runs/{rid}/end')
    def test_a_proposal_after_wait_again_waits_for_strategy_wait(self):
        self.restart({'SOLACE_WAIT_LIMIT_SECS':'1'})
        steps=[{'command':'strategy task','text':json.dumps({'title':'After limit'})},{'sleep':1.5},
               {'command':'view app'},{'command':'strategy wait'},{'say':'after wait'}]
        rid=self.api('POST','/strategy/runs',{'model':'fake','text':json.dumps(steps),'context':{'kind':'page'}})['id']
        run=self.wait_run(rid,lambda r:r['state']=='waiting' and r['questions'] and len(self.replies(rid))>=1)
        self.assertIn('`strategy wait` (a strategy run)',self.replies(rid)[0])
        self.api('POST',f'/strategy/runs/{rid}/questions/{run["questions"][0]["id"]}',{'option':'Reject'})
        self.wait_run(rid,lambda r:len(self.replies(rid))>=2)
        self.assertIn('the human replied; call `strategy wait`',self.replies(rid)[1])
        self.wait_run(rid,lambda r:len(self.replies(rid))>=3)
        self.assertEqual(self.replies(rid)[2],'The human answered the proposal Create task "After limit": Reject\n')
        self.api('POST',f'/strategy/runs/{rid}/end')
    def test_paused_discussion_holds_a_note_until_resume(self):
        self.restart({'SOLACE_WAIT_LIMIT_SECS':'10'})
        t=self.create([{'command':'task note first reply'},{'say':'after resume'}])
        self.action(t,'plan')
        t=self.action(t,'discuss',model='fake',text='Start');rid=t['runs'][-1]['id']
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting' and any(n['text']=='first reply' for n in x['notes']))
        self.action(t,'pause')
        self.api('POST','/tasks/'+t['id']+'/notes',{'text':'held note'})
        time.sleep(.6)
        self.assertFalse((self.root/'.solace'/(rid+'.responses')).exists())
        t=self.action(t,'resume')
        t=self.wait(t,lambda x:len(self.replies(rid))>=1)
        self.assertEqual(self.replies(rid)[0],'The human replied:\n\nheld note\n')
        self.assertEqual(sum(1 for n in t['notes'] if n['text']=='held note'),1)
        self.action(t,'end_discussion')
    def test_a_plain_reply_keeps_waiting_past_the_tool_limit(self):
        # The server's own wait has no limit: a plain-text reply leaves it
        # waiting for the human, with no new agent launched by the clock.
        self.restart({'SOLACE_WAIT_LIMIT_SECS':'1'})
        t=self.create([{'say':'plain reply'}]);self.action(t,'plan')
        t=self.action(t,'discuss',model='fake',text='Start');rid=t['runs'][-1]['id']
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting' and len(x['notes'])==2)
        pid=(self.root/'.solace'/(rid+'.pid')).read_text()
        time.sleep(1.6)
        t=self.get(t)
        self.assertEqual((len(t['notes']),(self.root/'.solace'/(rid+'.pid')).read_text()),(2,pid))
        self.api('POST','/tasks/'+t['id']+'/notes',{'text':'And Y?'})
        t=self.wait(t,lambda x:len(x['notes'])==4)
        self.assertIn('And Y?',(self.root/'.solace'/(rid+'.prompt')).read_text())
        self.action(t,'end_discussion')
        rid=self.api('POST','/strategy/runs',{'model':'fake','text':json.dumps([{'say':'plain plan'}]),'context':{'kind':'page'}})['id']
        run=self.wait_run(rid,lambda r:r['state']=='waiting' and (self.root/'.solace'/(rid+'.prompt')).exists())
        notes=run['notes']
        pid=(self.root/'.solace'/(rid+'.pid')).read_text()
        time.sleep(1.6)
        run=self.api('GET',f'/strategy/runs/{rid}')
        self.assertEqual(run['notes'],notes)
        self.assertEqual((self.root/'.solace'/(rid+'.pid')).read_text(),pid)
        self.api('POST',f'/strategy/runs/{rid}/notes',{'text':'after the limit'})
        self.wait_run(rid,lambda r:'after the limit' in (self.root/'.solace'/(rid+'.prompt')).read_text())
        self.api('POST',f'/strategy/runs/{rid}/end')
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
        self.action(t,'reassign','E7002')
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
        self.action(t,'reassign',text='b');self.wait(t,lambda x:x['column']=='needs_review')
    def test_a17_gleam_toml_is_written_directly(self):  # D-147: no question, no human edit
        manifest=self.root/'gleam.toml';original=manifest.read_text();new=original+'[dependencies]\n'
        t=self.completed([{'command':'read gleam.toml'},{'command':'write gleam.toml','text':new}])
        replies=(self.root/'.solace'/(t['runs'][-1]['id']+'.responses')).read_text()
        self.assertEqual(t['questions'],[]);self.assertEqual(manifest.read_text(),new,replies)
        manifest.write_text(original)
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
        # No continuing work action in `assigned`: answers still reach the waiting agent (D-88).
        config.write_text(config.read_text()+workflow.replace('actions = ["reassign", "new_agent", "review", "cancel"]','actions = ["new_agent", "review", "cancel"]'))
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
        end=time.monotonic()+5
        while self.rows(t['id'])!=['file:priv/a']:
            self.assertLess(time.monotonic(),end);time.sleep(.02)
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
    def test_a_panicking_command_keeps_the_mcp_server_serving(self):  # task:da797576d0cfaaac
        # A panic at the command point is that line's error, not the end of the
        # tool: the line after it still runs, and nothing of the panicked change
        # is kept.
        self.restart({'SOLACE_FAIL_AFTER':'panic:command'})
        t=self.assign([{'command':'submit app.one','text':'pub fn one() { 10 }'},{'command':'view app.one'}])
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
        first,second=[json.loads(l)['result'] for l in (self.root/'.solace'/(t['runs'][-1]['id']+'.responses')).read_text().splitlines()]
        self.assertTrue(first['isError'])
        self.assertIn('Solace hit an internal error running `submit app.one`',first['content'][0]['text'])
        self.assertFalse(second['isError'])
        self.assertIn('{ 1 }',second['content'][0]['text'])
        self.assertEqual(self.rows(t['id']),[])
        self.assertEqual((self.root/'src/app.gleam').read_text(),'pub fn one() { 1 }\n\npub fn two() { 2 }\n\npub fn removed() { 3 }\n')
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
    def test_a_move_is_recorded_and_undone(self):  # D-84
        (self.root/'priv/d/deep').mkdir(parents=True);(self.root/'priv/d/one').write_text('1');(self.root/'priv/d/deep/two').write_text('2')
        (self.root/'priv/b').write_text('b')
        t=self.completed([{'command':'move priv/a priv/moved/a'},{'command':'move priv/a priv/moved/a'},
                          {'command':'move priv/b priv/moved/a'},{'command':'move priv/d docs/d'},{'command':'task diff'}])
        rid=t['runs'][-1]['id']
        replies=[json.loads(l)['result']['content'][0]['text'] for l in (self.root/'.solace'/(rid+'.responses')).read_text().splitlines()]
        self.assertIn('Moved priv/a to priv/moved/a',replies[0])
        self.assertIn('Already moved priv/a',replies[1])
        self.assertIn('already exists',replies[2])
        self.assertIn('Moved priv/d to docs/d (2 files)',replies[3])
        self.assertIn('file:priv/a (deleted)',replies[4]);self.assertIn('file:priv/moved/a (added)',replies[4])
        self.assertEqual(sorted(c['name'] for c in t['changes']),['file:docs/d/deep/two','file:docs/d/one','file:priv/a','file:priv/d/deep/two','file:priv/d/one','file:priv/moved/a'])
        self.action(t,'cancel')
        self.assertEqual((self.root/'priv/a').read_text(),'original');self.assertEqual((self.root/'priv/d/deep/two').read_text(),'2')
        self.assertFalse((self.root/'priv/moved/a').exists());self.assertFalse((self.root/'docs/d/one').exists())
        self.assertEqual((self.root/'priv/b').read_text(),'b')
    def test_an_investigation_cannot_move(self):  # D-84
        t=self.create([{'command':'move priv/a priv/b'},{'command':'task done looked'}])
        self.action(t,'plan');self.action(t,'investigate',model='fake')
        t=self.wait(t,lambda x:x['runs'] and x['runs'][-1]['state']=='ended')
        self.assertIn('E7010',(self.root/'.solace'/(t['runs'][-1]['id']+'.responses')).read_text())
        self.assertEqual((self.root/'priv/a').read_text(),'original');self.assertFalse((self.root/'priv/b').exists())
    def test_a_crash_after_a_move_keeps_its_undo(self):  # D-84
        self.restart({'SOLACE_FAIL_AFTER':'command'})
        t=self.assign([{'command':'move priv/a priv/b'}])
        self.wait(t,lambda x:x['runs'][-1]['state']=='ended')
        self.cli('read','priv/b')
        self.assertEqual(self.rows(t['id']),['file:priv/a','file:priv/b'])
        self.action(t,'cancel')
        self.assertEqual((self.root/'priv/a').read_text(),'original');self.assertFalse((self.root/'priv/b').exists())

    def test_a11_embedded_client_and_internal_files(self):
        for path,expected in [('/',b'Solace tasks'),('/workspace',b'Solace tasks'),('/migrate',b'Solace tasks'),('/strategy',b'Solace tasks'),('/strategy?tab=tasks',b'Solace tasks'),('/design',b'Solace tasks'),('/design?tab=ui-ux',b'Solace tasks'),('/settings',b'Solace tasks'),('/knowledgebase',b'Solace tasks'),('/app.js',b'/workflow'),('/app.js',b'.design{font:')]:
            c=http.client.HTTPConnection('127.0.0.1',self.port);c.request('GET',path);r=c.getresponse();self.assertEqual(r.status,200);self.assertIn(expected,r.read());c.close()
        # The guide's old address still answers, sending the browser on to the Knowledgebase page (D-140).
        c=http.client.HTTPConnection('127.0.0.1',self.port);c.request('GET','/guide');r=c.getresponse();self.assertEqual(r.status,301);self.assertEqual(r.getheader('Location'),'/knowledgebase');r.read();c.close()
        for command in ['files','read','write','remove']:
            p=subprocess.run([BIN,command,'.solace/board'],cwd=self.root,env=self.env,input='x',text=True,capture_output=True)
            self.assertIn('E6006',p.stdout)

    def test_models_availability_and_attention(self):
        models={m['id']:m for m in self.api('GET','/models')}
        self.assertTrue(models['fake']['available']);self.assertIsNone(models['fake']['reason'])
        self.assertFalse(models['missing']['available']);self.assertIn('not installed',models['missing']['reason'])
        self.assertIn('codex-gpt-6-astra',models)  # built-in presets are listed too
        self.assertEqual(models['native']['model'],'fixture',"the list names each model's own model")
        self.assertIsNone(models['fake'].get('model'),"a run argv names no model")
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

    def test_a_model_the_fake_codex_reports_is_offered(self):
        # The fake Codex answers `model/list`: its model shows in /models with
        # no config change, with the efforts Codex names, and its hidden model stays out.
        end=time.monotonic()+15;models={}
        while time.monotonic()<end:
            models={m['id']:m for m in self.api('GET','/models')}
            if 'codex-gpt-6-fake' in models:break
            time.sleep(.2)
        self.assertIn('codex-gpt-6-fake',models,json.dumps(sorted(models)))
        self.assertEqual(models['codex-gpt-6-fake']['label'],'GPT-6 Fake')
        self.assertEqual(models['codex-gpt-6-fake']['efforts'],['low','medium','high'])
        self.assertTrue(models['codex-gpt-6-fake']['available'],json.dumps(models['codex-gpt-6-fake']))
        self.assertNotIn('codex-gpt-6-fake-hidden',models,'a hidden model is left out')
        self.assertNotIn('gpt-6-fake',(self.root/'system.toml').read_text(),'no config change asked for the model')
        # Claude Code has no list command: the pick lists one entry per family —
        # the alias names the latest model, so a shipped exact preset gives way to it.
        self.assertNotIn('claude-opus-5-5',models,"the alias names the latest model")
        self.assertIn('claude-opus',models)
        self.assertEqual(models['claude-opus']['label'],'Opus (latest)')
        self.assertNotIn('claude-fable-5-1',models,"the alias names the latest model")
        self.assertIn('claude-fable',models)
        self.assertNotIn('claude-haiku-5-5',models,"the alias names the latest model")
        self.assertIn('claude-haiku',models)
        self.assertNotIn('claude-haiku-4-5',models,"the stale exact preset is gone")

    def test_a_run_shows_the_model_it_used(self):
        t=self.create([{'command':'task done done'}])
        self.action(t,'plan')
        t=self.action(t,'assign',model='native-claude')
        t=self.wait(t,lambda x:x['column']=='needs_review')
        self.assertEqual(t['runs'][-1]['used'],'claude-opus-5-5')
        models={m['id']:m for m in self.api('GET','/models')}
        self.assertEqual(models['native-claude']['label'],'Fixture',"a model with its own name keeps its label")
        self.assertEqual(models['native-claude']['model'],'fixture',"the list names the model it runs")

    def test_a10_events_reconnect(self):
        stream=self.stream()
        while stream.readline()!=b'event: ready\n':pass  # after the replay, once
        self.assertEqual(stream.readline(),b'data: {}\n')
        # D-78: a new stream starts from the project's workflow and tasks, then follows the changes.
        stream=self.stream();seq,e=self.next_event(stream);self.assertIn('workflow',e)
        seq,e=self.next_event(stream);self.assertIsInstance(e['tasks'],list)
        # D-87: the root's records follow the tasks, before the changes.
        _,e=self.next_event(stream);self.assertEqual(e['milestones'],[])
        _,e=self.next_event(stream);self.assertEqual(e['features'],[])
        _,e=self.next_event(stream);self.assertEqual(e['vision']['text'],'')
        _,e=self.next_event(stream);self.assertEqual(e,{'personas':[]})
        _,e=self.next_event(stream);self.assertIn('roleModels',e)
        _,e=self.next_event(stream);self.assertEqual(e,{'strategyRuns':[]})
        _,e=self.next_event(stream);self.assertEqual(e,{'designRuns':[]})
        _,e=self.next_event(stream);self.assertEqual(e,{'designs':[]})
        # D-129: the tasks with no project follow the runs, marked.
        _,e=self.next_event(stream);self.assertEqual(e,{'noProject':True,'tasks':[]})
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

    def test_a_task_whose_home_is_no_project_is_rehomed_on_load(self):  # D-160
        # The load re-homes a task whose home is no project: to the first project of
        # its scope, keeping only the projects of it, and to the root when none is
        # one. The move is noted, and the scope edits save like any other task's.
        (self.root/'sub/.solace').mkdir(parents=True)
        (self.root/'sub/.solace/keep').write_text('')
        (self.root/'solace.toml').write_text('unwatched=["priv/gen/"]\nprojects=["sub"]\n')
        a=self.create();pa=self.root/'.solace/tasks'/(a['id']+'.json')
        v=json.loads(pa.read_text());v['home']='gone';v['scope']=['gone'];pa.write_text(json.dumps(v))
        listed=next(x for x in self.api('GET','/tasks') if x['id']==a['id'])
        self.assertEqual((listed['home'],listed['scope']),('.',['.']),'re-homed to the root')
        self.assertNotIn('noProject',listed,'and unmarked')
        self.assertTrue(any('Moved from `gone` to `.`' in n['text'] for n in self.get(a)['notes']),'the move is noted')
        self.assertEqual(self.api('POST','/tasks/'+a['id']+'/scope',{'scope':['sub']})['home'],'sub','its scope saves')
        # A scope with a project re-homes there, keeping only the projects of the scope.
        b=self.create();pb=self.root/'.solace/tasks'/(b['id']+'.json')
        v=json.loads(pb.read_text());v['home']='gone';v['scope']=['gone','sub'];pb.write_text(json.dumps(v))
        stream=self.stream()
        while True:
            _,e=self.next_event(stream)
            if 'tasks' in e and any(x['id']==b['id'] for x in e['tasks']):
                x=next(x for x in e['tasks'] if x['id']==b['id'])
                self.assertEqual((x['home'],x['scope']),('sub',['sub']),'re-homed to the first project of its scope, keeping only the projects')
                break
        saved=json.loads(pb.read_text())
        self.assertEqual(saved['scope'],['sub'],'the saved scope keeps only the projects')
        self.assertTrue(any('Moved from `gone` to `sub`' in n['text'] and 'Dropped `gone`' in n['text'] for n in saved['notes']),'the note names the folder it dropped')

    def test_a_saved_noProject_mark_streams_unmarked(self):  # D-160
        # The mark comes from the home on each load: a saved one streams nothing.
        t=self.create();path=self.root/'.solace/tasks'/(t['id']+'.json')
        v=json.loads(path.read_text());v['noProject']=True;path.write_text(json.dumps(v))
        stream=self.stream()
        while True:
            _,e=self.next_event(stream)
            if 'tasks' in e and 'noProject' not in e:
                self.assertNotIn('noProject',next(x for x in e['tasks'] if x['id']==t['id']),'the task streams unmarked')
            if e.get('noProject') is True:
                self.assertEqual(e['tasks'],[],'and the no-project bucket stays empty')
                break

    def test_delete_note(self):
        t=self.create()
        for text in ['a','b','c']:self.api('POST','/tasks/'+t['id']+'/notes',{'text':text})
        t=self.api('DELETE','/tasks/'+t['id']+'/notes/1');self.assertEqual([n['text'] for n in t['notes']],['a','c'])
        self.api('DELETE','/tasks/'+t['id']+'/notes/2',code='E7001')

    def test_inbox_is_applied_after_a_restart(self):
        # D-77: a message waits in the inbox while no server runs, and is applied when one starts.
        t=self.create()
        self.stop()
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
        self.assertTrue(wait_for(prompt,8),'the discussion agent never started')
        self.stop()
        responses=self.root/'.solace'/(rid+'.responses')
        self.assertTrue(wait_for(responses,4),'the note call never returned')
        exited=self.root/'.solace'/(rid+'.exited')
        self.assertTrue(wait_for(exited,4),'the discussion agent never exited')
        self.assertIn('reaches the board',responses.read_text())
        inbox=self.root/'.solace/tasks/inbox'
        self.assertEqual(len(list(inbox.iterdir())),1)
        self.serve()
        t=self.wait(t,lambda x:len(x['notes'])==2)
        self.assertEqual([n['text'] for n in t['notes']],['What about X?','reply'])
        self.assertEqual(list(inbox.iterdir()),[])

    def test_a_queued_review_note_finishes_after_the_server_restarts(self):
        # A queued note finishes the review when the server returns, rather than
        # landing as feedback on an ended run.
        note = '**Assessment: Needs work**\n\n### Findings\n\nLate feedback.'
        t = self.completed([])
        self.api('PATCH', '/tasks/'+t['id'], {'body':json.dumps([{'sleep':.5},{'command':'task note','text':note}])})
        t = self.action(t, 'review', model='fake')
        rid = t['runs'][-1]['id']
        prompt = self.root/'.solace'/(rid+'.prompt')
        self.assertTrue(wait_for(prompt, 8), 'the review agent never started')
        self.stop()
        responses = self.root/'.solace'/(rid+'.responses')
        self.assertTrue(wait_for(responses, 4), 'the note call never returned')
        exited = self.root/'.solace'/(rid+'.exited')
        self.assertTrue(wait_for(exited, 4), 'the review agent never exited')
        self.assertIn('reaches the board', responses.read_text())
        self.serve()
        t = self.wait(t, lambda x: x['runs'][-1]['outcome']=='reviewed'
                     and len([n for n in x['notes'] if n['author']=='agent' and n['run']==rid])==1)
        self.assertEqual([n['text'] for n in t['notes'] if n['author']=='agent' and n['run']==rid], [note])
        self.assertEqual(t['finished']['outcome'], 'reviewed')

    def test_a_queued_review_note_after_its_assessment_adds_no_second_note(self):
        # The assessment already finished the review: a queued note that arrives
        # afterward is dropped instead of becoming a second agent note.
        assessment = '**Assessment: Looks good**\n\nFine.'
        note = '**Assessment: Needs work**\n\n### Findings\n\nLate feedback.'
        t = self.completed([])
        self.api('PATCH', '/tasks/'+t['id'], {'body':json.dumps([
            {'sleep':.5}, {'command':'task note','text':note}, {'say':assessment}])})
        t = self.action(t, 'review', model='fake')
        rid = t['runs'][-1]['id']
        prompt = self.root/'.solace'/(rid+'.prompt')
        self.assertTrue(wait_for(prompt, 8), 'the review agent never started')
        self.stop()
        responses = self.root/'.solace'/(rid+'.responses')
        self.assertTrue(wait_for(responses, 4), 'the note call never returned')
        exited = self.root/'.solace'/(rid+'.exited')
        self.assertTrue(wait_for(exited, 4), 'the review agent never exited')
        self.assertIn('reaches the board', responses.read_text())
        self.serve()
        t = self.wait(t, lambda x: x['runs'][-1]['outcome']=='reviewed'
                     and len([n for n in x['notes'] if n['author']=='agent' and n['run']==rid])==1)
        self.assertEqual([n['text'] for n in t['notes'] if n['author']=='agent' and n['run']==rid], [note])
        self.assertEqual(t['finished']['outcome'], 'reviewed')
        self.assertEqual(t['runs'][-1]['outcome'], 'reviewed')

    def test_a_stopped_server_queues_an_ask(self):
        # The same for a question: the agent is told it is queued, not left waiting.
        t=self.assign([{'sleep':.5},{'command':'task ask','text':'Which?','options':['a','b']}])
        rid=t['runs'][-1]['id']
        prompt=self.root/'.solace'/(rid+'.prompt')
        self.assertTrue(wait_for(prompt,8),'the work agent never started')
        self.stop()
        responses=self.root/'.solace'/(rid+'.responses')
        self.assertTrue(wait_for(responses,4),'the ask call never returned')
        exited=self.root/'.solace'/(rid+'.exited')
        self.assertTrue(wait_for(exited,4),'the work agent never exited')
        self.assertIn('reaches the board',responses.read_text())
        self.serve()
        # The restart ended the run before the question was applied: the question
        # is kept as a note, and does not wake the dead run or ask anyone.
        t=self.wait(t,lambda x:x['notes'])
        self.assertEqual(t['notes'][0]['text'],'Which?')
        self.assertEqual(t['questions'],[])
        self.assertEqual(t['runs'][-1]['state'],'ended')
        self.assertEqual(list((self.root/'.solace/tasks/inbox').iterdir()),[])

    def test_task_links_survive_completion_and_a_feature_delete(self):  # D-87
        m=self.api('POST','/milestones',{'title':'1.0'})
        f=self.api('POST','/features',{'title':'F','milestone':m['id']})
        t=self.completed([])
        self.assertIsInstance(t['createdTime'],int)  # D-130
        linked=self.api('PATCH','/tasks/'+t['id'],{'feature':f['id']})
        self.assertEqual((linked['feature'],linked['milestone'],linked['dependsOn'],linked['closedTime']),(f['id'],m['id'],[],None))
        self.action(t,'approve');done=self.action(t,'complete')
        self.assertIsInstance(done['closedTime'],int)
        self.api('DELETE','/features/'+f['id'])
        t=self.api('GET','/tasks/'+t['id'])
        self.assertEqual((t['feature'],t['milestone'],t['closedTime'],t['archivedTime'],t['createdTime']),(None,m['id'],done['closedTime'],None,linked['createdTime']))
        # D-95: an archived task leaves the board but stays in the task list.
        archived=self.api('POST','/tasks/'+t['id']+'/archive',{})
        self.assertIsInstance(archived['archivedTime'],int)
        listed=[x for x in self.api('GET','/tasks') if x['id']==t['id']]
        self.assertEqual(listed[0]['archivedTime'],archived['archivedTime'])

    def test_a_canceled_task_goes_back_to_planning(self):  # D-108, D-133
        w=self.api('GET','/workflow')
        self.assertEqual(w['actions']['to_planning']['label'],'Back to planning')
        t=self.create();dep=self.create()
        self.api('PATCH','/tasks/'+dep['id'],{'dependsOn':[t['id']]})
        self.api('POST','/tasks/'+t['id']+'/notes',{'text':'Keep me'})
        self.assertEqual(self.action(t,'to_planning','E7001')['allowed'],['plan','cancel','pause','resume','discuss','stop','end_discussion'])
        gone=self.action(t,'cancel');self.assertEqual(gone['column'],'cancelled');self.assertIsInstance(gone['closedTime'],int)
        self.assertIsInstance(self.api('POST','/tasks/'+t['id']+'/archive',{})['archivedTime'],int)
        back=self.action(t,'to_planning')
        self.assertEqual((back['column'],back['archivedTime'],back['closedTime'],back['title']),('planning',None,gone['closedTime'],'Fixture'))
        self.assertEqual(back['notes'][0]['text'],'Keep me')
        self.assertEqual((back['notes'][-1]['author'],back['notes'][-1]['text']),('solace','The human brought this task back from Canceled to Planning.'))
        self.assertEqual([x['archivedTime'] for x in self.api('GET','/tasks') if x['id']==t['id']],[None])
        self.assertEqual(self.get(dep)['dependsOn'],[t['id']])
        # A complete task stays as it is.
        done=self.completed([]);self.action(done,'approve');self.action(done,'complete')
        self.assertEqual(self.action(done,'to_planning','E7001')['allowed'],[])

    def test_back_to_planning_waits_for_the_run_and_keeps_changes(self):  # D-133
        working=self.assign([{'sleep':3}])
        self.assertIn('stop',self.action(working,'to_planning','E7001')['message'])
        self.action(working,'stop')
        self.assertEqual(self.action(working,'to_planning')['column'],'planning')
        # From Review: the changes stay and the approval goes, as Reassign does.
        t=self.completed([{'command':'read priv/a'},{'command':'write priv/a','text':'keep this'}])
        before=self.get(t)['changes'];self.assertTrue(before)
        self.action(t,'approve')
        back=self.action(t,'to_planning')
        self.assertEqual((back['column'],back['stamps']),('planning',{}))
        self.assertEqual(self.get(t)['changes'],before)

    def test_a_task_link_finds_its_task(self):  # D-98
        t=self.completed([])
        self.assertEqual(self.api('GET','/tasks/find/'+t['id']),{'id':t['id'],'dir':'.','archived':False,'title':t['title'],'project':True,'onBoard':True})
        self.action(t,'approve');self.action(t,'complete')
        self.api('POST','/tasks/'+t['id']+'/archive',{})
        found=self.api('GET','/tasks/find/'+t['id'])
        self.assertEqual((found['archived'],found['onBoard']),(True,False))
        status,_,body=self.raw('GET','/tasks/find/0123456789abcdef')
        self.assertEqual((status,json.loads(body)['code']),(404,'E7023'))
        # The link's own address loads the shell, which resolves it.
        status,kind,_=self.raw('GET','/task/'+t['id'])
        self.assertEqual((status,kind),(200,'text/html'))

    def test_a_feature_or_milestone_link_finds_its_title(self):  # D-122
        m=self.api('POST','/milestones',{'title':'1.0'})
        f=self.api('POST','/features',{'title':'F','milestone':m['id']})
        for kind,r in (('milestones',m),('features',f)):
            self.assertEqual(self.api('GET','/%s/find/%s'%(kind,r['id'])),{'id':r['id'],'title':r['title']})
            status,_,body=self.raw('GET','/%s/find/0123456789abcdef'%kind)
            self.assertEqual((status,json.loads(body)['code']),(404,'E7023'))
            status,_,body=self.raw('GET','/%s/find/bad.id'%kind)
            self.assertEqual((status,json.loads(body)['code']),(400,'E7001'))
        # The links' own addresses load the shell, which resolves them.
        for page in ('/feature/'+f['id'],'/milestone/'+m['id']):
            status,kind,_=self.raw('GET',page)
            self.assertEqual((status,kind),(200,'text/html'))

    def test_a_design_link_finds_its_title_and_tab(self):  # D-234
        (self.root/'designs').mkdir(exist_ok=True)
        for design_id,tab in (('0123456789abcdef','ui'),('fedcba9876543210','architecture')):
            design={'schemaVersion':1,'id':design_id,'title':'Home '+tab,'tab':tab,
                    'elements':[{'id':'root','type':'screen','properties':{'width':'fill','height':'fill'},'children':[]}]}
            (self.root/'designs'/(design_id+'.json')).write_text(json.dumps(design))
        self.assertEqual(self.api('GET','/designs/find/0123456789abcdef'),{'id':'0123456789abcdef','title':'Home ui','tab':'ui-ux'})
        self.assertEqual(self.api('GET','/designs/find/fedcba9876543210'),{'id':'fedcba9876543210','title':'Home architecture','tab':'architecture'})
        status,_,body=self.raw('GET','/designs/find/1111111111111111')
        self.assertEqual((status,json.loads(body)['code']),(404,'E7023'))
        status,_,body=self.raw('GET','/designs/find/bad.id')
        self.assertEqual((status,json.loads(body)['code']),(400,'E7001'))
        # The link's own address loads the shell, which resolves it.
        status,kind,_=self.raw('GET','/design/0123456789abcdef')
        self.assertEqual((status,kind),(200,'text/html'))

    def test_milestones_and_features_are_root_records(self):  # D-87
        stream=self.stream()
        _,e=self.next_event(stream);self.assertIn('workflow',e)
        _,e=self.next_event(stream);self.assertIn('tasks',e)
        seq,e=self.next_event(stream);self.assertEqual(e['milestones'],[])
        _,e=self.next_event(stream);self.assertEqual(e['features'],[])
        _,e=self.next_event(stream);self.assertIn('vision',e)
        _,e=self.next_event(stream);self.assertEqual(e,{'personas':[]})
        _,e=self.next_event(stream);self.assertIn('roleModels',e)
        _,e=self.next_event(stream);self.assertEqual(e,{'strategyRuns':[]})
        _,e=self.next_event(stream);self.assertEqual(e,{'designRuns':[]})
        _,e=self.next_event(stream);self.assertEqual(e,{'designs':[]})
        # D-129: the tasks with no project follow the runs, marked.
        _,e=self.next_event(stream);self.assertEqual(e,{'noProject':True,'tasks':[]})
        m=self.api('POST','/milestones',{'title':'1.0','targetDate':'2027-01-01'})
        f=self.api('POST','/features',{'title':'F','milestone':m['id']})
        self.assertEqual((m['state'],m['targetDate'],f['state'],f['milestone']),('proposed','2027-01-01','proposed',m['id']))
        _,e=self.next_event(stream);self.assertEqual(e['milestone']['id'],m['id'])
        _,e=self.next_event(stream);self.assertEqual(e['feature']['id'],f['id'])
        # An unknown milestone in a body is E7023 with 400; per-project routes are checked in workspace_acceptance.
        status,v=self.bare('POST','/features',{'Content-Type':'application/json'},json.dumps({'title':'G','milestone':'nope'}))
        self.assertEqual((status,json.loads(v)['code']),(400,'E7023'))
        # After a restart the records are still there, replayed after the tasks, and a
        # Last-Event-ID from the snapshot resumes the root log: the restart's own
        # workflow event leads, from the project's store, before the log's records.
        self.restart({})
        self.assertEqual([x['title'] for x in self.api('GET','/milestones')],['1.0'])
        self.assertEqual([x['title'] for x in self.api('GET','/features')],['F'])
        resumed=self.stream(seq)
        _,e=self.next_event(resumed);self.assertIn('workflow',e)
        _,e=self.next_event(resumed);self.assertEqual(e['milestone']['id'],m['id'])
        _,e=self.next_event(resumed);self.assertEqual(e['feature']['id'],f['id'])
        fresh=self.stream()
        _,e=self.next_event(fresh);self.assertIn('workflow',e)
        _,e=self.next_event(fresh);self.assertIn('tasks',e)
        _,e=self.next_event(fresh);self.assertEqual([x['id'] for x in e['milestones']],[m['id']])
        _,e=self.next_event(fresh);self.assertEqual([x['id'] for x in e['features']],[f['id']])

    def test_the_vision_is_one_root_document(self):  # D-91
        stream=self.stream()
        while 'vision' not in (e:=self.next_event(stream)[1]):pass
        self.assertEqual(e['vision']['text'],'')
        _,personas=self.next_event(stream);self.assertEqual(personas,{'personas':[]})
        _,role_models=self.next_event(stream);self.assertIn('roleModels',role_models)
        _,runs=self.next_event(stream);self.assertEqual(runs,{'strategyRuns':[]})
        _,design_runs=self.next_event(stream);self.assertEqual(design_runs,{'designRuns':[]})
        _,designs=self.next_event(stream);self.assertEqual(designs,{'designs':[]})
        # D-129: the tasks with no project follow the runs, marked.
        _,hp=self.next_event(stream);self.assertEqual(hp,{'noProject':True,'tasks':[]})
        v=self.api('PUT','/vision',{'text':'# Why\nFor teams.','base':e['vision']['revision']})
        self.assertEqual(self.api('GET','/vision'),v)
        _,e=self.next_event(stream);self.assertEqual(e,{'vision':v})
        # A put from an older revision is refused and writes nothing.
        status,body=self.bare('PUT','/vision',{'Content-Type':'application/json'},json.dumps({'text':'lost','base':'sha256:old'}))
        self.assertEqual((status,json.loads(body)['code']),(409,'E7026'))
        # D-92: at most 1000 characters, counted as Unicode code points.
        status,body=self.bare('PUT','/vision',{'Content-Type':'application/json'},json.dumps({'text':'é'*1001,'base':v['revision']}))
        self.assertEqual((status,json.loads(body)['code']),(400,'E7027'))
        self.assertEqual(self.api('GET','/vision')['text'],'# Why\nFor teams.')
        fresh=self.stream()
        while 'vision' not in (e:=self.next_event(fresh)[1]):pass
        self.assertEqual(e,{'vision':v})

    def test_personas_are_one_root_list(self):  # D-216
        stream=self.stream()
        while 'personas' not in (e:=self.next_event(stream)[1]):pass
        self.assertEqual(e,{'personas':[]})
        self.assertIn('roleModels',self.next_event(stream)[1])
        self.assertEqual(self.next_event(stream)[1],{'strategyRuns':[]})
        self.assertEqual(self.next_event(stream)[1],{'designRuns':[]})
        self.assertEqual(self.next_event(stream)[1],{'designs':[]})
        self.assertEqual(self.next_event(stream)[1],{'noProject':True,'tasks':[]})
        p=self.api('POST','/personas',{'name':'  Ada  ','objectives':'Ship','tools':'Rust','concerns':'Risk','other':'Notes'})
        self.assertEqual((p['name'],p['objectives'],p['tools'],p['concerns'],p['other']),('Ada','Ship','Rust','Risk','Notes'))
        self.assertTrue(p['revision'].startswith('sha256:'))
        self.assertEqual(self.api('GET','/personas'),[p])
        _,e=self.next_event(stream);self.assertEqual(e,{'personas':[p]})
        body=dict(p,objectives='Ship well');del body['revision'];body['base']=p['revision']
        q=self.api('PUT','/personas/'+p['id'],body)
        self.assertNotEqual(q['revision'],p['revision'])
        _,e=self.next_event(stream);self.assertEqual(e,{'personas':[q]})
        status,body=self.bare('PUT','/personas/'+p['id'],{'Content-Type':'application/json'},json.dumps({'name':'Lost','base':p['revision']}))
        self.assertEqual((status,json.loads(body)['code']),(409,'E7028'))
        self.assertEqual(self.api('GET','/personas'),[q])
        status,body=self.bare('POST','/personas',{'Content-Type':'application/json'},json.dumps({'name':' '}))
        self.assertEqual((status,json.loads(body)['code']),(400,'E7001'))
        status,body=self.bare('POST','/personas',{'Content-Type':'application/json'},json.dumps({'name':'Long','other':'é'*1000}))
        self.assertEqual((status,json.loads(body)['code']),(400,'E7029'))
        self.assertEqual(self.api('GET','/personas'),[q])
        gone=self.api('DELETE','/personas/'+p['id'])
        self.assertEqual(gone['id'],p['id'])
        _,e=self.next_event(stream);self.assertEqual(e,{'personas':[]})
        self.assertEqual(self.api('GET','/personas'),[])
        status,body=self.bare('PUT','/personas/'+p['id'],{'Content-Type':'application/json'},json.dumps({'name':'Back','base':q['revision']}))
        self.assertEqual((status,json.loads(body)['code']),(404,'E7023'))
        fresh=self.stream()
        while 'personas' not in (e:=self.next_event(fresh)[1]):pass
        self.assertEqual(e,{'personas':[]})

    def wait_run(self,rid,predicate,timeout=10):
        end=time.monotonic()+timeout
        while time.monotonic()<end:
            value=self.api('GET',f'/strategy/runs/{rid}')
            if predicate(value):return value
            time.sleep(.03)
        self.fail('timed out: '+json.dumps(value))
    def wait_design_run(self,rid,predicate,timeout=10):
        end=time.monotonic()+timeout
        while time.monotonic()<end:
            value=self.api('GET',f'/design/runs/{rid}')
            if predicate(value):return value
            time.sleep(.03)
        self.fail('timed out: '+json.dumps(value))
    def replies(self,rid):
        p=self.root/'.solace'/(rid+'.responses')
        return [json.loads(l)['result']['content'][0]['text'] for l in p.read_text().splitlines()] if p.exists() else []

    def test_strategy_runs_start_refuse_note_and_end(self):  # D-93, A19
        stream=self.stream()
        while 'vision' not in (e:=self.next_event(stream)[1]):pass
        _,e=self.next_event(stream);self.assertEqual(e,{'personas':[]})
        _,e=self.next_event(stream);self.assertIn('roleModels',e)
        _,e=self.next_event(stream);self.assertEqual(e,{'strategyRuns':[]})
        m=self.api('POST','/milestones',{'title':'M1'})
        f=self.api('POST','/features',{'title':'Feature F1','milestone':m['id']})
        t1,t2=self.create(),self.create()
        self.api('PATCH','/tasks/'+t1['id'],{'feature':f['id']})
        self.api('PATCH','/tasks/'+t2['id'],{'feature':f['id']})
        # A finished work run gives the agent a run log to read.
        t3=self.completed([]);r3=t3['runs'][-1]['id'];t3id=t3['id']
        on_f={'kind':'feature','id':f['id']}
        # The fake agent's steps are the run's first message.
        long='A long note. '+'y'*200+'\nSecond line.';n=len(t3['notes'])+1
        self.api('POST','/tasks/'+t3id+'/notes',{'text':long})
        steps=[{'command':'strategy list'},{'command':'strategy list --archived'},{'command':f"strategy show feature {f['id']}"},{'command':f"strategy show milestone {m['id']}"},
               {'command':'submit app.one','text':'pub fn one() { 9 }'},{'command':'write priv/a','text':'changed'},{'command':'task done','text':'no'},
               {'command':'read ../x'},{'command':'grep -i plan'},
               {'command':f'strategy show run {t3id} {r3}'},{'command':f'strategy show run {t3id} {r3} 9999'},{'command':f'strategy show run {t3id} nope'},{'command':f'strategy show task {t3id}'},{'command':f"strategy show task {t1['id']} {t2['id']}"},
               {'command':f'strategy show note {t3id} {n}'},{'command':f'strategy show note {t3id} 99'},{'command':'strategy show note nope 1'},
               {'command':'strategy note','text':'A first plan'},{'command':'strategy note','text':'A second plan'}]
        run=self.api('POST','/strategy/runs',{'model':'fake','text':json.dumps(steps),'context':on_f})
        self.assertEqual((run['state'],run['context']),('running',on_f))
        while 'strategyRun' not in (e:=self.next_event(stream)[1]):pass
        self.assertEqual(e,{'strategyRun':run});self.assertNotIn('project',e)
        # A second run on the same feature is refused while the first is open.
        status,body=self.bare('POST','/strategy/runs',{'Content-Type':'application/json'},json.dumps({'model':'fake','text':'Again','context':on_f}))
        self.assertEqual((status,json.loads(body)['code']),(409,'E7003'))
        rid=run['id']
        # The agent reads the graph by following links, can't change code, and its note waits.
        waiting=self.wait_run(rid,lambda r:r['state']=='waiting')
        self.assertEqual([(n['author'],n['text']) for n in waiting['notes'][1:]],[('agent','A first plan')])
        out=self.replies(rid)
        listed=out[0]
        self.assertTrue(all(x in listed for x in ('Tasks:',t1['id'],t2['id'],t3id,'Features:',f['id'],'Milestones:',m['id'],'Vision:')),listed)
        self.assertIn(t3id,out[1])
        shown=json.loads(out[2])
        self.assertEqual((shown['record']['title'],shown['links']['milestone'],sorted(shown['links']['tasks'])),('Feature F1',m['id'],sorted([t1['id'],t2['id']])))
        self.assertEqual(json.loads(out[3])['links'],{'features':[f['id']],'tasks':[t1['id'],t2['id']]})
        for r in out[4:7]:self.assertTrue(r.startswith('E7010'),r)
        # Outside a strategy run, strategy commands are refused.
        p=subprocess.run([BIN,'strategy','list'],cwd=self.root,env=self.env,text=True,capture_output=True)
        self.assertTrue(p.stdout.startswith('E7001'),p.stdout)
        # Refusals give no task-run advice; grep explains its syntax.
        self.assertIn('E6006',out[7]);self.assertIn('--project <folder>',out[7]);self.assertNotIn('task ask',out[7])
        self.assertIn('literal text, already case-insensitive',out[8])
        # A task's run log: its steps in outline, an unknown step or run refused, and the run listed on the task.
        outline=json.loads(out[9]);self.assertEqual((outline['run']['id'],'log' in outline['run'],type(outline['steps'])),(r3,False,list))
        self.assertTrue(out[10].startswith('E7001'),out[10]);self.assertTrue(out[11].startswith('E7023'),out[11])
        # D-101: the task is compact, its notes one line each, and `strategy show note` gives them in full.
        shown=json.loads(out[12])
        self.assertEqual(shown['links']['runs'],[r3])
        line=shown['record']['notes'][-1]
        self.assertTrue(line.startswith(f'{n} human ') and line.split(': ',1)[1]==long.splitlines()[0][:100]+'…',line)
        self.assertNotIn('conversation',shown['record'])
        multi=json.loads(out[13])
        self.assertEqual([r['record']['id'] for r in multi],[t1['id'],t2['id']])
        self.assertEqual((multi[0]['links']['feature'],multi[1]['links']['milestone']),(f['id'],m['id']))
        full=json.loads(out[14])
        self.assertEqual((full['task'],full['notes'][0]['note'],full['notes'][0]['text']),(t3id,n,long))
        self.assertTrue(out[15].startswith('E7001'),out[15]);self.assertTrue(out[16].startswith('E7023'),out[16])
        self.assertEqual(len(out),17)
        # The tool a strategy run's agent sees names only what it can do.
        env={**self.env,'SOLACE_TASK':'strategy','SOLACE_RUN':rid}
        p=subprocess.run([BIN,'mcp'],cwd=self.root,env=env,text=True,capture_output=True,input=json.dumps({'jsonrpc':'2.0','id':1,'method':'tools/list'})+'\n')
        tool=json.dumps(json.loads(p.stdout)['result']['tools'][0])
        self.assertIn('strategy note',tool)
        for word in ['task ask','submit','`test`','session start','work add']:self.assertNotIn(word,tool)
        self.assertFalse((self.root/'.solace'/(rid+'.exited')).exists())
        self.assertEqual(((self.root/'priv/a').read_text(),(self.root/'src/app.gleam').read_text().splitlines()[0]),('original','pub fn one() { 1 }'))
        prompt=(self.root/'.solace'/(rid+'.prompt')).read_text()
        self.assertIn('Feature F1',prompt);self.assertIn('Reply only with `strategy note`',prompt)
        for word in ['task ask','Start with `session start','`submit`','are refused']:self.assertNotIn(word,prompt)
        # The human's next note reaches the waiting agent, which replies again.
        self.api('POST',f'/strategy/runs/{rid}/notes',{'text':'And tasks'})
        self.wait_run(rid,lambda r:r['state']=='waiting' and r['notes'][-1]['text']=='A second plan')
        self.assertEqual(self.replies(rid)[17],'The human replied:\n\nAnd tasks\n')
        ended=self.api('POST',f'/strategy/runs/{rid}/end')
        self.assertEqual((ended['state'],ended['outcome'],len(ended['notes']),ended['notes'][0]['author']),('ended','planned',1,'solace'))
        self.assertIn('A second plan',ended['notes'][0]['text'])
        # The agent hears that the run ended and exits on its own.
        deadline=time.monotonic()+10
        while not (self.root/'.solace'/(rid+'.exited')).exists() and time.monotonic()<deadline:time.sleep(.05)
        self.assertTrue((self.root/'.solace'/(rid+'.exited')).exists())
        self.assertIn('has ended',self.replies(rid)[18])
        while 'strategyRun' not in (e:=self.next_event(stream)[1]) or e['strategyRun']['state']!='ended':pass
        self.assertEqual(e,{'strategyRun':ended})
        status,body=self.bare('POST',f'/strategy/runs/{rid}/notes',{'Content-Type':'application/json'},json.dumps({'text':'late'}))
        self.assertEqual((status,json.loads(body)['code']),(409,'E7003'))
        listed=self.api('GET','/strategy/runs')
        self.assertEqual(([r['id'] for r in listed],'notes' in listed[0]),([rid],False))
        self.assertEqual(self.api('GET',f'/strategy/runs/{rid}')['notes'],ended['notes'])
        # A new connection replays the runs after the vision and personas.
        fresh=self.stream()
        while 'vision' not in (e:=self.next_event(fresh)[1]):pass
        _,e=self.next_event(fresh);self.assertEqual(e,{'personas':[]})
        _,e=self.next_event(fresh);self.assertIn('roleModels',e)
        _,e=self.next_event(fresh);self.assertEqual(e,{'strategyRuns':listed})

    def test_design_runs_read_reply_and_refuse_changes(self):  # D-96, D-240
        (self.root/'designs').mkdir(exist_ok=True)
        design_id='0123456789abcdef'
        design={'schemaVersion':1,'id':design_id,'title':'Home','tab':'architecture','elements':[]}
        (self.root/'designs'/(design_id+'.json')).write_text(json.dumps(design))
        (self.root/'DECISIONS.md').write_text('# Decisions\n## D-1 First thing\n')
        (self.root/'plain/.solace').mkdir(parents=True)
        sqlite3.connect(self.root/'plain/.solace/store.db').close()
        (self.root/'solace.toml').write_text('unwatched=["priv/gen/"]\nprojects=["plain"]\n')
        t=self.create()
        steps=[{'command':'strategy list'},{'command':f"strategy show task {t['id']}"},
               {'command':'view cli.HOST'},{'command':'read DECISIONS.md'},
               {'command':'design list'},{'command':f'design show {design_id}'},
               {'command':'read ../outside'},
               {'command':'view app.one --project . --draft 1'},
               {'command':'view app.one --project plain'},
               {'command':'submit app.one','text':'pub fn one() { 9 }'},
               {'command':f"strategy task {t['id']}",'text':json.dumps({'title':'Changed'})},
               {'command':'task note hello'},
               {'command':'design note','text':'A calm design'}]
        rid=self.api('POST','/design/runs',{'model':'fake','text':json.dumps(steps)})['id']
        run=self.wait_design_run(rid,lambda r:r['state']=='waiting')
        self.assertEqual([(n['author'],n['text']) for n in run['notes'][1:]],[('agent','A calm design')])
        out=self.replies(rid)
        self.assertIn(t['id'],out[0]);self.assertIn('Vision:',out[0])
        self.assertEqual(json.loads(out[1])['record']['id'],t['id'])
        self.assertIn('cli',out[2])
        self.assertIn('Decisions',out[3])
        self.assertIn(design_id,out[4])
        self.assertIn(design_id,out[5]);self.assertIn('Home',out[5])
        self.assertTrue(out[6].startswith('E6006'),out[6])
        self.assertTrue(out[7].startswith('E6007'),out[7])
        self.assertTrue(out[8].startswith('E6013'),out[8]);self.assertIn('files, grep and read',out[8])
        for r in out[6:9]:
            self.assertNotIn('task ask',r);self.assertNotIn('write and run',r);self.assertNotIn('in a board run',r)
        for r in out[9:12]:
            self.assertTrue(r.startswith('E7010'),r)
            self.assertNotIn('task ask',r);self.assertNotIn('strategy wait',r)
        # The tool a design run's agent sees names only what it can do.
        env={**self.env,'SOLACE_TASK':'design','SOLACE_RUN':rid}
        p=subprocess.run([BIN,'mcp'],cwd=self.root,env=env,text=True,capture_output=True,input=json.dumps({'jsonrpc':'2.0','id':1,'method':'tools/list'})+'\n')
        tool=json.dumps(json.loads(p.stdout)['result']['tools'][0])
        self.assertIn('design note',tool);self.assertIn('design propose',tool);self.assertIn('strategy list',tool)
        for word in ['task ask','submit','`test`','session start','work add','strategy note','strategy task']:
            self.assertNotIn(word,tool)
        prompt=(self.root/'.solace'/(rid+'.prompt')).read_text()
        self.assertIn('Reply with `design note`',prompt)
        self.assertIn('design propose',prompt)
        role=prompt.split('\n\nNotes so far:')[0]
        for word in ['submit','`test`','session start','task ask','strategy note']:
            self.assertNotIn(word,role)
        # The human's next note reaches the waiting agent, which replies again.
        self.api('POST',f'/design/runs/{rid}/notes',{'text':'Make it calm'})
        deadline=time.monotonic()+10
        while time.monotonic()<deadline and 'The human replied:\n\nMake it calm\n' not in self.replies(rid):time.sleep(.05)
        self.assertIn('The human replied:\n\nMake it calm\n',self.replies(rid))
        self.api('POST',f'/design/runs/{rid}/end')

    def test_a_design_proposal_is_accepted_and_listed(self):  # D-243
        # The human asks for a Home screen; the fake agent proposes it and waits.
        operations=[{'op':'add','index':0,'element':{'id':'root','type':'screen','properties':{'width':'fill','height':'fill'}}}]
        steps=[{'command':'design propose','text':json.dumps({'title':'Home','tab':'ui','operations':operations})}]
        rid=self.api('POST','/design/runs',{'model':'fake','text':json.dumps(steps)})['id']
        run=self.wait_design_run(rid,lambda r:r['state']=='waiting' and len(r['questions'])==1)
        q=run['questions'][0]
        design_id=q['proposal']['design']['id']
        self.assertEqual((q['options'],q['proposal']['design']['title'],q['proposal']['design']['elements'][0]['id']),(['Approve','Reject'],'Home','root'))
        self.assertEqual(self.api('GET','/designs')['designs'],[])
        answered=self.api('POST',f'/design/runs/{rid}/questions/{q["id"]}',{'option':'Approve'})
        self.assertEqual(answered['questions'][0]['answer'],'Approved and saved version 1: created design:'+design_id)
        self.assertEqual([(d['id'],d['title'],d['tab']) for d in self.api('GET','/designs')['designs']],[(design_id,'Home','ui')])
        self.assertEqual(self.api('GET','/designs/'+design_id)['elements'][0]['id'],'root')
        versions=self.api('GET',f'/designs/{design_id}/versions')
        self.assertEqual([(v['number'],v['source']) for v in versions],[(1,{'kind':'run','id':rid})])
        deadline=time.monotonic()+10
        while time.monotonic()<deadline and not self.replies(rid):time.sleep(.05)
        self.assertIn('Approved and saved version 1',self.replies(rid)[0])
        self.api('POST',f'/design/runs/{rid}/end')

    def test_strategy_proposals_wait_for_approval(self):  # D-93, A19
        f=self.api('POST','/features',{'title':'Done F','state':'complete'})
        steps=[{'command':'strategy note\nstrategy vision','text':'not alone'},
               {'command':'strategy task','text':json.dumps({'title':'Rejected one'})},
               {'command':'strategy task','text':json.dumps({'title':'Approved one','feature':f['id']})},
               {'command':'strategy vision','text':'Agent vision'},
               {'command':'strategy note','text':'Done proposing'}]
        rid=self.api('POST','/strategy/runs',{'model':'fake','text':json.dumps(steps),'context':{'kind':'page'}})['id']
        def asked(n):return self.wait_run(rid,lambda r:len(r['questions'])==n and r['state']=='waiting')['questions'][-1]
        # Reject creates nothing.
        q=asked(1)
        self.assertEqual((q['options'],q['proposal']['kind'],q['answer']),(['Approve','Approve to Planning','Reject'],'task',None))
        # D-118: a question carries when it was asked and how many notes came before it.
        self.assertIsInstance(q['time'],int);self.assertIsInstance(q['after'],int)
        first=q
        self.api('POST',f'/strategy/runs/{rid}/questions/{q["id"]}',{'option':'Reject'})
        # A task linked to a complete feature reopens it only on Approve.
        q=asked(2)
        self.assertGreater(q['after'],first['after']);self.assertGreaterEqual(q['time'],first['time'])
        self.assertNotIn('Rejected one',[t['title'] for t in self.api('GET','/tasks')])
        self.assertEqual(q['proposal']['reopens'],[{'kind':'feature','id':f['id']}]);self.assertIn('back to upcoming',q['text'])
        self.assertEqual(self.api('GET','/features')[0]['state'],'complete')
        self.api('POST',f'/strategy/runs/{rid}/questions/{q["id"]}',{'option':'Approve'})
        self.assertEqual(self.api('GET','/features')[0]['state'],'upcoming')
        t=next(t for t in self.api('GET','/tasks') if t['title']=='Approved one')
        self.assertEqual(t['feature'],f['id'])
        self.assertEqual(self.api('GET','/tasks/'+t['id'])['notes'][-1]['text'],f'Created by strategy run {rid}')
        # A vision proposal approved after a human edit is not applied.
        q=asked(3)
        self.assertEqual(q['proposal']['base'],self.api('GET','/vision')['revision'])
        self.api('PUT','/vision',{'text':'Human vision','base':q['proposal']['base']})
        answered=self.api('POST',f'/strategy/runs/{rid}/questions/{q["id"]}',{'option':'Approve'})
        self.assertTrue(answered['questions'][2]['answer'].startswith('Not applied: E7026'),answered['questions'][2])
        self.assertEqual(self.api('GET','/vision')['text'],'Human vision')
        run=self.wait_run(rid,lambda r:r['state']=='waiting' and r['notes'][-1]['text']=='Done proposing')
        out=self.replies(rid)
        self.assertTrue(out[0].startswith('E7017'),out[0])
        self.assertEqual(out[1],'The human answered the proposal Create task "Rejected one": Reject\n')
        self.assertIn(f'Approved and applied: created task:{t["id"]}',out[2]);self.assertIn('Not applied: E7026',out[3])
        # Every answer is on the run's notes, and so in its transcript once it ends.
        answers=[n['text'] for n in run['notes'] if n['author']=='solace']
        self.assertEqual(len(answers),3)
        for a,want in zip(answers,['Answer: Reject','Answer: Approved and applied','Answer: Not applied: E7026']):self.assertIn(want,a)
        ended=self.api('POST',f'/strategy/runs/{rid}/end')
        self.assertEqual((ended['outcome'],len(ended['notes'])),('planned',1));self.assertIn('Answer: Not applied: E7026',ended['notes'][0]['text'])

    def test_strategy_mid_work_notes_arrive_once_in_order(self):
        steps=[{'sleep':.4},{'command':'strategy note','text':'first plan'},
               {'sleep':.4},{'command':'strategy note','text':'second plan'},
               {'sleep':.4},{'command':'strategy note','text':'third plan'}]
        rid=self.api('POST','/strategy/runs',{'model':'fake','text':json.dumps(steps),'context':{'kind':'page'}})['id']
        self.wait_run(rid,lambda r:(self.root/'.solace'/(rid+'.prompt')).exists())
        for text in ['mid one','mid two']:self.api('POST',f'/strategy/runs/{rid}/notes',{'text':text})
        self.wait_run(rid,lambda r:r['state']=='waiting' and len(r['notes'])>=4 and len(self.replies(rid))>=1)
        self.assertEqual(self.replies(rid)[0],'The human replied:\n\nmid one\n\nmid two\n')
        self.api('POST',f'/strategy/runs/{rid}/notes',{'text':'mid three'})
        self.wait_run(rid,lambda r:r['state']=='waiting' and len(r['notes'])>=7 and len(self.replies(rid))>=2)
        self.assertEqual(self.replies(rid)[1],'The human replied:\n\nmid three\n')
        self.api('POST',f'/strategy/runs/{rid}/end')

    def test_strategy_note_survives_a_plain_reply(self):
        steps=[{'sleep':.6},{'say':'plain plan'}]
        rid=self.api('POST','/strategy/runs',{'model':'fake','text':json.dumps(steps),'context':{'kind':'page'}})['id']
        self.wait_run(rid,lambda r:(self.root/'.solace'/(rid+'.prompt')).exists())
        self.api('POST',f'/strategy/runs/{rid}/notes',{'text':'mid note'})
        self.wait_run(rid,lambda r:r['state']=='waiting' and len(r['notes'])==4)
        self.assertIn('mid note',(self.root/'.solace'/(rid+'.prompt')).read_text())
        time.sleep(.4)
        self.assertEqual(len(self.api('GET',f'/strategy/runs/{rid}')['notes']),4)
        self.api('POST',f'/strategy/runs/{rid}/end')

    def test_strategy_note_during_a_proposal_and_late_answer(self):
        steps=[{'command':'strategy task','text':json.dumps({'title':'Proposed'})},
               {'command':'strategy note','text':'heard note'},
               {'command':'strategy note','text':'heard answer'}]
        rid=self.api('POST','/strategy/runs',{'model':'fake','text':json.dumps(steps),'context':{'kind':'page'}})['id']
        q=self.wait_run(rid,lambda r:r['state']=='waiting' and len(r['questions'])==1)['questions'][0]
        for text in ['change title','also add scope']:self.api('POST',f'/strategy/runs/{rid}/notes',{'text':text})
        self.wait_run(rid,lambda r:r['state']=='waiting' and len(r['notes'])==4 and len(self.replies(rid))==1)
        self.assertEqual(self.replies(rid)[0],'The human replied:\n\nchange title\n\nalso add scope\n')
        self.api('POST',f'/strategy/runs/{rid}/questions/{q["id"]}',{'option':'Approve'})
        run=self.wait_run(rid,lambda r:r['state']=='waiting' and len(r['notes'])==6 and len(self.replies(rid))==2)
        self.assertIn('The human answered the proposal Create task "Proposed": Approved and applied',self.replies(rid)[1])
        time.sleep(.4)
        self.assertEqual(len(self.replies(rid)),2)  # The late answer is delivered once.
        self.api('POST',f'/strategy/runs/{rid}/end')

    def test_a_proposal_answered_while_the_agent_works_reaches_its_next_note(self):
        self.restart({'SOLACE_WAIT_LIMIT_SECS':'1'})
        steps=[{'command':'strategy task','text':json.dumps({'title':'While working'})},
               {'sleep':1.5},{'command':'strategy note','text':'heard while working'},
               {'command':'strategy wait'},{'say':'after'}]
        rid=self.api('POST','/strategy/runs',{'model':'fake','text':json.dumps(steps),'context':{'kind':'page'}})['id']
        run=self.wait_run(rid,lambda r:r['state']=='waiting' and r['questions'] and len(self.replies(rid))>=1)
        self.assertIn('`strategy wait` (a strategy run)',self.replies(rid)[0])
        self.api('POST',f'/strategy/runs/{rid}/questions/{run["questions"][0]["id"]}',{'option':'Reject'})
        self.wait_run(rid,lambda r:len(self.replies(rid))>=3)
        self.assertEqual(self.replies(rid)[1],'The human answered the proposal Create task "While working": Reject\n')
        self.assertEqual(self.replies(rid)[2],'Still waiting for the human. Call `strategy wait` (a strategy run) or `task wait` (a discussion) to keep waiting; it sends nothing.\n')
        self.api('POST',f'/strategy/runs/{rid}/end')

    def test_a_proposal_answered_after_a_plain_reply_reaches_the_fresh_brief(self):
        self.restart({'SOLACE_WAIT_LIMIT_SECS':'1'})
        steps=[{'command':'strategy task','text':json.dumps({'title':'After exit'})},{'say':'plain plan'}]
        rid=self.api('POST','/strategy/runs',{'model':'fake','text':json.dumps(steps),'context':{'kind':'page'}})['id']
        run=self.wait_run(rid,lambda r:r['state']=='waiting' and r['questions'] and len(self.replies(rid))>=1)
        self.assertIn('`strategy wait` (a strategy run)',self.replies(rid)[0])
        self.api('POST',f'/strategy/runs/{rid}/questions/{run["questions"][0]["id"]}',{'option':'Reject'})
        prompt=self.root/'.solace'/(rid+'.prompt')
        end=time.monotonic()+10
        while 'Answer: Reject' not in prompt.read_text() and time.monotonic()<end:time.sleep(.03)
        text=prompt.read_text()
        self.assertIn('Answer: Reject',text)
        self.assertIn('Create task "After exit"',text)
        self.wait_run(rid,lambda r:len(self.replies(rid))>=2)
        self.assertIn('`strategy wait` (a strategy run)',self.replies(rid)[1])
        self.assertFalse(any('The human answered the proposal' in r for r in self.replies(rid)),self.replies(rid))
        self.api('POST',f'/strategy/runs/{rid}/end')

    def test_answering_an_older_proposal_wakes_the_agent(self):
        steps=[{'command':'strategy task','text':json.dumps({'title':'First proposal'})},
               {'command':'strategy task','text':json.dumps({'title':'Second proposal'})},
               {'command':'strategy note','text':'heard the first answer'},
               {'command':'strategy note','text':'heard the second answer'}]
        rid=self.api('POST','/strategy/runs',{'model':'fake','text':json.dumps(steps),'context':{'kind':'page'}})['id']
        first=self.wait_run(rid,lambda r:r['state']=='waiting' and len(r['questions'])==1)['questions'][0]
        self.api('POST',f'/strategy/runs/{rid}/notes',{'text':'change it'})
        second=self.wait_run(rid,lambda r:r['state']=='waiting' and len(r['questions'])==2)['questions'][-1]
        self.assertIn('The human replied:\n\nchange it',self.replies(rid)[0])
        # Approving the older proposal reaches the agent while a newer one waits.
        self.api('POST',f'/strategy/runs/{rid}/questions/{first["id"]}',{'option':'Approve'})
        self.wait_run(rid,lambda r:r['state']=='waiting' and len(self.replies(rid))>=2)
        self.assertIn('The human answered the proposal Create task "First proposal": Approved and applied',self.replies(rid)[1])
        self.assertNotIn('Second proposal',self.replies(rid)[1])
        self.api('POST',f'/strategy/runs/{rid}/questions/{second["id"]}',{'option':'Approve'})
        self.wait_run(rid,lambda r:r['state']=='waiting' and len(self.replies(rid))>=3)
        self.assertIn('The human answered the proposal Create task "Second proposal": Approved and applied',self.replies(rid)[2])
        self.assertEqual(len([r for r in self.replies(rid) if 'First proposal' in r]),1)
        self.api('POST',f'/strategy/runs/{rid}/end')

    def test_a_strategy_proposal_scope_names_projects(self):  # D-129
        # A scope folder that isn't a project is refused when the proposal is
        # sent: the agent gets the error naming the folder and the projects,
        # the human is never asked, and nothing is written. A mixed scope is
        # refused too; a scope of projects, and no scope, are accepted.
        steps=[{'command':'strategy task','text':json.dumps({'title':'Lost','scope':['solace-cli']})},
               {'command':'strategy task','text':json.dumps({'title':'Half lost','scope':['.','solace-cli']})},
               {'command':'strategy task','text':json.dumps({'title':'Kept','scope':['.']})},
               {'command':'strategy note','text':'Done proposing'}]
        rid=self.api('POST','/strategy/runs',{'model':'fake','text':json.dumps(steps),'context':{'kind':'page'}})['id']
        def asked(n):return self.wait_run(rid,lambda r:len(r['questions'])==n and r['state']=='waiting')['questions'][-1]
        q=asked(1)
        self.assertEqual((q['proposal']['kind'],q['proposal']['fields']['title']),('task','Kept'))
        self.api('POST',f'/strategy/runs/{rid}/questions/{q["id"]}',{'option':'Approve'})
        self.wait_run(rid,lambda r:r['state']=='waiting' and r['notes'][-1]['text']=='Done proposing')
        out=self.replies(rid)
        self.assertEqual(len([o for o in out[:2] if o.startswith('E7001') and 'solace-cli is not a project' in o and 'scope takes project folders: .' in o]),2,out)
        self.assertIn('Approved and applied',out[2])
        titles=[t['title'] for t in self.api('GET','/tasks')]
        self.assertEqual(('Lost' in titles,'Half lost' in titles,'Kept' in titles),(False,False,True))
        self.assertFalse((self.root/'solace-cli').exists(),'no store at the refused folder')
        self.api('POST',f'/strategy/runs/{rid}/end')

    def test_a_usage_limit_makes_its_sign_ins_models_unavailable(self):
        # A model on Codex's own sign-in (the fake codex, signed in by its key variable).
        with (self.root/'system.toml').open('a') as f:
            f.write('\n[[models]]\nid="signed-in"\nlabel="Signed in"\nagent="codex"\nmodel="fixture"\n')
        self.restart({'OPENAI_API_KEY':'test-key'})
        models={m['id']:m for m in self.api('GET','/models')}
        self.assertTrue(models['signed-in']['available'],models['signed-in'])
        # Codex's own words (src/board/usage-limits.json), as its `exec --json` error event.
        said='You’ve hit your usage limit. To get more access now, send a request to your admin or try again later.'
        t=self.create([{'exit':1,'error':said}]);self.action(t,'plan')
        t=self.action(t,'assign',model='signed-in')
        self.wait(t,lambda x:(x['attention'] or {}).get('hold'))
        def listed():
            end=time.monotonic()+5
            while True:
                models={m['id']:m for m in self.api('GET','/models')}
                if not models['signed-in']['available'] or time.monotonic()>end:return models
                time.sleep(.05)
        models=listed()
        # Every model on the sign-in is unavailable, with the reason; the list stays complete.
        for id in ['signed-in','codex-gpt-6-sol']:
            self.assertEqual((models[id]['available'],models[id]['reason'],models[id]['retry']),(False,'Codex usage limit reached',True),id)
        self.assertIn('codex-gpt-6-astra',models)
        # A model on its own endpoint or program isn't on the sign-in.
        for id in ['native','fake']:self.assertTrue(models[id]['available'],models[id])
        again=self.create();self.action(again,'plan')
        e=self.action(again,'assign','E7011',model='signed-in');self.assertIn('usage limit',e['message'])
        # The mark outlasts a restart.
        self.restart()
        self.assertFalse({m['id']:m for m in self.api('GET','/models')}['signed-in']['available'])
        # Try again clears it; the reply is the list as it is now.
        models={m['id']:m for m in self.api('POST','/models/try-again',{'agent':'codex'})}
        self.assertTrue(models['signed-in']['available'],models['signed-in'])
        self.assertEqual(self.api('POST','/models/try-again',{'agent':'nobody'},'E7001')['code'],'E7001')

    def test_the_agents_own_sign_in_status_decides_and_refresh_rechecks(self):
        # Codex's status command decides: OPENAI_API_KEY alone is not a sign-in.
        status=self.root/'status';status.write_text('out')
        with (self.root/'system.toml').open('a') as f:
            f.write('\n[[models]]\nid="signed-in"\nlabel="Signed in"\nagent="codex"\nmodel="fixture"\n')
            f.write('\n[[models]]\nid="gateway"\nlabel="Gateway"\nagent="claude"\nmodel="x"\nendpoint={kind="anthropic",base_url="http://localhost:1"}\n')
        self.restart({'SOLACE_FAKE_STATUS':str(status)})
        models={m['id']:m for m in self.api('GET','/models')}
        self.assertEqual((models['signed-in']['available'],models['signed-in']['reason']),(False,'codex is not signed in'))
        # An endpoint model needs its own key, not Claude's sign-in.
        self.assertTrue(models['gateway']['available'],models['gateway'])
        # Signing in shows after the refresh; signing out again shows too.
        status.write_text('in')
        models={m['id']:m for m in self.api('POST','/models/try-again',{})}
        self.assertTrue(models['signed-in']['available'],models['signed-in'])
        status.write_text('out')
        models={m['id']:m for m in self.api('POST','/models/try-again',{})}
        self.assertFalse(models['signed-in']['available'])
        # Claude Code refusing an endpoint model holds that model alone, with
        # its reason; Try again and a later successful run each clear it.
        status.write_text('in')
        models={m['id']:m for m in self.api('POST','/models/try-again',{})}
        self.assertTrue(models['signed-in']['available'],models['signed-in'])
        def gateway(available):
            end=time.monotonic()+5
            while True:
                models={m['id']:m for m in self.api('GET','/models')}
                if models['gateway']['available']==available or time.monotonic()>end:return models
                time.sleep(.05)
        def refuse():
            t=self.create([{'exit':1,'error':'There is an issue with the selected model (x): model not found'}])
            self.action(t,'plan');self.action(t,'assign',model='gateway')
        refuse()
        models=gateway(False)
        self.assertEqual((models['gateway']['available'],models['gateway']['retry']),(False,True),models['gateway'])
        self.assertIn('selected model',models['gateway']['reason'])
        self.assertTrue(models['signed-in']['available'],models['signed-in'])
        models={m['id']:m for m in self.api('POST','/models/try-again',{})}
        self.assertTrue(models['gateway']['available'],models['gateway'])

    def test_a_claude_key_overrides_a_signed_out_status(self):
        # Claude Code's key is a sign-in even when the program answers signed out.
        with (self.root/'system.toml').open('a') as f:
            f.write('\n[[models]]\nid="claude-key"\nlabel="Claude key"\nagent="claude"\nmodel="fixture"\n')
        status=self.root/'status';status.write_text('out')
        # A key makes discovery ask Anthropic for its models; keep that call offline.
        curl=self.root/'fake-bin/curl';curl.write_text('#!/bin/sh\nexit 1\n');curl.chmod(0o755)
        self.restart({'SOLACE_FAKE_STATUS':str(status),'ANTHROPIC_API_KEY':'test-key'})
        models={m['id']:m for m in self.api('GET','/models')}
        self.assertTrue(models['claude-key']['available'],models['claude-key'])
        # Without the key, the signed-out answer holds.
        self.restart({'ANTHROPIC_API_KEY':''})
        models={m['id']:m for m in self.api('GET','/models')}
        self.assertEqual((models['claude-key']['available'],models['claude-key']['reason']),(False,'claude is not signed in'))

    def test_a_plain_discussion_reply_clears_a_limit_mark(self):
        # A mark whose reset has passed leaves the model available but stays until an answer.
        with (self.root/'system.toml').open('a') as f:
            f.write('\n[[models]]\nid="signed-in"\nlabel="Signed in"\nagent="codex"\nmodel="fixture"\n')
        self.stop()
        limits=self.root/'agent-holds.json'
        limits.write_text(json.dumps({'codex':{'reason':'Codex usage limit reached','until':1,'time':1}}))
        self.env['OPENAI_API_KEY']='test-key'
        self.serve()
        self.assertTrue({m['id']:m for m in self.api('GET','/models')}['signed-in']['available'])
        t=self.create([{'say':'plain reply'}]);self.action(t,'plan')
        t=self.action(t,'discuss',model='signed-in',text='What about X?')
        self.wait(t,lambda x:x['runs'][-1]['state']=='waiting' and len(x['notes'])==2)
        end=time.monotonic()+5
        while 'codex' in json.loads(limits.read_text()) and time.monotonic()<end:time.sleep(.05)
        self.assertNotIn('codex',json.loads(limits.read_text()))

    def test_a_plain_strategy_reply_clears_a_limit_mark(self):
        # A strategy agent that answers in plain text took its turn (`planned`), like a discussion.
        with (self.root/'system.toml').open('a') as f:
            f.write('\n[[models]]\nid="signed-in"\nlabel="Signed in"\nagent="codex"\nmodel="fixture"\n')
        self.stop()
        limits=self.root/'agent-holds.json'
        limits.write_text(json.dumps({'codex':{'reason':'Codex usage limit reached','until':1,'time':1}}))
        self.env['OPENAI_API_KEY']='test-key'
        self.serve()
        rid=self.api('POST','/strategy/runs',{'model':'signed-in','text':json.dumps([{'say':'plain reply'}]),'context':{'kind':'page'}})['id']
        end=time.monotonic()+10
        while self.api('GET',f'/strategy/runs/{rid}')['state']!='waiting' and time.monotonic()<end:time.sleep(.05)
        while 'codex' in json.loads(limits.read_text()) and time.monotonic()<end:time.sleep(.05)
        self.assertNotIn('codex',json.loads(limits.read_text()))

    def test_a_batched_review_note_is_recorded_before_the_agent_exits(self):
        # A batched note waits for the board, so the review is not failed by the exit.
        t = self.completed([])
        t, _, _ = self.review(t, [{'command':'task diff\ntask note','text':'**Assessment: Looks good** batched'}])
        self.assertEqual(t['runs'][-1]['outcome'], 'reviewed')
        notes = [n for n in t['notes'] if n['author'] == 'agent' and 'batched' in n['text']]
        self.assertEqual(len(notes), 1)
        self.assertEqual(t['notes'][-1]['text'], '**Assessment: Looks good** batched')

if __name__=='__main__':unittest.main()

