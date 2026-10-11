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
    expect(to_string(board.view(model))).toContain('<solace-task base dir hosted task="t1">');
    dispatch(new board.NewTask()); dispatch(new board.DraftTitle('New task')); dispatch(new board.DraftBody('Human description')); dispatch(new board.Create());
    await until(() => task.title === 'New task' && !model.creating && model.draft_title === '');
    // The created task opens.
    expect(to_string(board.view(model))).toContain('task="t2"');
    // A live update of the open task goes to its details, and a run that ended raises a notice.
    task.runs = [{ id: 'r1', kind: 'work', state: 'running', model: 'fake', log: '/logs/r1' }];
    events.enqueue(new TextEncoder().encode('id: 1\ndata: ' + JSON.stringify({ task }) + '\n\n'));
    await until(() => Array.from(model.live).some(m => m[1].includes('"running"')));
    task.runs = [{ ...task.runs[0], state: 'ended', outcome: 'finished' }];
    events.enqueue(new TextEncoder().encode('id: 2\ndata: ' + JSON.stringify({ task }) + '\n\n'));
    await until(() => to_string(board.view(model)).includes('<strong>Finished New task</strong>'));
    workflow.columns.push({ id: 'planning', label: 'Custom planning', actions: ['review'], final: false });
    events.enqueue(new TextEncoder().encode('id: 3\ndata: ' + JSON.stringify({ workflow }) + '\n\n'));
    await until(() => to_string(board.view(model)).includes('Custom planning'));
    for (const pair of [['GET', '/projects'], ['GET', '/workflow'], ['GET', '/events'], ['POST', '/tasks']]) expect(requests).toContainEqual(pair);
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
  const oldLocation = globalThis.location, oldFetch = globalThis.fetch, oldEventSource = globalThis.EventSource;
  const paths = [];
  let source;
  globalThis.location = { pathname: '/p/web/contracts' };
  globalThis.fetch = async path => { paths.push(path); return new Response('{}'); };
  globalThis.EventSource = class {
    constructor(path) { this.path = path; source = this; }
    addEventListener() {}
    close() { this.closed = true; }
  };
  try {
    const api = await import('../src/api.mjs');
    await new Promise(resolve => api.request('/p/shop', '/graph', 'GET', '', resolve));
    await new Promise(resolve => api.request('/p/shop', '/p/other/tasks', 'GET', '', resolve));
    await new Promise(resolve => api.request('', '/graph', 'GET', '', resolve));
    expect(paths).toEqual(['/p/shop/graph', '/p/other/tasks', '/p/web/graph']);
    expect(api.events('/p/shop', () => {})).toBe(source);
    expect(source.path).toBe('/p/shop/events');
    api.close_events(source);
    expect(source.closed).toBe(true);
  } finally {
    globalThis.location = oldLocation; globalThis.fetch = oldFetch; globalThis.EventSource = oldEventSource;
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

// A browser without a DOM for the component tests: requests and event streams
// are recorded, and timers wait until the test fires them.
function fakeBrowser(search = '', replies = {}) {
  const saved = Object.fromEntries(['fetch', 'EventSource', 'location', 'setTimeout'].map(k => [k, Object.getOwnPropertyDescriptor(globalThis, k)]));
  const paths = [], sources = [], timers = [];
  Object.defineProperty(globalThis, 'location', { configurable: true, value: { pathname: '/', search } });
  globalThis.fetch = async path => {
    paths.push(path);
    return Response.json(path in replies ? replies[path] : {});
  };
  globalThis.setTimeout = callback => { timers.push(callback); return 0; };
  globalThis.EventSource = class {
    listeners = {};
    constructor(path) { this.path = path; sources.push(this); }
    addEventListener(name, f) { this.listeners[name] = f; }
    close() { this.closed = true; }
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

test('the project panel sends only the requests its buttons make', async () => {
  const projects = { workspace: true, projects: [{ dir: 'a/b', name: 'ab', base: '' }],
    found: [{ dir: 'c/d', name: 'cd', imported: true, member: false }, { dir: 'e/f', name: 'ef', imported: false, member: false }] };
  const browser = fakeBrowser('', { '/projects': projects });
  const sent = [];
  globalThis.fetch = async (path, opts) => {
    sent.push([opts?.method ?? 'GET', path, opts?.body ? JSON.parse(opts.body) : null]);
    return Response.json(opts?.method === 'GET' && path === '/projects' ? projects : opts?.method === 'GET' && path.endsWith('/workflow') ? { columns: [] } : {});
  };
  try {
    const b = mount(board);
    b.dispatch(new board.Connected());
    await settle();
    expect(sent).toEqual([['GET', '/projects', null], ['GET', '/workflow', null]]);
    // An imported project that isn't a member joins; anything else sends nothing.
    b.dispatch(new board.AddProject('c/d'));
    b.dispatch(new board.AddProject('e/f'));
    b.dispatch(new board.AddProject('z/z'));
    await settle();
    expect(sent[2]).toEqual(['POST', '/projects', { dir: 'c/d' }]);
    expect(sent.length).toBe(3);
    // The new-project form posts what it holds.
    b.dispatch(new board.ProjectName('new'));
    b.dispatch(new board.ProjectDir('newdir'));
    b.dispatch(new board.ProjectTarget('javascript'));
    b.dispatch(new board.CreateProject());
    await settle();
    expect(sent[3]).toEqual(['POST', '/projects/new', { name: 'new', dir: 'newdir', target: 'javascript' }]);
    // Remove asks first whether to keep the files; deleting them needs the project's name.
    b.dispatch(new board.RemoveProject('a/b'));
    b.dispatch(new board.RemoveOnly());
    await settle();
    expect(sent[4]).toEqual(['DELETE', '/projects/a%2Fb', {}]);
    b.dispatch(new board.RemoveProject('a/b'));
    b.dispatch(new board.RemoveFiles());
    b.dispatch(new board.RemoveTyped('wrong'));
    b.dispatch(new board.RemoveDelete());
    await settle();
    expect(sent.length).toBe(5);
    b.dispatch(new board.RemoveTyped('a/b'));
    b.dispatch(new board.RemoveDelete());
    await settle();
    expect(sent[5]).toEqual(['DELETE', '/projects/a%2Fb', { delete: true, name: 'a/b' }]);
    // Cancel closes the dialog, so RemoveOnly then sends nothing.
    b.dispatch(new board.RemoveProject('a/b'));
    b.dispatch(new board.RemoveCancel());
    b.dispatch(new board.RemoveOnly());
    await settle();
    expect(sent.length).toBe(6);
  } finally { browser.restore(); }
});

test('the new task editor formats its selection, and notices come and go', async () => {
  const browser = fakeBrowser();
  const root = { addEventListener: () => {}, getElementById: () => ({ selectionStart: 1, selectionEnd: 2 }) };
  try {
    const b = mount(board);
    b.dispatch(new board.Attached(root));
    b.dispatch(new board.DraftBody('xy'));
    b.dispatch(new board.Format('draft-body', '*', '*'));
    await settle();
    expect(b.model.draft_body).toBe('x*y*');
    b.dispatch(new board.Block('draft-body', 'list'));
    await settle();
    expect(b.model.draft_body).toBe('- x*y*');
    // Previews add and remove; a notice from a run that ended is dismissed.
    b.dispatch(new board.Preview('draft-body', true));
    expect(Array.from(b.model.previews)).toEqual(['draft-body']);
    b.dispatch(new board.Preview('draft-body', false));
    expect(Array.from(b.model.previews)).toEqual([]);
    const task = run => JSON.stringify({ task: { id: 't1', title: 'T', body: '', column: 'assigned', rank: 1,
      runs: [{ id: 'r1', kind: 'work', state: run, model: 'm', log: '', outcome: run === 'ended' ? 'finished' : '' }] } });
    b.dispatch(new board.CaughtUp());
    b.dispatch(new board.Live(task('running')));
    b.dispatch(new board.Live(task('ended')));
    expect(Array.from(b.model.notices).map(n => n.heading)).toEqual(['Finished T']);
    b.dispatch(new board.Dismiss(b.model.notices.head));
    expect(Array.from(b.model.notices)).toEqual([]);
  } finally { browser.restore(); }
});

test('the page registers the board and the task detail as elements', async () => {
  const tags = [];
  const saved = [globalThis.document, globalThis.HTMLElement, globalThis.customElements, globalThis.location];
  globalThis.location = { pathname: '/', search: '' };
  globalThis.document = { documentElement: { dataset: {} } };
  globalThis.HTMLElement = class {};
  globalThis.customElements = { get: name => tags.includes(name) ? class {} : undefined, define: name => tags.push(name) };
  try {
    await import('../src/main.mjs');
    await settle();
    expect(tags.sort()).toEqual(['solace-board', 'solace-task']);
  } finally {
    [globalThis.document, globalThis.HTMLElement, globalThis.customElements, globalThis.location] = saved;
  }
});
