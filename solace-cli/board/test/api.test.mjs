import { test, expect } from 'bun:test';
import * as board from '../build/dev/javascript/solace_board/solace_board.mjs';
import { perform } from '../build/dev/javascript/lustre/lustre/effect.mjs';
import { to_string } from '../build/dev/javascript/lustre/lustre/element.mjs';

// This server has no Solace store, process runner or filesystem access.
test('A11: Lustre board uses only the HTTP API, including live task and workflow updates', async () => {
  const requests = [];
  let task = { id: 't1', title: 'Initial', body: 'Description', column: 'backlog', rank: 1,
    type: 'implementation', paused_from: null, notes: [], questions: [], runs: [], changes: [], stamps: {}, links: [] };
  let workflow = { columns: [{ id: 'backlog', label: 'Backlog', actions: ['plan'], final: false }]};
  let events;
  const server = Bun.serve({ port: 0, fetch: async req => {
    const path = new URL(req.url).pathname;
    requests.push([req.method, path]);
    if (path === '/events') return new Response(new ReadableStream({ start(c) { events = c; } }), { headers: { 'Content-Type': 'text/event-stream' } });
    if (req.method === 'GET') return Response.json(path === '/workflow' ? workflow : path === '/models' ? [{ id: 'fake', label: 'Fake' }] : path === '/tasks' ? [task] : task);
    const body = await req.json();
    if (path.endsWith('/actions')) {
      if (body.action === 'complete') return Response.json({ code: 'E7006', message: 'Approval required' }, { status: 409 });
      task = { ...task, column: 'planning' };
    } else if (path.endsWith('/notes')) task.notes.push({ author: 'human', text: body.text });
    else if (path.includes('/questions/')) task.questions[0].answer = body.option || body.text;
    else task = { ...task, ...body };
    return Response.json(task);
  }});
  const fetchOriginal = globalThis.fetch;
  const controllers = [];
  globalThis.fetch = (path, options) => fetchOriginal(new URL(path, server.url), options);
  // EventSource is a browser primitive absent from Bun; consume real SSE here.
  globalThis.EventSource = class {
    constructor(path) {
      const controller = new AbortController(); controllers.push(controller);
      fetch(path, { signal: controller.signal }).then(async response => {
        let text = '';
        for await (const chunk of response.body) {
          text += new TextDecoder().decode(chunk);
          while (text.includes('\n\n')) {
            const at = text.indexOf('\n\n'); const frame = text.slice(0, at); text = text.slice(at + 2);
            const data = frame.split('\n').find(line => line.startsWith('data: '));
            if (data) this.onmessage?.({ data: data.slice(6) });
          }
        }
      }).catch(() => {});
    }
  };
  let model;
  const noop = () => {};
  const execute = effect => perform(effect, dispatch, noop, noop, noop, noop, noop, noop);
  const dispatch = msg => { let effect; [model, effect] = board.update(model, msg); execute(effect); };
  const until = async predicate => {
    for (let i = 0; i < 200; i++) { if (predicate()) return; await Bun.sleep(10); }
    throw new Error('UI did not reach expected state: ' + to_string(board.view(model)));
  };
  try {
    let effect; [model, effect] = board.init(); execute(effect);
    await until(() => model.model === 'fake' && to_string(board.view(model)).includes('Initial') && events);
    dispatch(new board.Select('t1'));
    await until(() => to_string(board.view(model)).includes('Questions'));
    dispatch(new board.Title('New task')); dispatch(new board.Body('Human description')); dispatch(new board.Create());
    await until(() => task.title === 'New task');
    dispatch(new board.Title('Edited')); dispatch(new board.Save()); await until(() => task.title === 'Edited');
    dispatch(new board.Text('Human note')); dispatch(new board.AddNote());
    await until(() => to_string(board.view(model)).includes('Human note'));
    dispatch(new board.Action('plan')); await until(() => task.column === 'planning');
    dispatch(new board.Action('complete')); await until(() => model.error === 'Approval required');
    task.questions = [{ id: 'q1', text: 'Choose', options: ['yes', 'no'] }];
    task.runs = [{ id: 'r1', kind: 'work', state: 'waiting', model: 'fake', log: '/logs/r1' }];
    events.enqueue(new TextEncoder().encode('id: 1\ndata: ' + JSON.stringify({ task }) + '\n\n'));
    await until(() => to_string(board.view(model)).includes('Choose'));
    dispatch(new board.Answer('q1', 'yes')); await until(() => task.questions[0].answer === 'yes');
    workflow.columns.push({ id: 'planning', label: 'Custom planning', actions: ['review'], final: false });
    events.enqueue(new TextEncoder().encode('id: 2\ndata: ' + JSON.stringify({ workflow }) + '\n\n'));
    await until(() => to_string(board.view(model)).includes('Custom planning'));
    for (const pair of [['GET', '/workflow'], ['GET', '/tasks'], ['GET', '/tasks/t1'], ['GET', '/models'], ['GET', '/events'], ['POST', '/tasks'], ['PATCH', '/tasks/t1'], ['POST', '/tasks/t1/notes'], ['POST', '/tasks/t1/actions'], ['POST', '/tasks/t1/questions/q1']]) expect(requests).toContainEqual(pair);
    expect(to_string(board.view(model))).toContain('/logs/r1');
  } finally {
    controllers.forEach(c => c.abort()); server.stop(true); globalThis.fetch = fetchOriginal; delete globalThis.EventSource;
  }
});
