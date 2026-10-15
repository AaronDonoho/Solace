import { test, expect } from 'bun:test';
import * as task from '../build/dev/javascript/components/solace_task.mjs';
import { perform } from '../build/dev/javascript/lustre/lustre/effect.mjs';
import { to_string } from '../build/dev/javascript/lustre/lustre/element.mjs';

// One <solace-task>, driven as Lustre drives it; what it emits to its host is kept.
function mount() {
  const noop = () => {};
  const emitted = [];
  let model;
  const dispatch = msg => {
    let effect; [model, effect] = task.update(model, msg);
    perform(effect, dispatch, (name, data) => emitted.push([name, data]), noop, noop, noop, noop, noop);
  };
  [model] = task.init();
  return { dispatch, emitted, get model() { return model; }, html: () => to_string(task.view(model)) };
}
const settle = () => Bun.sleep(20);

// This server has no Solace store, process runner or filesystem access.
test('A11: <solace-task> uses only the HTTP API, including live task and workflow updates', async () => {
  const requests = [];
  let current = { id: 't1', title: 'Initial', body: 'Description', column: 'backlog', rank: 1,
    type: 'implementation', paused_from: null, notes: [], questions: [], runs: [], changes: [], stamps: {}, links: [] };
  const workflow = { columns: [{ id: 'backlog', label: 'Backlog', actions: ['plan'], final: false }]};
  let events;
  const server = Bun.serve({ port: 0, fetch: async req => {
    const path = new URL(req.url).pathname;
    requests.push([req.method, path]);
    if (path === '/events') return new Response(new ReadableStream({ start(c) { events = c; c.enqueue(new TextEncoder().encode('event: ready\ndata: {}\n\n')); } }), { headers: { 'Content-Type': 'text/event-stream' } });
    if (path === '/projects') return Response.json({ workspace: false, projects: [{ dir: '', name: 'fixture', base: '' }], found: [] });
    if (path === '/models') return Response.json([{ id: 'fake', label: 'Fake' }]);
    if (req.method === 'GET') return Response.json(path === '/graph' ? { review: { summary: '1 effects added, 0 signature changed, 0 test unfinished' } } : path === '/workflow' ? workflow : current);
    const body = await req.json();
    if (path.endsWith('/actions')) {
      if (body.action === 'complete') return Response.json({ code: 'E7006', message: 'Approval required' }, { status: 409 });
      current = { ...current, column: 'planning' };
    } else if (path.endsWith('/notes')) current.notes.push({ author: 'human', text: body.text });
    else if (path.includes('/questions/')) current.questions[0].answer = body.option || body.text;
    else current = { ...current, ...body };
    return Response.json(current);
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
  const t = mount();
  const until = async predicate => {
    for (let i = 0; i < 200; i++) { if (predicate()) return; await Bun.sleep(10); }
    throw new Error('UI did not reach expected state: ' + t.html());
  };
  const send = value => events.enqueue(new TextEncoder().encode('data: ' + JSON.stringify(value) + '\n\n'));
  try {
    t.dispatch(new task.TaskChanged('t1'));
    expect(requests).toEqual([]);
    t.dispatch(new task.Connected());
    await until(() => t.model.model === 'fake' && t.html().includes('Initial') && events && t.model.caught_up);
    t.dispatch(new task.Edit(true)); t.dispatch(new task.Title('Edited')); t.dispatch(new task.Save()); await until(() => current.title === 'Edited');
    t.dispatch(new task.Text('Human note')); t.dispatch(new task.AddNote());
    await until(() => t.html().includes('Human note'));
    t.dispatch(new task.Action('plan')); await until(() => current.column === 'planning');
    expect(confirmations).toHaveLength(0);
    t.dispatch(new task.Text('Keep these instructions'));
    for (const action of ['cancel', 'complete']) {
      const before = requests.length;
      const beforeModel = t.model;
      t.dispatch(new task.Action(action));
      await Bun.sleep(30);
      expect(requests.length).toBe(before);
      expect(t.model).toBe(beforeModel);
      expect(confirmations.at(-1)).toContain(current.title);
    }
    expect(confirmations[0]).toContain('reverts its changes');
    expect(confirmations[1]).toContain('keeps its changes');
    acceptConfirmation = true;
    const actions = () => requests.filter(([method, path]) => method === 'POST' && path.endsWith('/actions')).length;
    const beforeCancel = actions();
    t.dispatch(new task.Action('cancel'));
    await until(() => t.model.text === '');
    expect(actions()).toBe(beforeCancel + 1);
    t.dispatch(new task.Action('complete')); await until(() => t.model.error === 'Approval required');
    // A live update brings a question; its own stream tells the host in a `notice`.
    current.questions = [{ id: 'q1', text: 'Choose', options: ['yes', 'no'] }];
    current.runs = [{ id: 'r1', kind: 'work', state: 'waiting', model: 'fake', log: '/logs/r1' }];
    send({ task: current });
    t.dispatch(new task.Tab('questions')); await until(() => t.html().includes('Choose'));
    expect(t.emitted.filter(([name]) => name === 'notice').map(([, n]) => n.heading)).toEqual(['Question on Edited']);
    t.dispatch(new task.Answer('q1', 'yes')); await until(() => current.questions[0].answer === 'yes');
    workflow.columns = [{ id: 'planning', label: 'Custom planning', actions: ['review'], final: false }];
    send({ workflow });
    await until(() => t.html().includes('>review</button>'));
    // Another task's update changes nothing.
    send({ task: { ...current, id: 't9', title: 'Elsewhere' } });
    await settle();
    expect(t.html()).not.toContain('Elsewhere');
    for (const pair of [['GET', '/projects'], ['GET', '/workflow'], ['GET', '/tasks/t1'], ['GET', '/models'], ['GET', '/events'], ['PATCH', '/tasks/t1'], ['POST', '/tasks/t1/notes'], ['POST', '/tasks/t1/actions'], ['POST', '/tasks/t1/questions/q1']]) expect(requests).toContainEqual(pair);
    t.dispatch(new task.Tab('runs')); expect(t.html()).toContain('/logs/r1');
    // Events carry no changes: one arriving fetches the task's own, then its review.
    current.changes = [{ name: 'app.main' }];
    send({ task: { ...current, changes: undefined } });
    t.dispatch(new task.Tab('changes'));
    await until(() => t.html().includes('1 effects added'));
    expect(t.html()).toContain('/contracts?task=t1');
    expect(requests).toContainEqual(['GET', '/graph']);
  } finally {
    if (confirmOriginal === undefined) delete globalThis.confirm;
    else globalThis.confirm = confirmOriginal;
    controllers.forEach(c => c.abort()); server.stop(true); globalThis.fetch = fetchOriginal; delete globalThis.EventSource;
  }
});

test('markdown renders only images uploaded to the board, and http(s) links', async () => {
  const { markdown } = await import('../src/task_api.mjs');
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

test('markdown links a task by id, as [label](task:<id>) or a bare task:<id> (D-98)', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  const id = '0123456789abcdef', link = label => `<a href="/task/${id}" data-task="${id}">${label}</a>`;
  expect(markdown(`See [the fix](task:${id}).`)).toBe(`<p>See ${link('the fix')}.</p>`);
  expect(markdown(`After task:${id}, then`)).toBe(`<p>After ${link(`task:${id}`)}, then</p>`);
  expect(markdown(`- **[a](task:${id})**`)).toBe(`<ul><li><strong>${link('a')}</strong></li></ul>`);
  // A reference in a link's label stays inside that one link.
  expect(markdown(`[task:${id}](task:${id})`)).toBe(`<p>${link(`task:${id}`)}</p>`);
  // Code keeps it literal; a wrong id, a longer word or an image form stays text.
  expect(markdown(`\`task:${id}\``)).toBe(`<p><code>task:${id}</code></p>`);
  for (const text of ['task:0123', `task:${id}0`, `mytask:${id}`, `![x](task:${id})`, '[x](task:../../a)'])
    expect(markdown(text)).not.toContain('<a');
  expect(markdown(`[x" onclick="y](task:${id})`)).not.toContain('" onclick="');
});

test('markdown links a feature or milestone by id as it does a task (D-122)', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  const id = '0123456789abcdef';
  for (const kind of ['feature', 'milestone']) {
    const link = label => `<a href="/${kind}/${id}" data-${kind}="${id}">${label}</a>`;
    expect(markdown(`See [the plan](${kind}:${id}).`)).toBe(`<p>See ${link('the plan')}.</p>`);
    expect(markdown(`After ${kind}:${id}, then`)).toBe(`<p>After ${link(`${kind}:${id}`)}, then</p>`);
    expect(markdown(`[${kind}:${id}](${kind}:${id})`)).toBe(`<p>${link(`${kind}:${id}`)}</p>`);
    expect(markdown(`\`${kind}:${id}\``)).toBe(`<p><code>${kind}:${id}</code></p>`);
    for (const text of [`${kind}:0123`, `${kind}:${id}0`, `my${kind}:${id}`, `![x](${kind}:${id})`, `[x](${kind}:../../a)`])
      expect(markdown(text)).not.toContain('<a');
  }
  expect(markdown(`epic:${id}`)).not.toContain('<a');
});

test('markdown applies headings and lists mixed with other lines, nests lists, and keeps code literal', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  expect(markdown('Summary:\n- fixed A\n- tested B')).toBe('<p>Summary:</p><ul><li>fixed A</li><li>tested B</li></ul>');
  expect(markdown('## Root cause\nthe cache was stale')).toBe('<h4>Root cause</h4><p>the cache was stale</p>');
  expect(markdown('- a\n  - b')).toBe('<ul><li>a<ul><li>b</li></ul></li></ul>');
  expect(markdown('**a * b**')).toBe('<p><strong>a * b</strong></p>');
  expect(markdown('`**x**`')).toBe('<p><code>**x**</code></p>');
  expect(markdown('1. one\nmixed\n2. two')).toBe('<ol><li>one</li></ol><p>mixed</p><ol start="2"><li>two</li></ol>');
});

test('markdown renders blockquotes, including blocks inside them', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  expect(markdown('> a\n> **b**')).toBe('<blockquote><p>a<br><strong>b</strong></p></blockquote>');
  expect(markdown('> a\n>\n> b')).toBe('<blockquote><p>a</p><p>b</p></blockquote>');
  expect(markdown('> a\n> > b')).toBe('<blockquote><p>a</p><blockquote><p>b</p></blockquote></blockquote>');
  expect(markdown('> - x\n> - y')).toBe('<blockquote><ul><li>x</li><li>y</li></ul></blockquote>');
  expect(markdown('> ```\n> <b>\n> ```')).toBe('<blockquote><pre><code>&lt;b&gt;</code></pre></blockquote>');
  expect(markdown('before\n> quoted\nafter')).toBe('<p>before</p><blockquote><p>quoted</p></blockquote><p>after</p>');
  expect(markdown('> <script>')).toBe('<blockquote><p>&lt;script&gt;</p></blockquote>');
});

test('markdown nests quotes at most 100 deep; a deeper > stays text', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  const open = '<blockquote>'.repeat(100), close = '</blockquote>'.repeat(100);
  expect(markdown('>'.repeat(100) + ' x')).toBe(`${open}<p>x</p>${close}`);
  expect(markdown('>'.repeat(101) + ' x')).toBe(`${open}<p>&gt; x</p>${close}`);
  // Far deeper input renders without exhausting the stack.
  expect(markdown('>'.repeat(100000) + ' x')).toContain('<p>&gt;&gt;');
});

test('markdown keeps one numbered list across blank lines and starts it at its first number', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  expect(markdown('1. a\n1. b\n1. c')).toBe('<ol><li>a</li><li>b</li><li>c</li></ol>');
  expect(markdown('1. a\n\n1. b\n\n\n1. c')).toBe('<ol><li>a</li><li>b</li><li>c</li></ol>');
  expect(markdown('3. a\n1. b\n7. c')).toBe('<ol start="3"><li>a</li><li>b</li><li>c</li></ol>');
  expect(markdown('- a\n\n- b')).toBe('<ul><li>a</li><li>b</li></ul>');
  expect(markdown('1. a\n\ntext\n\n1. b')).toBe('<ol><li>a</li></ol><p>text</p><ol><li>b</li></ol>');
  expect(markdown('1. a\n   1. x\n   1. y')).toBe('<ol><li>a<ol><li>x</li><li>y</li></ol></li></ol>');
});

test('block turns whole selected lines into a markdown block', async () => {
  const { block } = await import('../src/task_api.mjs');
  const at = (selectionStart, selectionEnd = selectionStart) => ({ getElementById: () => ({ selectionStart, selectionEnd }) });
  const text = 'intro\none\n\ntwo\noutro';
  // From the middle of "one" to the middle of "two": both lines, set off by blank lines.
  expect(block(at(7, 13), 'e', text, 'numbered')).toBe('intro\n\n1. one\n\n2. two\n\noutro');
  expect(block(at(7, 13), 'e', text, 'list')).toBe('intro\n\n- one\n\n- two\n\noutro');
  expect(block(at(7, 13), 'e', text, 'quote')).toBe('intro\n\n> one\n>\n> two\n\noutro');
  expect(block(at(7, 13), 'e', text, 'heading')).toBe('intro\n\n#### one\n\n#### two\n\noutro');
  expect(block(at(7, 13), 'e', text, 'code')).toBe('intro\n\n```\none\n\ntwo\n```\n\noutro');
  // A selection ending at a line's start leaves that line out.
  expect(block(at(0, 2), 'e', 'a\nb', 'list')).toBe('- a\n\nb');
  // Without a selection, the cursor's line; on an empty line, a bare marker.
  expect(block(at(1), 'e', 'ab', 'quote')).toBe('> ab');
  expect(block(null, 'e', 'Hello\n', 'numbered')).toBe('Hello\n\n1. ');
  expect(block(null, 'e', '', 'code')).toBe('```\n\n```');
});

test('saved choices survive missing or throwing storage', async () => {
  const { load, store } = await import('../src/task_api.mjs');
  const original = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  try {
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('blocked'); } });
    expect(load('k')).toBe('');
    expect(() => store('k', 'v')).not.toThrow();
    const values = new Map();
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: { getItem: k => values.get(k) ?? null, setItem: (k, v) => values.set(k, v), removeItem: k => values.delete(k) } });
    store('solace-board-agent', 'fake'); expect(load('solace-board-agent')).toBe('fake');
    // A new element starts from the agent and effort chosen last.
    store('solace-board-effort', 'high');
    const t = mount();
    expect([t.model.model, t.model.effort]).toEqual(['fake', 'high']);
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
    const api = await import('../src/task_api.mjs');
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

test('editor helpers act on the shadow root they are given, and only on its own editors', async () => {
  const oldFetch = globalThis.fetch;
  const uploads = [], inserted = [];
  const editor = { selectionStart: 1, selectionEnd: 3 };
  const makeRoot = () => { const listeners = new Map(); return { listeners, getElementById: id => id === 'body' ? editor : null, addEventListener: (name, fn) => listeners.set(name, fn) }; };
  const shadowRoot = makeRoot(), outer = makeRoot();
  const input = root => ({ classList: { contains: name => name === 'md-image' }, dataset: { task: 't1', for: 'body', base: '/p/shop' },
    files: [{ type: 'image/png', name: 'picture.png' }], value: 'picked', getRootNode: () => root });
  globalThis.fetch = async (path, options) => { uploads.push([path, options.method]); return Response.json({ url: '/tasks/t1/files/0123456789abcdef.png' }); };
  try {
    const api = await import('../src/task_api.mjs');
    expect(api.wrap(shadowRoot, 'body', 'abcd', '**', '**')).toBe('a**bc**d');
    expect(api.wrap(null, 'body', 'abcd', '**', '**')).toBe('abcd****');
    api.images(shadowRoot, (id, image) => inserted.push([id, image]), () => {});
    api.images(outer, (id, image) => inserted.push(['outer', image]), () => {});
    expect([...outer.listeners.keys()].sort()).toEqual(['change', 'dragover', 'drop', 'paste']);
    // One picked image inside the nested root reaches both listeners as it bubbles out;
    // only the root that holds the editor uploads it.
    const picked = input(shadowRoot);
    const event = { target: {}, composedPath: () => [picked, shadowRoot], preventDefault() {} };
    shadowRoot.listeners.get('change')(event);
    outer.listeners.get('change')(event);
    for (let i = 0; i < 20 && !inserted.length; i++) await Bun.sleep(1);
    await settle();
    expect(uploads).toEqual([['/p/shop/tasks/t1/files', 'POST']]);
    expect(inserted).toEqual([['body', '![picture](/tasks/t1/files/0123456789abcdef.png)']]);
    expect(picked.value).toBe('');
  } finally { globalThis.fetch = oldFetch; }
});

test('an image pasted or dropped inside an editor\'s own shadow root uploads to its task, once', async () => {
  const oldFetch = globalThis.fetch;
  const uploads = [], inserted = [];
  const makeRoot = () => { const listeners = new Map(); return { listeners, addEventListener: (name, fn) => listeners.set(name, fn) }; };
  const page = makeRoot(), outer = makeRoot();
  // <solace-md-editor> in the page's root; CodeMirror's content in the editor's own root.
  const editor = { id: 'note-text', classList: { contains: name => name === 'md-editor' }, dataset: { task: 't1', base: '/p/shop' }, getRootNode: () => page };
  const inner = {}, content = { classList: { contains: () => false }, getRootNode: () => inner };
  const files = [{ type: 'image/png', name: 'shot.png' }];
  let prevented = 0;
  const event = { composedPath: () => [content, inner, editor, page, outer], clipboardData: { files }, dataTransfer: { files }, preventDefault() { prevented++; } };
  globalThis.fetch = async path => { uploads.push(path); return Response.json({ url: '/tasks/t1/files/0123456789abcdef.png' }); };
  try {
    const api = await import('../src/task_api.mjs');
    api.images(page, (id, image) => inserted.push([id, image]), () => {});
    api.images(outer, (id, image) => inserted.push(['outer', image]), () => {});
    for (const name of ['paste', 'drop']) { page.listeners.get(name)(event); outer.listeners.get(name)(event); }
    page.listeners.get('dragover')(event);
    outer.listeners.get('dragover')(event);
    for (let i = 0; i < 20 && inserted.length < 2; i++) await Bun.sleep(1);
    expect(uploads).toEqual(['/p/shop/tasks/t1/files', '/p/shop/tasks/t1/files']);
    expect(inserted).toEqual([['note-text', '![shot](/tasks/t1/files/0123456789abcdef.png)'], ['note-text', '![shot](/tasks/t1/files/0123456789abcdef.png)']]);
    // paste, drop and dragover, each by the page's root only.
    expect(prevented).toBe(3);
    // An editor of no task yet uploads nothing.
    delete editor.dataset.task;
    page.listeners.get('paste')(event);
    await settle();
    expect(uploads.length).toBe(2);
  } finally { globalThis.fetch = oldFetch; }
});

test('a text set on the editor from outside replaces only what differs, and keeps the selection sensible', async () => {
  const { replacement } = await import('../src/task_api.mjs');
  // Bold round "bc": the selection stays on "bc".
  expect(replacement('abcd', 'a**bc**d', 1, 3)).toEqual({ from: 1, to: 3, insert: '**bc**', anchor: 3, head: 5 });
  // Bold with nothing selected: the cursor goes between the marks; code the same.
  expect(replacement('ab', 'a****b', 1, 1)).toEqual({ from: 1, to: 1, insert: '****', anchor: 3, head: 3 });
  expect(replacement('ab', 'a``b', 1, 1)).toEqual({ from: 1, to: 1, insert: '``', anchor: 2, head: 2 });
  // An image or a link with nothing selected: the cursor ends it.
  expect(replacement('ab', 'a![x](/u)b', 1, 1)).toEqual({ from: 1, to: 1, insert: '![x](/u)', anchor: 9, head: 9 });
  // A list made of the selected line: only the new marks go in, and the cursor ends them.
  expect(replacement('one\ntwo', 'one\n\n- two', 4, 7)).toEqual({ from: 4, to: 4, insert: '\n- ', anchor: 7, head: 7 });
  // A box cleared after a send.
  expect(replacement('sent', '', 4, 4)).toEqual({ from: 0, to: 4, insert: '', anchor: 0, head: 0 });
  // The same text changes nothing.
  expect(replacement('same', 'same', 2, 2)).toEqual({ from: 4, to: 4, insert: '', anchor: 4, head: 4 });
});

test('the editor element is defined only where there is a browser', async () => {
  const { define_editor } = await import('../src/task_api.mjs');
  // No customElements here: CodeMirror, which this project doesn't install, is never loaded.
  expect(globalThis.customElements).toBeUndefined();
  expect(() => define_editor()).not.toThrow();
});

// A browser without a DOM: requests and event streams are recorded, and
// timers wait until the test fires them.
function fakeBrowser(replies = {}) {
  const saved = Object.fromEntries(['fetch', 'EventSource', 'location', 'setTimeout'].map(k => [k, Object.getOwnPropertyDescriptor(globalThis, k)]));
  const paths = [], sources = [], timers = [];
  Object.defineProperty(globalThis, 'location', { configurable: true, value: { pathname: '/', search: '' } });
  globalThis.fetch = async (path, opts) => {
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

test('nothing is asked until the element is added; a host that hands down events gets no second stream', async () => {
  const browser = fakeBrowser();
  try {
    // createElement, then the attributes, then append.
    const own = mount();
    own.dispatch(new task.BaseChanged('/p/shop'));
    own.dispatch(new task.TaskChanged('t1'));
    expect(browser.paths).toEqual([]);
    own.dispatch(new task.Connected());
    await settle();
    expect(browser.paths).toEqual(expect.arrayContaining(['/p/shop/tasks/t1', '/p/shop/workflow', '/models', '/projects']));
    expect(browser.sources.map(s => s.path)).toEqual(['/p/shop/events']);
    // Another task: the old stream closes and one for it opens.
    own.dispatch(new task.TaskChanged('t2'));
    expect(browser.sources[0].closed).toBe(true);
    expect(browser.sources.filter(s => !s.closed)).toHaveLength(1);
    // Hosted: no stream at all; the host's events apply at once.
    const hosted = mount();
    hosted.dispatch(new task.HostedChanged(true));
    hosted.dispatch(new task.TaskChanged('t3'));
    hosted.dispatch(new task.Connected());
    await settle();
    expect(browser.sources).toHaveLength(2);
    hosted.dispatch(new task.Live(JSON.stringify({ task: { id: 't3', title: 'From the host', body: '', column: 'x', rank: 1 } })));
    expect(hosted.html()).toContain('From the host');
  } finally { browser.restore(); }
});

test('a task that leaves its home project tells the host where it went', async () => {
  const browser = fakeBrowser();
  try {
    const t = mount();
    t.dispatch(new task.DirChanged('a'));
    t.dispatch(new task.BaseChanged('/p/a'));
    t.dispatch(new task.TaskChanged('t1'));
    t.dispatch(new task.ProjectsLoaded(JSON.stringify({ workspace: true, projects: [{ dir: 'a', name: 'a', base: '/p/a' }, { dir: 'b', name: 'b', base: '/p/b' }] })));
    t.dispatch(new task.Changed(JSON.stringify({ id: 't1', title: 'T', body: '', column: 'x', rank: 1, scope: ['b'] })));
    expect(t.emitted).toEqual([['task-moved', { id: 't1', dir: 'b', base: '/p/b' }]]);
  } finally { browser.restore(); }
});

test('a followed run stops refreshing when the element is removed and resumes in one chain when added again', async () => {
  const browser = fakeBrowser({ '/tasks/1/runs/r1': { state: 'running', transcript: [] } });
  const fetches = () => browser.paths.filter(p => p === '/tasks/1/runs/r1').length;
  const fire = async () => { browser.timers.splice(0).forEach(f => f()); await settle(); };
  try {
    const t = mount();
    t.dispatch(new task.Connected());
    t.dispatch(new task.TaskLoaded(t.model.generation, JSON.stringify({ id: '1', title: 'T', body: '', column: 'assigned', rank: 1, notes: [],
      runs: [{ id: 'r1', kind: 'work', state: 'running', model: 'm', log: '/l', started: 1 }] })));
    t.dispatch(new task.Watch('r1'));
    await settle();
    expect(fetches()).toBe(1);
    await fire();
    expect(fetches()).toBe(2);
    // Removed: the pending refresh fetches nothing and schedules nothing.
    t.dispatch(new task.Disconnected());
    await fire(); await fire();
    expect(fetches()).toBe(2);
    expect(browser.timers).toHaveLength(0);
    // Added again: the transcript refreshes again.
    t.dispatch(new task.Connected());
    await settle();
    expect(fetches()).toBe(3);
    // Moved while a refresh is pending: the old step is dropped, so one chain remains.
    t.dispatch(new task.Disconnected());
    t.dispatch(new task.Connected());
    await settle();
    expect(fetches()).toBe(4);
    await fire();
    expect(fetches()).toBe(5);
    await fire();
    expect(fetches()).toBe(6);
  } finally { browser.restore(); }
});

test('the editor sends what its buttons ask for, and wraps the selection in place', async () => {
  const browser = fakeBrowser();
  const sent = [];
  globalThis.fetch = async (path, opts) => {
    sent.push([opts?.method ?? 'GET', path, opts?.body ? JSON.parse(opts.body) : null]);
    return Response.json({});
  };
  const root = { addEventListener: () => {}, getElementById: () => ({ selectionStart: 1, selectionEnd: 3 }) };
  const answers = [false, true];
  globalThis.confirm = () => answers.shift();
  try {
    const t = mount();
    t.dispatch(new task.Attached(root));
    t.dispatch(new task.TaskLoaded(t.model.generation, JSON.stringify({ id: 't1', title: 'T', body: 'abcd', column: 'assigned', rank: 1 })));
    // A note posts its text and moves the pane to notes.
    t.dispatch(new task.Text('note text'));
    t.dispatch(new task.AddNote());
    await settle();
    expect(t.model.tab).toBe('notes');
    expect(sent).toContainEqual(['POST', '/tasks/t1/notes', { text: 'note text' }]);
    // An answer sends the option when there is one, the typed text when there isn't.
    t.dispatch(new task.Answer('q1', 'yes'));
    t.dispatch(new task.Answer('q2', ''));
    await settle();
    expect(sent).toContainEqual(['POST', '/tasks/t1/questions/q1', { option: 'yes' }]);
    expect(sent).toContainEqual(['POST', '/tasks/t1/questions/q2', { text: 'note text' }]);
    // Save needs a number for the rank; with one it patches the task.
    t.dispatch(new task.Rank('x'));
    t.dispatch(new task.Save());
    expect(t.model.error).toBe('Rank must be a number');
    expect(sent.filter(([m]) => m === 'PATCH')).toHaveLength(0);
    t.dispatch(new task.Title('T2'));
    t.dispatch(new task.Body('body text'));
    t.dispatch(new task.Rank('2.0'));
    t.dispatch(new task.Save());
    await settle();
    expect(sent).toContainEqual(['PATCH', '/tasks/t1', { title: 'T2', body: 'body text', rank: 2 }]);
    // Deleting a note asks first.
    t.dispatch(new task.DeleteNote(0));
    await settle();
    expect(sent.some(([m]) => m === 'DELETE')).toBe(false);
    t.dispatch(new task.DeleteNote(1));
    await settle();
    expect(sent).toContainEqual(['DELETE', '/tasks/t1/notes/1', {}]);
    // Format wraps the editor's selection: the description, then the note.
    t.dispatch(new task.Format('edit-body', '**', '**'));
    await settle();
    expect(t.model.body).toBe('b**od**y text');
    t.dispatch(new task.Text('typed'));
    t.dispatch(new task.Format('note-text', '[', ']'));
    await settle();
    expect(t.model.text).toBe('t[yp]ed');
    // The scope goes in the order the projects are listed.
    t.dispatch(new task.ProjectsLoaded(JSON.stringify({ workspace: true, projects: [{ dir: 'a', name: 'a', base: '' }, { dir: 'b', name: 'b', base: '/p/b' }] })));
    t.dispatch(new task.DirChanged('b'));
    t.dispatch(new task.SetScope('a', true));
    await settle();
    expect(sent).toContainEqual(['POST', '/tasks/t1/scope', { scope: ['a', 'b'] }]);
  } finally { browser.restore(); delete globalThis.confirm; }
});

test('<solace-task> registers once, however many parts of the page ask', async () => {
  const tags = [];
  const saved = [globalThis.document, globalThis.HTMLElement, globalThis.customElements];
  globalThis.document = { documentElement: { dataset: {} } };
  globalThis.HTMLElement = class {};
  globalThis.customElements = { get: name => tags.includes(name) ? class {} : undefined, define: name => tags.push(name) };
  try {
    expect(task.register().isOk()).toBe(true);
    expect(task.register().isOk()).toBe(true);
    expect(tags).toEqual(['solace-task']);
  } finally {
    [globalThis.document, globalThis.HTMLElement, globalThis.customElements] = saved;
  }
});
