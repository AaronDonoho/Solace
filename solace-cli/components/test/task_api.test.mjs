import { test, expect } from 'bun:test';
import * as task from '../build/dev/javascript/components/solace_task.mjs';
import { perform } from '../build/dev/javascript/lustre/lustre/effect.mjs';
import { to_string } from '../build/dev/javascript/lustre/lustre/element.mjs';
import { install as installDrafts, restore as restoreDrafts } from './draft_storage.mjs';

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
  installDrafts();
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
    if (path === '/role-models') return Response.json([{ role: '', name: 'Default', default: 'fake' }]);
    if (req.method === 'GET') return Response.json(path === '/graph/review' ? { summary: '1 effects added, 0 signature changed, 0 test unfinished' } : path === '/workflow' ? workflow : current);
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
    await until(() => t.model.acting === '');
    const { load_draft } = await import('../src/task_api.mjs');
    expect(t.model.text).toBe('');
    expect(load_draft('note-text', 't1')).toBe('');
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
    // The review is one summary, not the project's whole graph.
    current.changes = [{ name: 'app.main' }];
    send({ task: { ...current, changes: undefined } });
    t.dispatch(new task.Tab('changes'));
    await until(() => t.html().includes('1 effects added'));
    expect(t.html()).toContain('/contracts?task=t1');
    expect(requests).toContainEqual(['GET', '/graph/review']);
    expect(requests).not.toContainEqual(['GET', '/graph']);
  } finally {
    if (confirmOriginal === undefined) delete globalThis.confirm;
    else globalThis.confirm = confirmOriginal;
    controllers.forEach(c => c.abort()); server.stop(true); globalThis.fetch = fetchOriginal; delete globalThis.EventSource;
    restoreDrafts();
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

test('markdown links a feature, milestone or design by id as it does a task (D-122, D-234)', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  const id = '0123456789abcdef';
  for (const kind of ['feature', 'milestone', 'design']) {
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

test('markdown renders a table from a header, separator and body rows', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  const plain = '<table><thead><tr><th>Name</th><th>Value</th></tr></thead><tbody><tr><td>a</td><td>b</td></tr></tbody></table>';
  expect(markdown('| Name | Value |\n| --- | --- |\n| a | b |')).toBe(plain);
  expect(markdown('Name | Value\n--- | ---\na | b')).toBe(plain);
  // A table starts after a paragraph line and ends at a blank line.
  expect(markdown('Before:\n| Name | Value |\n| --- | --- |\n| a | b |\n\nAfter'))
    .toBe(`<p>Before:</p>${plain}<p>After</p>`);
});

test('markdown aligns table columns from the separator', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  expect(markdown('| L | C | R |\n| :--- | :---: | ---: |\n| a | b | c |')).toBe(
    '<table><thead><tr><th style="text-align:left">L</th><th style="text-align:center">C</th><th style="text-align:right">R</th></tr></thead><tbody><tr><td style="text-align:left">a</td><td style="text-align:center">b</td><td style="text-align:right">c</td></tr></tbody></table>');
});

test('markdown formats table cells and keeps escaped and code-span pipes', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  const id = '0123456789abcdef';
  expect(markdown(
    '| **bold** | [docs](https://example.com/a?b=1&c=2) |\n' +
    '| --- | --- |\n' +
    '| `x|y` | a \\| pipe and task:' + id + ' |',
  )).toBe(
    '<table><thead><tr><th><strong>bold</strong></th><th><a href="https://example.com/a?b=1&amp;c=2" rel="noopener noreferrer">docs</a></th></tr></thead><tbody><tr><td><code>x|y</code></td><td>a | pipe and <a href="/task/' + id + '" data-task="' + id + '">task:' + id + '</a></td></tr></tbody></table>');
});

test('markdown fills short table rows and drops extra cells', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  expect(markdown('| a | b | c |\n| --- | --- | --- |\n| 1 |\n| 1 | 2 | 3 | 4 |')).toBe(
    '<table><thead><tr><th>a</th><th>b</th><th>c</th></tr></thead><tbody><tr><td>1</td><td></td><td></td></tr><tr><td>1</td><td>2</td><td>3</td></tr></tbody></table>');
});

test('markdown leaves lines that only look like a table as a paragraph', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  expect(markdown('| a | b |\n| c | d |')).toBe('<p>| a | b |<br>| c | d |</p>');
  expect(markdown('| a | b |\n| --- |\n| c | d |')).toBe('<p>| a | b |<br>| --- |<br>| c | d |</p>');
});

test('markdown keeps an unmatched backtick in its table cell', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  expect(markdown('| a ` b | c |\n| --- | --- |')).toBe(
    '<table><thead><tr><th>a ` b</th><th>c</th></tr></thead><tbody></tbody></table>');
});

test('markdown keeps a pipe inside a double-backtick code span in its cell', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  expect(markdown('| ``a ` | b`` | c |\n| --- | --- |')).toBe(
    '<table><thead><tr><th><code>a ` | b</code></th><th>c</th></tr></thead><tbody></tbody></table>');
});

test('markdown keeps a list whose items look like a table', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  expect(markdown('- a | b\n- | -')).toBe('<ul><li>a | b</li><li>| -</li></ul>');
});

test('markdown reads inline code spans by their backtick run length', async () => {
  const { markdown } = await import('../src/task_api.mjs');
  expect(markdown('x ``a`b`` y')).toBe('<p>x <code>a`b</code> y</p>');
  expect(markdown('a ` b')).toBe('<p>a ` b</p>');
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

test('storage helpers survive missing or throwing storage', async () => {
  const { load, store } = await import('../src/task_api.mjs');
  const original = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  try {
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('blocked'); } });
    expect(load('k')).toBe('');
    expect(() => store('k', 'v')).not.toThrow();
    const values = new Map();
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: { getItem: k => values.get(k) ?? null, setItem: (k, v) => values.set(k, v), removeItem: k => values.delete(k) } });
    store('k', 'v'); expect(load('k')).toBe('v');
    // A new element leaves the model to the task's role.
    const t = mount();
    expect([t.model.model, t.model.effort]).toEqual(['', '']);
    store('k', ''); expect(load('k')).toBe('');
  } finally {
    if (original) Object.defineProperty(globalThis, 'localStorage', original); else delete globalThis.localStorage;
  }
});

test('drafts are keyed by editor and owner, and survive blocked storage', async () => {
  const { draft_key, load_draft, save_draft, clear_draft } = await import('../src/task_api.mjs');
  const original = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  try {
    const values = new Map();
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
      getItem: k => values.get(k) ?? null,
      setItem: (k, v) => values.set(k, v),
      removeItem: k => values.delete(k),
    } });
    expect(draft_key('note-text', 't1')).toBe('solace-draft:note-text:t1');
    expect(draft_key('note-text', 't2')).not.toBe(draft_key('note-text', 't1'));
    save_draft('note-text', 't1', 'half written');
    expect(load_draft('note-text', 't1')).toBe('half written');
    expect(load_draft('note-text', 't2')).toBe('');
    save_draft('note-text', 't1', '');
    expect(load_draft('note-text', 't1')).toBe('');
    save_draft('note-text', 't1', 'again');
    clear_draft('note-text', 't1');
    expect(load_draft('note-text', 't1')).toBe('');

    Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('blocked'); } });
    expect(load_draft('note-text', 't1')).toBe('');
    expect(() => save_draft('note-text', 't1', 'lost')).not.toThrow();
    expect(() => clear_draft('note-text', 't1')).not.toThrow();
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
    expect(paths).toEqual(['/p/shop/graph', '/p/other/tasks', '/graph']);
    expect(api.events('/p/shop', () => {})).toBe(source);
    expect(source.path).toBe('/p/shop/events');
    api.close_events(source);
    expect(source.closed).toBe(true);
    expect(api.events('', () => {}).path).toBe('/events');
  } finally {
    globalThis.location = oldLocation; globalThis.fetch = oldFetch; globalThis.EventSource = oldEventSource;
  }
});

test('a stream the browser closed for good reopens from the last event, with backoff', async () => {
  const saved = Object.fromEntries(['EventSource', 'setTimeout', 'clearTimeout'].map(k => [k, Object.getOwnPropertyDescriptor(globalThis, k)]));
  const sources = [], timers = [];
  globalThis.setTimeout = (callback, wait) => { timers.push({ callback, wait }); return timers.length; };
  globalThis.clearTimeout = () => {};
  globalThis.EventSource = class {
    readyState = 1;
    constructor(path) { this.path = path; sources.push(this); }
    addEventListener() {}
    close() { this.closed = true; this.readyState = 2; }
  };
  try {
    const api = await import('../src/task_api.mjs');
    const got = [];
    const source = api.events('/p/shop', text => got.push(text));
    expect(source.path).toBe('/p/shop/events');
    source.onmessage({ data: 'one', lastEventId: '{"tasks":3,".":2}' });
    expect(got).toEqual(['one']);
    source.readyState = 2;
    source.onerror();
    expect(timers[0].wait).toBe(1000);
    timers[0].callback();
    expect(sources[1].path).toBe('/p/shop/events?last=' + encodeURIComponent('{"tasks":3,".":2}'));
    sources[1].readyState = 2;
    sources[1].onerror();
    expect(timers[1].wait).toBe(2000);
    api.close_events(source);
    expect(sources[1].closed).toBe(true);
    timers[1].callback();
    expect(sources).toHaveLength(2);
    expect(api.events('', () => {}).path).toBe('/events');
  } finally {
    for (const [k, d] of Object.entries(saved)) d ? Object.defineProperty(globalThis, k, d) : delete globalThis[k];
  }
});

test('a rejected request reports a lost connection, not the browser exception', async () => {
  const oldFetch = globalThis.fetch;
  const replies = [];
  globalThis.fetch = async () => { throw new TypeError('Load Failed'); };
  try {
    const api = await import('../src/task_api.mjs');
    await new Promise(resolve => api.request('', '/tasks/t1', 'GET', '', text => {
      replies.push(text);
      resolve();
    }));
    expect(JSON.parse(replies[0])).toEqual({
      connectionLost: true,
      message: 'Connection lost; the request may not have completed.',
    });
    expect(replies[0]).not.toContain('Load Failed');
  } finally { globalThis.fetch = oldFetch; }
});

test('a receive callback that throws is not reported as a lost connection', async () => {
  const oldFetch = globalThis.fetch;
  const oldError = console.error;
  const replies = [], errors = [];
  console.error = error => errors.push(error);
  globalThis.fetch = async () => new Response('{}');
  try {
    const api = await import('../src/task_api.mjs');
    api.request('', '/tasks/t1', 'GET', '', text => {
      replies.push(text);
      throw new Error('UI callback failed');
    });
    await settle();
    expect(replies).toEqual(['{}']);
    expect(errors.map(error => error.message)).toEqual(['UI callback failed']);
  } finally {
    console.error = oldError;
    globalThis.fetch = oldFetch;
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

test('a rejected image upload says the connection was lost, not the browser exception', async () => {
  const oldFetch = globalThis.fetch;
  const failures = [];
  const listeners = new Map();
  const root = { addEventListener: (name, fn) => listeners.set(name, fn) };
  const input = { classList: { contains: name => name === 'md-image' }, dataset: { task: 't1', for: 'body', base: '' },
    files: [{ type: 'image/png', name: 'shot.png' }], value: 'shot', getRootNode: () => root };
  globalThis.fetch = async () => { throw new TypeError('Load Failed'); };
  try {
    const api = await import('../src/task_api.mjs');
    api.images(root, () => {}, text => failures.push(text));
    listeners.get('change')({ target: input, composedPath: () => [input, root], preventDefault() {} });
    for (let i = 0; i < 20 && !failures.length; i++) await Bun.sleep(1);
    expect(failures).toEqual(['Connection lost; the image was not uploaded.']);
  } finally { globalThis.fetch = oldFetch; }
});

test('a failed insert after an upload is not reported as a lost upload', async () => {
  const oldFetch = globalThis.fetch;
  const failures = [];
  const listeners = new Map();
  const root = { addEventListener: (name, fn) => listeners.set(name, fn) };
  const input = { classList: { contains: name => name === 'md-image' }, dataset: { task: 't1', for: 'body', base: '' },
    files: [{ type: 'image/png', name: 'shot.png' }], value: 'shot', getRootNode: () => root };
  globalThis.fetch = async () => Response.json({ url: '/tasks/t1/files/0123456789abcdef.png' });
  try {
    const api = await import('../src/task_api.mjs');
    api.images(root, () => { throw new Error('editor failed'); }, text => failures.push(text));
    listeners.get('change')({ target: input, composedPath: () => [input, root], preventDefault() {} });
    for (let i = 0; i < 20 && !failures.length; i++) await Bun.sleep(1);
    expect(failures).toEqual(['Image uploaded, but could not be inserted.']);
  } finally { globalThis.fetch = oldFetch; }
});

test('a non-JSON image reply is a failed upload, not a lost connection', async () => {
  const oldFetch = globalThis.fetch;
  const failures = [];
  const listeners = new Map();
  const root = { addEventListener: (name, fn) => listeners.set(name, fn) };
  const input = { classList: { contains: name => name === 'md-image' }, dataset: { task: 't1', for: 'body', base: '' },
    files: [{ type: 'image/png', name: 'shot.png' }], value: 'shot', getRootNode: () => root };
  globalThis.fetch = async () => new Response('<html>Bad Gateway</html>', { status: 502 });
  try {
    const api = await import('../src/task_api.mjs');
    api.images(root, () => {}, text => failures.push(text));
    listeners.get('change')({ target: input, composedPath: () => [input, root], preventDefault() {} });
    for (let i = 0; i < 20 && !failures.length; i++) await Bun.sleep(1);
    expect(failures).toEqual(['Image upload failed']);
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

test('the editor element is defined once, and only where there is a browser', async () => {
  const { define_editor } = await import('../src/task_api.mjs');
  // No customElements here: CodeMirror, which this project doesn't install, is never loaded.
  expect(globalThis.customElements).toBeUndefined();
  let loaded = 0;
  expect(() => define_editor(() => { loaded++; throw new Error('loaded'); })).not.toThrow();
  expect(loaded).toBe(0);
  // In a browser, concurrent asks share one load, and the element is defined once.
  const defined = [];
  const saved = globalThis.customElements;
  let finish;
  const done = new Promise(resolve => { finish = resolve; });
  globalThis.customElements = {
    get: name => defined.includes(name) ? class {} : undefined,
    define: name => defined.push(name),
  };
  try {
    const load = async () => {
      loaded++;
      return { define: () => {
        if (!customElements.get('solace-md-editor')) customElements.define('solace-md-editor', class {});
        finish();
      }};
    };
    define_editor(load);
    define_editor(load);
    await done;
    await settle();
    expect(loaded).toBe(1);
    expect(defined).toEqual(['solace-md-editor']);
    // Once defined, another ask loads nothing.
    define_editor(load);
    await settle();
    expect(loaded).toBe(1);
  } finally {
    globalThis.customElements = saved;
  }
});

test('a failed editor load is reported, and a later ask tries again', async () => {
  const { define_editor } = await import('../src/task_api.mjs');
  const saved = globalThis.customElements;
  const oldError = console.error;
  const errors = [], defined = [];
  let loaded = 0;
  globalThis.customElements = {
    get: name => defined.includes(name) ? class {} : undefined,
    define: name => defined.push(name),
  };
  console.error = (...args) => errors.push(args);
  try {
    define_editor(async () => { loaded++; throw new Error('CodeMirror unavailable'); });
    await settle();
    expect(loaded).toBe(1);
    expect(errors.map(([message, error]) => [message, error.message])).toEqual([
      ['Could not load the markdown editor.', 'CodeMirror unavailable'],
    ]);
    // A failed ask doesn't stop a later one from trying again.
    define_editor(async () => {
      loaded++;
      return { define: () => customElements.define('solace-md-editor', class {}) };
    });
    await settle();
    expect(loaded).toBe(2);
    expect(defined).toEqual(['solace-md-editor']);
  } finally {
    console.error = oldError;
    globalThis.customElements = saved;
  }
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

test('an option answer keeps note text that was not sent', async () => {
  installDrafts();
  const answer = {
    id: 't1', title: 'T', body: '', column: 'assigned', rank: 1, notes: [],
    questions: [{ id: 'q1', text: 'Why?', options: ['Yes'], answer: 'Yes' }],
    runs: [], changes: [], stamps: {}, links: [],
  };
  const before = {
    ...answer,
    questions: [{ ...answer.questions[0], answer: '' }],
  };
  const browser = fakeBrowser({ '/tasks/t1/questions/q1': answer });
  try {
    const t = mount();
    t.dispatch(new task.TaskLoaded(t.model.generation, JSON.stringify(before)));
    t.dispatch(new task.Text('Keep me'));
    t.dispatch(new task.Answer('q1', 'Yes'));
    await settle();
    expect(t.model.text).toBe('Keep me');
  } finally {
    browser.restore();
    restoreDrafts();
  }
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

test('a lost save reply checks the task again', async () => {
  const browser = fakeBrowser();
  const calls = [];
  globalThis.fetch = async (path, options) => {
    calls.push([options?.method ?? 'GET', path]);
    if (options?.method === 'PATCH') throw new TypeError('Load Failed');
    return new Promise(() => {});
  };
  try {
    const t = mount();
    t.dispatch(new task.TaskLoaded(t.model.generation, JSON.stringify({ id: 't1', title: 'T', body: '', column: 'assigned', rank: 1 })));
    t.dispatch(new task.Edit(true));
    t.dispatch(new task.Title('T2'));
    t.dispatch(new task.Save());
    await settle();
    expect(calls).toContainEqual(['PATCH', '/tasks/t1']);
    expect(calls).toContainEqual(['GET', '/tasks/t1']);
    expect(t.model.error).toBe('Connection lost; checking the task…');
  } finally { browser.restore(); }
});

test('a late lost change reply does not refresh the task shown now', async () => {
  const browser = fakeBrowser();
  try {
    const t = mount();
    t.dispatch(new task.TaskLoaded(t.model.generation, JSON.stringify({ id: 't1', title: 'One', body: '', column: 'x', rank: 1 })));
    const sent = t.model.generation;
    t.dispatch(new task.TaskChanged('t2'));
    t.dispatch(new task.TaskLoaded(t.model.generation, JSON.stringify({ id: 't2', title: 'Two', body: '', column: 'x', rank: 1 })));
    const before = browser.paths.length;
    t.dispatch(new task.ChangedAt(sent, JSON.stringify({ connectionLost: true, message: 'Connection lost; the request may not have completed.' })));
    await settle();
    expect(browser.paths.length).toBe(before);
    expect(t.model.error).toBe('');
  } finally { browser.restore(); }
});

test('<solace-task> registers once, however many parts of the page ask', () => {
  const tags = [];
  const saved = [globalThis.document, globalThis.HTMLElement, globalThis.customElements];
  globalThis.document = { documentElement: { dataset: {} } };
  globalThis.HTMLElement = class {};
  globalThis.customElements = {
    get: name => {
      return name === 'solace-md-editor' || tags.includes(name) ? class {} : undefined;
    },
    define: name => tags.push(name),
  };
  try {
    expect(task.register().isOk()).toBe(true);
    expect(task.register().isOk()).toBe(true);
    expect(tags).toEqual(['solace-task']);
  } finally {
    [globalThis.document, globalThis.HTMLElement, globalThis.customElements] = saved;
  }
});

test('focus_editor focuses the named editor and tolerates no root', async () => {
  const { focus_editor } = await import('../src/task_api.mjs');
  let focused = 0;
  const root = {
    getElementById: id => id === 'note-text' ? { focus: () => { focused++; } } : null,
  };
  focus_editor(root, 'note-text');
  focus_editor(root, 'other');
  expect(focused).toBe(1);
  expect(() => focus_editor(null, 'note-text')).not.toThrow();
});

test('the editor action pad follows the measured action row', async () => {
  const { watchActionPad } = await import('../src/editor_actions.mjs');
  const savedResize = globalThis.ResizeObserver, savedMutations = globalThis.MutationObserver;
  const style = new Map(), observed = [];
  let actions = null, mutation = null, resize = null;
  globalThis.ResizeObserver = class {
    constructor(callback) { resize = { callback }; }
    observe(target) { observed.push(target); }
    disconnect() { observed.length = 0; }
  };
  globalThis.MutationObserver = class {
    constructor(callback) { mutation = callback; }
    observe() {}
    disconnect() {}
  };
  const composer = {
    style: {
      setProperty: (name, value) => style.set(name, value),
      removeProperty: name => style.delete(name),
    },
    querySelector: () => actions,
  };
  try {
    const stop = watchActionPad(composer);
    expect(style.has('--md-action-pad')).toBe(false);
    actions = { getBoundingClientRect: () => ({ width: 120 }), scrollWidth: 120 };
    mutation();
    expect(style.get('--md-action-pad')).toBe('120px');
    expect(observed).toEqual([actions]);
    actions.getBoundingClientRect = () => ({ width: 84 });
    actions.scrollWidth = 84;
    resize.callback();
    expect(style.get('--md-action-pad')).toBe('84px');
    actions = null;
    mutation();
    expect(style.has('--md-action-pad')).toBe(false);
    stop();
  } finally {
    globalThis.ResizeObserver = savedResize;
    globalThis.MutationObserver = savedMutations;
  }
});

test('the editor action pad observes its row once', async () => {
  const { watchActionPad } = await import('../src/editor_actions.mjs');
  const savedResize = globalThis.ResizeObserver, savedMutations = globalThis.MutationObserver;
  const reports = [];
  let measures = 0;
  globalThis.ResizeObserver = class {
    constructor(callback) { this.callback = callback; }
    observe() { reports.push(this.callback); }
    disconnect() { reports.length = 0; }
  };
  globalThis.MutationObserver = class { observe() {} disconnect() {} };
  const actions = {
    getBoundingClientRect: () => { measures++; return { width: 120 }; },
    scrollWidth: 120,
  };
  const composer = {
    style: { setProperty() {}, removeProperty() {} },
    querySelector: () => actions,
  };
  try {
    const stop = watchActionPad(composer);
    expect(measures).toBe(1);
    expect(reports.length).toBe(1);
    reports.shift()();
    expect(measures).toBe(2);
    expect(reports.length).toBe(0);
    stop();
  } finally {
    globalThis.ResizeObserver = savedResize;
    globalThis.MutationObserver = savedMutations;
  }
});

test('the editor action pad measures without observers', async () => {
  const { watchActionPad } = await import('../src/editor_actions.mjs');
  const savedResize = globalThis.ResizeObserver, savedMutations = globalThis.MutationObserver;
  const style = new Map();
  const actions = { getBoundingClientRect: () => ({ width: 36 }), scrollWidth: 36 };
  const composer = {
    style: {
      setProperty: (name, value) => style.set(name, value),
      removeProperty: name => style.delete(name),
    },
    querySelector: () => actions,
  };
  globalThis.ResizeObserver = undefined;
  globalThis.MutationObserver = undefined;
  try {
    expect(() => watchActionPad(composer)).not.toThrow();
    expect(style.get('--md-action-pad')).toBe('36px');
  } finally {
    globalThis.ResizeObserver = savedResize;
    globalThis.MutationObserver = savedMutations;
  }
});
