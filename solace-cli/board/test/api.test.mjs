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
    if (path === '/events') return new Response(new ReadableStream({ start(c) { events = c; c.enqueue(new TextEncoder().encode('event: ready\ndata: {}\n\n')); } }), { headers: { 'Content-Type': 'text/event-stream' } });
    if (req.method === 'GET') return Response.json(path === '/graph' ? { review: { summary: '1 effects added, 0 signature changed, 0 test unfinished' } } : path === '/workflow' ? workflow : path === '/models' ? [{ id: 'fake', label: 'Fake' }] : path === '/tasks' ? [task] : task);
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
  const confirmOriginal = globalThis.confirm;
  const confirmations = [];
  let acceptConfirmation = false;
  globalThis.confirm = question => { confirmations.push(question); return acceptConfirmation; };
  const controllers = [];
  globalThis.fetch = (path, options) => fetchOriginal(new URL(path, server.url), options);
  // EventSource is a browser primitive absent from Bun; consume real SSE here.
  globalThis.EventSource = class {
    listeners = {};
    addEventListener(name, f) { this.listeners[name] = f; }
    constructor(path) {
      const controller = new AbortController(); controllers.push(controller);
      fetch(path, { signal: controller.signal }).then(async response => {
        let text = '';
        for await (const chunk of response.body) {
          text += new TextDecoder().decode(chunk);
          while (text.includes('\n\n')) {
            const at = text.indexOf('\n\n'); const frame = text.slice(0, at); text = text.slice(at + 2);
            const data = frame.split('\n').find(line => line.startsWith('data: '));
            const name = frame.split('\n').find(line => line.startsWith('event: '));
            if (name) this.listeners[name.slice(7)]?.({ data: data?.slice(6) });
            else if (data) this.onmessage?.({ data: data.slice(6) });
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
    dispatch(new board.NewTask()); dispatch(new board.DraftTitle('New task')); dispatch(new board.DraftBody('Human description')); dispatch(new board.Create());
    await until(() => task.title === 'New task' && !model.creating && model.draft_title === '');
    dispatch(new board.Edit(true)); dispatch(new board.Title('Edited')); dispatch(new board.Save()); await until(() => task.title === 'Edited');
    dispatch(new board.Text('Human note')); dispatch(new board.AddNote());
    await until(() => to_string(board.view(model)).includes('Human note'));
    dispatch(new board.Action('plan')); await until(() => task.column === 'planning');
    expect(confirmations).toHaveLength(0);
    dispatch(new board.Text('Keep these instructions'));
    for (const action of ['cancel', 'complete']) {
      const before = requests.length;
      const beforeModel = model;
      dispatch(new board.Action(action));
      await Bun.sleep(30);
      expect(requests.length).toBe(before);
      expect(model).toBe(beforeModel);
      expect(confirmations.at(-1)).toContain(task.title);
    }
    expect(confirmations[0]).toContain('reverts its changes');
    expect(confirmations[1]).toContain('keeps its changes');
    acceptConfirmation = true;
    const beforeCancel = requests.filter(([method, path]) => method === 'POST' && path.endsWith('/actions')).length;
    dispatch(new board.Action('cancel'));
    await until(() => model.text === '');
    expect(requests.filter(([method, path]) => method === 'POST' && path.endsWith('/actions')).length).toBe(beforeCancel + 1);
    dispatch(new board.Action('complete')); await until(() => model.error === 'Approval required');
    task.questions = [{ id: 'q1', text: 'Choose', options: ['yes', 'no'] }];
    task.runs = [{ id: 'r1', kind: 'work', state: 'waiting', model: 'fake', log: '/logs/r1' }];
    events.enqueue(new TextEncoder().encode('id: 1\ndata: ' + JSON.stringify({ task }) + '\n\n'));
    dispatch(new board.Tab('questions')); await until(() => to_string(board.view(model)).includes('Choose'));
    expect(to_string(board.view(model))).toContain('<strong>Question on Edited</strong><p>Choose</p>');
    dispatch(new board.Answer('q1', 'yes')); await until(() => task.questions[0].answer === 'yes');
    workflow.columns.push({ id: 'planning', label: 'Custom planning', actions: ['review'], final: false });
    events.enqueue(new TextEncoder().encode('id: 2\ndata: ' + JSON.stringify({ workflow }) + '\n\n'));
    await until(() => to_string(board.view(model)).includes('Custom planning'));
    for (const pair of [['GET', '/workflow'], ['GET', '/tasks'], ['GET', '/tasks/t1'], ['GET', '/models'], ['GET', '/events'], ['POST', '/tasks'], ['PATCH', '/tasks/t1'], ['POST', '/tasks/t1/notes'], ['POST', '/tasks/t1/actions'], ['POST', '/tasks/t1/questions/q1']]) expect(requests).toContainEqual(pair);
    dispatch(new board.Tab('runs')); expect(to_string(board.view(model))).toContain('/logs/r1');
    task.changes = [{ name: 'app.main' }];
    dispatch(new board.Select('t1')); dispatch(new board.Tab('changes'));
    await until(() => to_string(board.view(model)).includes('1 effects added'));
    expect(to_string(board.view(model))).toContain('/view?task=t1');
    expect(requests).toContainEqual(['GET', '/graph']);
  } finally {
    if (confirmOriginal === undefined) delete globalThis.confirm;
    else globalThis.confirm = confirmOriginal;
    controllers.forEach(c => c.abort()); server.stop(true); globalThis.fetch = fetchOriginal; delete globalThis.EventSource;
  }
});

test('markdown renders only images uploaded to the board, and http(s) links', async () => {
  const { markdown } = await import('../src/api.mjs');
  const url = '/tasks/t-1/files/0123456789abcdef.png';
  expect(markdown(`![shot](${url})`)).toBe(`<p><img alt="shot" src="${url}"></p>`);
  for (const bad of ['![x](https://evil/x.png)', '![x](javascript:alert(1))', '![x](/tasks/t/files/../../solace.toml)'])
    expect(markdown(bad)).not.toContain('<img');
  expect(markdown('![x" onerror="alert(1)](' + url + ')')).not.toContain('" onerror="');
  expect(markdown('[docs](https://example.com/a?b=1&c=2)')).toBe('<p><a href="https://example.com/a?b=1&amp;c=2" rel="noopener noreferrer">docs</a></p>');
  expect(markdown('[x](javascript:alert(1))')).not.toContain('<a');
});

test('draft storage survives missing or throwing storage', async () => {
  const { load, store } = await import('../src/api.mjs');
  const original = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  try {
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('blocked'); } });
    expect(load('k')).toBe('');
    expect(() => store('k', 'v')).not.toThrow();
    const values = new Map();
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: { getItem: k => values.get(k) ?? null, setItem: (k, v) => values.set(k, v), removeItem: k => values.delete(k) } });
    store('k', 'draft'); expect(load('k')).toBe('draft');
    store('k', ''); expect(load('k')).toBe('');
  } finally {
    if (original) Object.defineProperty(globalThis, 'localStorage', original); else delete globalThis.localStorage;
  }
});
