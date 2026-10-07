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
    if (path === '/projects') return Response.json({ workspace: false, projects: [{ dir: '', name: 'fixture', base: '' }], found: [] });
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
    dispatch(new board.Connected());
    await until(() => model.model === 'fake' && to_string(board.view(model)).includes('Initial') && events);
    dispatch(new board.Select('', 't1'));
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
    for (const pair of [['GET', '/projects'], ['GET', '/workflow'], ['GET', '/tasks'], ['GET', '/tasks/t1'], ['GET', '/models'], ['GET', '/events'], ['POST', '/tasks'], ['PATCH', '/tasks/t1'], ['POST', '/tasks/t1/notes'], ['POST', '/tasks/t1/actions'], ['POST', '/tasks/t1/questions/q1']]) expect(requests).toContainEqual(pair);
    dispatch(new board.Tab('runs')); expect(to_string(board.view(model))).toContain('/logs/r1');
    task.changes = [{ name: 'app.main' }];
    dispatch(new board.Select('', 't1')); dispatch(new board.Tab('changes'));
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
  // A workspace project's images load through its prefix; the stored URL stays project-relative.
  expect(markdown(`![shot](${url})`, '/p/bench%2Fpilot')).toBe(`<p><img alt="shot" src="/p/bench%2Fpilot${url}"></p>`);
  for (const bad of ['![x](https://evil/x.png)', '![x](javascript:alert(1))', '![x](/tasks/t/files/../../solace.toml)'])
    expect(markdown(bad)).not.toContain('<img');
  expect(markdown('![x" onerror="alert(1)](' + url + ')')).not.toContain('" onerror="');
  expect(markdown('[docs](https://example.com/a?b=1&c=2)')).toBe('<p><a href="https://example.com/a?b=1&amp;c=2" rel="noopener noreferrer">docs</a></p>');
  expect(markdown('[x](javascript:alert(1))')).not.toContain('<a');
});

test('markdown applies headings and lists mixed with other lines, nests lists, and keeps code literal', async () => {
  const { markdown } = await import('../src/api.mjs');
  expect(markdown('Summary:\n- fixed A\n- tested B')).toBe('<p>Summary:</p><ul><li>fixed A</li><li>tested B</li></ul>');
  expect(markdown('## Root cause\nthe cache was stale')).toBe('<h4>Root cause</h4><p>the cache was stale</p>');
  expect(markdown('- a\n  - b')).toBe('<ul><li>a<ul><li>b</li></ul></li></ul>');
  expect(markdown('**a * b**')).toBe('<p><strong>a * b</strong></p>');
  expect(markdown('`**x**`')).toBe('<p><code>**x**</code></p>');
  expect(markdown('1. one\nmixed\n2. two')).toBe('<ol><li>one</li></ol><p>mixed</p><ol><li>two</li></ol>');
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
  globalThis.location = { pathname: '/p/web/view' };
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

test('editor helpers act on the shadow root they are given, wherever the board is nested', async () => {
  const oldDocument = globalThis.document, oldFetch = globalThis.fetch;
  const uploads = [], inserted = [];
  const input = { classList: { contains: name => name === 'md-image' }, dataset: { task: 't1', for: 'body', base: '/p/shop' }, files: [{ type: 'image/png', name: 'picture.png' }], value: 'picked' };
  const editor = { selectionStart: 1, selectionEnd: 3 };
  const makeRoot = () => { const listeners = new Map(); return { listeners, getElementById: id => id === 'body' ? editor : null, addEventListener: (name, fn) => listeners.set(name, fn) }; };
  const shadowRoot = makeRoot(), other = makeRoot();
  // Inside a shell's own shadow root, document can't find the board.
  globalThis.document = { querySelector: () => null };
  globalThis.fetch = async (path, options) => { uploads.push([path, options.method]); return Response.json({ url: '/tasks/t1/files/0123456789abcdef.png' }); };
  try {
    const api = await import('../src/api.mjs');
    expect(api.wrap(shadowRoot, 'body', 'abcd', '**', '**')).toBe('a**bc**d');
    expect(api.wrap(null, 'body', 'abcd', '**', '**')).toBe('abcd****');
    api.images(shadowRoot, (id, image) => inserted.push([id, image]), () => {});
    api.images(other, () => {}, () => {});
    api.prevent_drag(other);
    expect([...other.listeners.keys()].sort()).toEqual(['change', 'dragover', 'dragstart', 'drop', 'paste']);
    shadowRoot.listeners.get('change')({ target: {}, composedPath: () => [input, shadowRoot], preventDefault() {} });
    for (let i = 0; i < 20 && !inserted.length; i++) await Bun.sleep(1);
    expect(uploads).toEqual([['/p/shop/tasks/t1/files', 'POST']]);
    expect(inserted).toEqual([['body', '![picture](/tasks/t1/files/0123456789abcdef.png)']]);
    expect(input.value).toBe('');
  } finally { globalThis.document = oldDocument; globalThis.fetch = oldFetch; }
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
    const view = await import('../build/dev/javascript/solace_board/solace_view.mjs');
    // createElement, then setAttribute('base'), then append: nothing is asked until it connects.
    const shop = mount(board);
    shop.dispatch(new board.BaseChanged('/p/shop'));
    expect(browser.paths).toEqual([]);
    expect(browser.sources).toEqual([]);
    shop.dispatch(new board.Connected());
    await settle();
    expect(browser.paths).toEqual(expect.arrayContaining(['/p/shop/projects', '/p/shop/models']));
    expect(browser.sources.map(s => s.path)).toEqual(['/p/shop/events']);
    // A second component on the same page keeps its own base.
    const contract = mount(view);
    contract.dispatch(new view.BaseChanged('/p/web'));
    contract.dispatch(new view.Connected());
    await settle();
    expect(browser.paths).toContain('/p/web/graph?task=t1');
    expect(browser.sources.map(s => s.path)).toEqual(['/p/shop/events', '/p/web/events']);
    // Changing one instance's base moves only its own requests and stream.
    contract.dispatch(new view.BaseChanged('/p/api'));
    await settle();
    expect(browser.paths).toContain('/p/api/graph?task=t1');
    expect(browser.sources.map(s => [s.path, !!s.closed])).toEqual([['/p/shop/events', false], ['/p/web/events', true], ['/p/api/events', false]]);
    // Without a base, the page address decides.
    const plain = mount(board);
    plain.dispatch(new board.Connected());
    expect(browser.sources.at(-1).path).toBe('/events');
  } finally { browser.restore(); }
});

test('a component removed and added again reopens one stream, which receives later events', async () => {
  const browser = fakeBrowser('?task=t1');
  try {
    const view = await import('../build/dev/javascript/solace_board/solace_view.mjs');
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

    const v = mount(view);
    v.dispatch(new view.Connected());
    v.dispatch(new view.Disconnected());
    v.dispatch(new view.Connected());
    await settle();
    expect(browser.sources[2].closed).toBe(true);
    const reloads = () => browser.paths.filter(p => p === '/graph?task=t1').length;
    const before = reloads();
    browser.sources[3].onmessage({ data: JSON.stringify({ task: { id: 't1' } }) });
    await settle();
    expect(reloads()).toBe(before + 1);
    // One open stream per component.
    expect(browser.sources.filter(s => !s.closed)).toHaveLength(2);
  } finally { browser.restore(); }
});

test('a followed run stops refreshing when the board is removed and resumes in one chain when added again', async () => {
  const browser = fakeBrowser('', { '/tasks/1/runs/r1': { state: 'running', transcript: [] } });
  const fetches = () => browser.paths.filter(p => p === '/tasks/1/runs/r1').length;
  const fire = async () => { browser.timers.splice(0).forEach(f => f()); await settle(); };
  try {
    const b = mount(board);
    b.dispatch(new board.Connected());
    b.dispatch(new board.TaskLoaded(JSON.stringify({ id: '1', title: 'T', body: '', column: 'assigned', rank: 1, notes: [],
      runs: [{ id: 'r1', kind: 'work', state: 'running', model: 'm', log: '/l', started: 1 }] })));
    b.dispatch(new board.Watch('r1'));
    await settle();
    expect(fetches()).toBe(1);
    await fire();
    expect(fetches()).toBe(2);
    // Removed: the pending refresh fetches nothing and schedules nothing.
    b.dispatch(new board.Disconnected());
    await fire(); await fire();
    expect(fetches()).toBe(2);
    expect(browser.timers).toHaveLength(0);
    // Added again: the transcript refreshes again.
    b.dispatch(new board.Connected());
    await settle();
    expect(fetches()).toBe(3);
    // Moved while a refresh is pending: the old step is dropped, so one chain remains.
    b.dispatch(new board.Disconnected());
    b.dispatch(new board.Connected());
    await settle();
    expect(fetches()).toBe(4);
    await fire();
    expect(fetches()).toBe(5);
    await fire();
    expect(fetches()).toBe(6);
  } finally { browser.restore(); }
});
