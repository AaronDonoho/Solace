import { test, expect } from 'bun:test';
import * as board from '../build/dev/javascript/solace_board/solace_board.mjs';
import { perform } from '../build/dev/javascript/lustre/lustre/effect.mjs';
import { to_string } from '../build/dev/javascript/lustre/lustre/element.mjs';

// This server has no Solace store, process runner or filesystem access.
test('A11: Lustre board uses only the HTTP API, including live task and workflow updates', async () => {
  const requests = [];
  let task = { id: 't1', title: 'Initial', body: 'Description', column: 'planning', rank: 1,
    type: 'implementation', paused_from: null, notes: [], questions: [], runs: [], changes: [], stamps: {}, links: [] };
  let workflow = { columns: [{ id: 'planning', label: 'Planning', actions: ['assign'], final: false }]};
  let events;
  const server = Bun.serve({ port: 0, fetch: async req => {
    const path = new URL(req.url).pathname;
    requests.push([req.method, path]);
    if (path === '/events') return new Response(new ReadableStream({ start(c) { events = c; c.enqueue(new TextEncoder().encode('id: 0\ndata: ' + JSON.stringify({ workflow }) + '\n\nid: 0\ndata: ' + JSON.stringify({ tasks: [task] }) + '\n\nevent: ready\ndata: {}\n\n')); } }), { headers: { 'Content-Type': 'text/event-stream' } });
    if (path === '/projects') return Response.json({ workspace: false, projects: [{ dir: '', name: 'fixture', base: '' }], found: [] });
    if (req.method === 'GET') return Response.json(path === '/workflow' ? workflow : path === '/tasks' ? [task] : task);
    const body = await req.json();
    task = { ...task, ...body, id: 't2' };
    return Response.json(task);
  }});
  const fetchOriginal = globalThis.fetch;
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
    dispatch(new board.Connected());
    await until(() => to_string(board.view(model)).includes('Initial') && events);
    // A card opens the task's details, which load it themselves.
    dispatch(new board.Select('', 't1'));
    expect(to_string(board.view(model))).toContain('<solace-task bar base dir hosted task="t1">');
    // A live update of the open task goes to its details, and a run that ended raises a notice.
    task.runs = [{ id: 'r1', kind: 'work', state: 'running', model: 'fake', log: '/logs/r1' }];
    events.enqueue(new TextEncoder().encode('id: 1\ndata: ' + JSON.stringify({ task }) + '\n\n'));
    await until(() => Array.from(model.live).some(m => m[1].includes('"running"')));
    task.runs = [{ ...task.runs[0], state: 'ended', outcome: 'finished' }];
    events.enqueue(new TextEncoder().encode('id: 2\ndata: ' + JSON.stringify({ task }) + '\n\n'));
    await until(() => to_string(board.view(model)).includes('<strong>Finished Initial</strong>'));
    workflow.columns.push({ id: 'assigned', label: 'Custom assigned', actions: ['review'], final: false });
    events.enqueue(new TextEncoder().encode('id: 3\ndata: ' + JSON.stringify({ workflow }) + '\n\n'));
    await until(() => to_string(board.view(model)).includes('Custom assigned'));
    for (const pair of [['GET', '/projects'], ['GET', '/workflow'], ['GET', '/events']]) expect(requests).toContainEqual(pair);
    // Tasks are created on the Strategy page: the board posts none.
    expect(requests).not.toContainEqual(['POST', '/tasks']);
    // The board asks for nothing that belongs to one task's details.
    expect(requests.filter(([, path]) => path === '/models' || path.startsWith('/tasks/'))).toEqual([]);
  } finally {
    controllers.forEach(c => c.abort()); server.stop(true); globalThis.fetch = fetchOriginal; delete globalThis.EventSource;
  }
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

test('transport uses the base it is given; explicit project paths stay absolute', async () => {
  const oldLocation = globalThis.location, oldFetch = globalThis.fetch;
  const paths = [];
  globalThis.location = { pathname: '/p/web/contracts' };
  globalThis.fetch = async path => { paths.push(path); return new Response('{}'); };
  try {
    const api = await import('../src/api.mjs');
    await new Promise(resolve => api.request('/p/shop', '/graph', 'GET', '', resolve));
    await new Promise(resolve => api.request('/p/shop', '/p/other/tasks', 'GET', '', resolve));
    await new Promise(resolve => api.request('', '/graph', 'GET', '', resolve));
    expect(paths).toEqual(['/p/shop/graph', '/p/other/tasks', '/graph']);
  } finally {
    globalThis.location = oldLocation; globalThis.fetch = oldFetch;
  }
});

test('dragging is stopped once per root, wherever the board is nested', async () => {
  const api = await import('../src/api.mjs');
  const listeners = new Map();
  const root = { addEventListener: (name, fn) => listeners.set(name, (listeners.get(name) ?? 0) + 1) };
  api.prevent_drag(root);
  api.prevent_drag(root);
  expect([...listeners.entries()]).toEqual([['dragstart', 1]]);
});

test('text dragged within a text box or a markdown editor moves; anything else stays put', async () => {
  const api = await import('../src/api.mjs');
  let drag;
  api.prevent_drag({ addEventListener: (name, fn) => { drag = fn; } });
  const stopped = path => { let prevented = false; drag({ composedPath: () => path, preventDefault() { prevented = true; } }); return prevented; };
  const card = { closest: () => null, classList: { contains: () => false } };
  const box = { closest: s => (s === 'input,textarea' ? box : null) };
  // The selected text inside the editor's own shadow root: a text node, then CodeMirror's content, then the editor.
  const editor = { closest: () => null, classList: { contains: name => name === 'md-editor' } };
  expect(stopped([card])).toBe(true);
  expect(stopped([box])).toBe(false);
  expect(stopped([{}, card, {}, editor])).toBe(false);
});

// A browser without a DOM for the component tests: requests and event streams
// are recorded, and timers wait until the test fires them.
function fakeBrowser(search = '', replies = {}, pathname = '/') {
  const saved = Object.fromEntries(['fetch', 'EventSource', 'location', 'setTimeout'].map(k => [k, Object.getOwnPropertyDescriptor(globalThis, k)]));
  const paths = [], sources = [], timers = [];
  Object.defineProperty(globalThis, 'location', { configurable: true, value: { pathname, search } });
  globalThis.fetch = async path => {
    paths.push(path);
    return Response.json(path in replies ? replies[path] : {});
  };
  globalThis.setTimeout = callback => { timers.push(callback); return 0; };
  globalThis.EventSource = class {
    listeners = {};
    constructor(path) { this.path = path; this.readyState = 1; sources.push(this); }
    addEventListener(name, f) { this.listeners[name] = f; }
    close() { this.closed = true; this.readyState = 2; }
    // A connection the browser has given up on, not one it will retry.
    fail() { this.readyState = 2; this.onerror?.(); }
  };
  const restore = () => { for (const [k, d] of Object.entries(saved)) d ? Object.defineProperty(globalThis, k, d) : delete globalThis[k]; };
  return { paths, sources, timers, restore };
}
// One component instance, driven as Lustre drives it.
function mount(app) {
  const noop = () => {};
  let model;
  const dispatch = msg => { let effect; [model, effect] = app.update(model, msg); perform(effect, dispatch, noop, noop, noop, noop, noop, noop); };
  [model] = app.init();
  return { dispatch, get model() { return model; } };
}
const settle = () => Bun.sleep(20);

test('each instance uses its own base, including one set before it is added to the page', async () => {
  const browser = fakeBrowser('?task=t1');
  try {
    // createElement, then setAttribute('base'), then append: nothing is asked until it connects.
    const shop = mount(board);
    shop.dispatch(new board.BaseChanged('/p/shop'));
    expect(browser.paths).toEqual([]);
    expect(browser.sources).toEqual([]);
    shop.dispatch(new board.Connected());
    await settle();
    expect(browser.paths).toEqual(expect.arrayContaining(['/p/shop/projects']));
    expect(browser.sources.map(s => s.path)).toEqual(['/p/shop/events']);
    // Without a base, the page address decides.
    const plain = mount(board);
    plain.dispatch(new board.Connected());
    expect(browser.sources.at(-1).path).toBe('/events');
  } finally { browser.restore(); }
});

test('a component removed and added again reopens one stream, which receives later events', async () => {
  const browser = fakeBrowser('?task=t1');
  try {
    const b = mount(board);
    b.dispatch(new board.Connected());
    browser.sources[0].listeners.ready();
    expect(b.model.caught_up).toBe(true);
    b.dispatch(new board.Disconnected());
    expect(browser.sources[0].closed).toBe(true);
    b.dispatch(new board.Connected());
    expect(browser.sources).toHaveLength(2);
    // The new stream replays what was missed before showing live events.
    expect(b.model.caught_up).toBe(false);
    browser.sources[1].onmessage({ data: 'missed' });
    expect(Array.from(b.model.replay)).toEqual(['missed']);
    browser.sources[1].listeners.ready();
    expect(b.model.caught_up).toBe(true);
    // One open stream.
    expect(browser.sources.filter(s => !s.closed)).toHaveLength(1);
  } finally { browser.restore(); }
});

test('the board asks its root routes at the root on a contract page', async () => {
  const browser = fakeBrowser('', {}, '/p/web/contracts');
  try {
    const b = mount(board);
    b.dispatch(new board.Connected());
    await settle();
    expect(browser.paths).toContain('/projects');
    expect(browser.paths).not.toContain('/p/web/projects');
    expect(browser.sources.map(source => source.path)).toEqual(['/events']);
  } finally { browser.restore(); }
});

test('notices come and go', async () => {
  const browser = fakeBrowser();
  try {
    const b = mount(board);
    // A notice from a run that ended is dismissed.
    const task = run => JSON.stringify({ task: { id: 't1', title: 'T', body: '', column: 'assigned', rank: 1,
      runs: [{ id: 'r1', kind: 'work', state: run, model: 'm', log: '', outcome: run === 'ended' ? 'finished' : '' }] } });
    b.dispatch(new board.CaughtUp());
    b.dispatch(new board.Live(task('running')));
    b.dispatch(new board.Live(task('ended')));
    expect(Array.from(b.model.notices).map(n => n.notice.heading)).toEqual(['Finished T']);
    b.dispatch(new board.Dismiss(b.model.notices.head.notice));
    expect(Array.from(b.model.notices)).toEqual([]);
  } finally { browser.restore(); }
});

test('a question asked while the stream was down is toasted when it reopens', async () => {
  const browser = fakeBrowser();
  try {
    const b = mount(board);
    b.dispatch(new board.Connected());
    browser.sources[0].listeners.ready();
    const old = { id: 't1', title: 'T', body: '', column: 'assigned', rank: 1, runs: [], questions: [] };
    b.dispatch(new board.Live(JSON.stringify({ task: old })));
    // The browser closes the stream for good, and the page opens it from the last position.
    browser.sources[0].fail();
    expect(browser.timers).toHaveLength(1);
    browser.timers[0]();
    const reopened = browser.sources[1];
    expect(reopened.path).toBe('/events');
    // The replay carries the question asked during the gap.
    reopened.onmessage({ data: JSON.stringify({ task: old }), lastEventId: '{"tasks":1,".":0}' });
    const question = { id: 'q1', text: 'Which?', options: ['A', 'B'], answer: '' };
    reopened.onmessage({ data: JSON.stringify({ task: { ...old, questions: [question] } }), lastEventId: '{"tasks":2,".":0}' });
    expect(Array.from(b.model.notices).map(n => n.notice.heading)).toContain('Question on T');
  } finally { browser.restore(); }
});

test('the page registers the board and the task detail as elements', async () => {
  const tags = [];
  const saved = [globalThis.document, globalThis.HTMLElement, globalThis.customElements, globalThis.location];
  globalThis.location = { pathname: '/', search: '' };
  globalThis.document = { documentElement: { dataset: {}, style: {} } };
  globalThis.HTMLElement = class {};
  globalThis.customElements = { get: name => tags.includes(name) ? class {} : undefined, define: name => tags.push(name) };
  try {
    await import('../src/main.mjs');
    await settle();
    expect(tags.sort()).toEqual(['solace-board', 'solace-md-editor', 'solace-task']);
  } finally {
    [globalThis.document, globalThis.HTMLElement, globalThis.customElements, globalThis.location] = saved;
  }
});
