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
        self.root = Path(self.tmp.name)
        (self.root/'src').mkdir()
        (self.root/'src/app.gleam').write_text('pub fn one() { 1 }\n\npub fn two() { 2 }\n\npub fn removed() { 3 }\n')
        (self.root/'gleam.toml').write_text('name="fixture"\nversion="1.0.0"\ntarget="javascript"\n')
        (self.root/'solace.toml').write_text('resources=["priv/**"]\n')
        (self.root/'priv').mkdir()
        (self.root/'priv/a').write_text('original')
        config=self.root/'system.toml'
        config.write_text('task_timeout=2\n[[models]]\nid="fake"\nlabel="Fake agent"\nagent="codex"\nrun=["python3",'+json.dumps(FAKE)+']\n[[models]]\nid="missing"\nlabel="Missing agent"\nagent="codex"\nrun=["solace-no-such-program"]\n')
        self.env={**os.environ,'SOLACE_CONFIG':str(config)}
        self.cli('import')
        self.server=subprocess.Popen([BIN,'serve','--port','0'],cwd=self.root,env=self.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        line=self.server.stdout.readline()
        self.assertIn('listening',line)
        self.port=int(line.strip().rsplit(':',1)[1])
        self.clients=[]
    def tearDown(self):
        # End all fake processes before terminating their supervisor.
        try:
            for t in self.api('GET','/tasks'):
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
    def wait(self,t,predicate):
        end=time.monotonic()+10
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
        seq=None
        while True:
            line=r.readline().decode().strip()
            if line.startswith('id:'):seq=int(line[3:])
            if line.startswith('data:'):return seq,json.loads(line[5:])
    def test_a1_default_action_matrix(self):
        w=self.api('GET','/workflow')
        # Seed column positions through the human-owned JSON fixture, then use only HTTP actions.
        for col in w['columns']:
            for name in [*w['actions'],'pause','resume']:
                t=self.create([{'sleep':.2},{'command':'task done done'}])
                t['column']=col['id']
                (self.root/'.solace/board'/f'{t["id"]}.json').write_text(json.dumps(t))
                allowed=[] if col['final'] else [*col['actions'],'pause','resume']
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
        self.assertEqual(self.action(t,'complete')['column'],'done')
        self.api('PATCH','/tasks/'+t['id'],{'title':'no'},'E7004')
        t=self.completed([]);self.action(t,'approve');t=self.action(t,'start_over');self.assertEqual(t['stamps'],{})
        self.wait(t,lambda x:x['column']=='needs_review');self.action(t,'approve');self.assertEqual(self.action(t,'cancel')['column'],'cancelled')
        crashed=self.assign([{'exit':7}]);crashed=self.wait(crashed,lambda x:x['column']=='blocked');self.assertIn('without finishing',crashed['notes'][-1]['text'])
        self.assertIn('without finishing',crashed['attention']['reason'])
        self.assertIsNone(self.action(crashed,'cancel')['attention'])
        timed=self.assign([{'sleep':3}]);timed=self.wait(timed,lambda x:x['column']=='blocked');self.assertIn('timeout',timed['notes'][-1]['text'])
    def test_a3_custom_workflow_and_invalid(self):
        original=(self.root/'solace.toml').read_text()
        default=Path(__file__).parents[1].joinpath('src/default-board.toml').read_text()
        custom=default.replace('actions = ["plan", "cancel"]','actions = ["triage", "plan", "cancel"]')+'\ntriage = {to="triage"}\n\n[[board.columns]]\nid="triage"\nlabel="Triage"\nactions=["plan"]\n'
        (self.root/'solace.toml').write_text(original+custom)
        t=self.create();self.assertEqual(self.action(t,'triage')['column'],'triage');self.assertEqual(self.action(t,'plan')['column'],'planning')
        for broken in [default.replace('to = "planning"','to = "missing"'),default+'\nunused={to="backlog"}\n',default.replace('asked = "blocked",',''),default.replace('next = "done"','next = "missing"'),default.replace('id = "done"','id = "done"\nactions=["plan"]')]:
            (self.root/'solace.toml').write_text(original+broken);self.api('GET','/workflow',code='E6020')
        (self.root/'solace.toml').write_text(original)
    def test_a5_question_wait_and_new_agent(self):
        t=self.assign([{'command':'task ask','text':'Which?','options':['a','b']},{'command':'task done answered'}])
        t=self.wait(t,lambda x:x['column']=='blocked');rid=t['runs'][-1]['id'];pid=int((self.root/'.solace'/(rid+'.pid')).read_text());os.kill(pid,0)
        self.assertFalse((self.root/'.solace'/(rid+'.responses')).exists())
        time.sleep(2.2)  # Waiting does not consume the two-second task timeout.
        self.assertEqual(self.get(t)['runs'][-1]['state'],'waiting')
        events=self.stream()
        while True:
            _,event=self.next_event(events)
            if event.get('task',{}).get('questions'):
                self.assertEqual(event['task']['questions'][0]['text'],'Which?');break
        other=self.create();self.action(other,'plan');self.action(other,'assign','E7005',model='fake')
        self.action(t,'review','E7003',model='fake')
        self.api('PATCH','/tasks/'+t['id'],{'title':'blocked edit'},'E7003')
        self.action(t,'unblock','E7002')
        self.api('POST',f'/tasks/{t["id"]}/questions/{t["questions"][0]["id"]}',{'option':'a'})
        self.action(t,'unblock');t=self.wait(t,lambda x:x['column']=='needs_review');self.assertEqual(t['runs'][-1]['id'],rid)
        response=json.loads((self.root/'.solace'/(rid+'.responses')).read_text().splitlines()[0]);self.assertEqual(json.loads(response['result']['content'][0]['text'])['questions'][0]['answer'],'a')
        t=self.assign([{'command':'task ask','text':'Again?','options':['a','b']},{'command':'task done answered'}]);t=self.wait(t,lambda x:x['column']=='blocked');old=t['runs'][-1]['id']
        t=self.action(t,'new_agent',text='b');new=t['runs'][-1]['id'];self.assertNotEqual(old,new)
        self.wait(t,lambda x:len(x['questions'])==2)
        prompt=(self.root/'.solace'/(new+'.prompt')).read_text();self.assertIn("Answer: b",prompt);self.assertNotIn('"runs"',prompt)
        self.action(t,'unblock',text='b');self.wait(t,lambda x:x['column']=='needs_review')
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
        t=self.create([*steps,{'command':'task ask','text':'Which?','options':['a','b']},{'command':'task done report'}]);self.action(t,'plan')
        self.action(t,'investigate',model='fake');t=self.wait(t,lambda x:x['runs'][-1]['state']=='waiting')
        self.assertEqual(t['column'],'planning');self.assertIn('question',t['attention']['reason']);rid=t['runs'][-1]['id']
        self.action(t,'investigate','E7002');self.action(t,'investigate',text='a')
        t=self.wait(t,lambda x:x['runs'][-1]['state']=='ended');self.assertEqual(t['runs'][-1]['id'],rid);self.assertEqual(t['runs'][-1]['outcome'],'reported')
        self.assertEqual(t['column'],'planning');self.assertEqual(t['notes'][-1]['text'],'report');self.assertFalse(t['changes'])
        prompt=(self.root/'.solace'/(rid+'.prompt')).read_text()
        self.assertIn('Investigate this task',prompt)
        self.assertIn('This run is read-only',prompt)
        self.assertEqual(before,self.cli('view','app.one'));self.assertFalse((self.root/'priv/new').exists())
        responses=(self.root/'.solace'/(rid+'.responses')).read_text();self.assertEqual(responses.count('E7010'),2)
        self.assertEqual(self.action(t,'complete')['column'],'done')  # an answered question ends after planning
        review=self.completed([]);self.api('PATCH','/tasks/'+review['id'],{'body':json.dumps([*steps,{'command':'task note review report'}])})
        self.action(review,'review',model='fake');review=self.wait(review,lambda x:x['runs'][-1]['state']=='ended');self.assertEqual(review['column'],'needs_review')
        self.assertEqual(review['notes'][-1]['text'],'review report')
    def test_declared_run_resources_revert(self):
        config=self.root/'solace.toml'
        config.write_text(config.read_text()+'\n[commands]\nedit={run=["python3","-c",'+json.dumps("from pathlib import Path; Path('priv/a').write_text('changed'); Path('priv/created').write_text('new')")+']}\n')
        t=self.completed([{'command':'run edit'}])
        self.assertEqual(len(t['changes']),2)
        self.assertEqual((self.root/'priv/a').read_text(),'changed')
        self.action(t,'cancel')
        self.assertEqual((self.root/'priv/a').read_text(),'original');self.assertFalse((self.root/'priv/created').exists())

    def test_a11_embedded_client_and_internal_files(self):
        for path,expected in [('/',b'Solace tasks'),('/app.js',b'/workflow'),('/guide',b'Using Solace')]:
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
        t=self.wait(t,lambda x:x['column']=='blocked')  # the fake agent has no steps and exits
        self.assertTrue(t['attention']['reason'])
        asked=self.assign([{'command':'task ask','text':'Which?','options':['a','b']}])
        asked=self.wait(asked,lambda x:x['column']=='blocked');self.assertIn('question',asked['attention']['reason'])
        self.assertIsNone(self.action(asked,'cancel')['attention'])

    def test_a10_events_reconnect(self):
        stream=self.stream();seq,e=self.next_event(stream);self.assertIn('workflow',e)
        t=self.create();seq,e=self.next_event(stream);self.assertEqual(e['task']['id'],t['id'])
        self.api('POST','/tasks/'+t['id']+'/notes',{'text':'first'});self.api('POST','/tasks/'+t['id']+'/notes',{'text':'second'})
        resumed=self.stream(seq);s1,e1=self.next_event(resumed);s2,e2=self.next_event(resumed)
        self.assertEqual(s2,s1+1);self.assertEqual(e1['task']['notes'][-1]['text'],'first');self.assertEqual(e2['task']['notes'][-1]['text'],'second')

if __name__=='__main__':unittest.main()
