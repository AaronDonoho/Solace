import { test, expect } from 'bun:test';
import * as strategy from '../build/dev/javascript/strategy/strategy.mjs';
import * as features from '../build/dev/javascript/strategy/features.mjs';
import * as personas from '../build/dev/javascript/strategy/personas.mjs';
import { None } from '../build/dev/javascript/gleam_stdlib/gleam/option.mjs';
import { toList } from '../build/dev/javascript/strategy/gleam.mjs';
import { perform } from '../build/dev/javascript/lustre/lustre/effect.mjs';
import { to_string } from '../build/dev/javascript/lustre/lustre/element.mjs';
import { readFile } from 'node:fs/promises';

const frame = (data, id = 0) => 'id: ' + id + '\ndata: ' + JSON.stringify(data) + '\n\n';
const encode = text => new TextEncoder().encode(text);

// The message box grows with its text where the browser has no `field-sizing`.
// D-122: a feature or milestone link's card is scrolled to, focused and marked, once it is drawn.
test('a linked card is revealed once it is drawn', async () => {
  const { reveal_card } = await import('../src/api.mjs');
  const classes = new Set(), attrs = {};
  let scrolled = false, focused = false, drawn = false;
  const card = {
    scrollIntoView: () => { scrolled = true; }, focus: () => { focused = true; },
    hasAttribute: n => n in attrs, setAttribute: (n, v) => { attrs[n] = v; },
    classList: { add: c => classes.add(c), remove: c => classes.delete(c) },
  };
  const asked = [];
  const root = { querySelector: s => { asked.push(s); return drawn ? card : null; } };
  reveal_card(root, 'feature', 'f1');
  expect(scrolled).toBe(false);
  drawn = true;
  await new Promise(r => setTimeout(r, 200));
  expect(asked[0]).toBe('[data-feature-id="f1"]');
  expect([scrolled, focused, attrs.tabindex, classes.has('revealed')]).toEqual([true, true, '-1', true]);
  asked.length = 0;
  reveal_card(root, 'milestone', 'a"]');
  expect(asked).toEqual([]);
});

// A workspace root: its own `/tasks` holds only its project, so the list comes from the merged stream.
// A feature card dragged by its grip: the others slide aside, and the drop reports the index it passes.
test('a dragged feature card reports where it lands, or -1 when cancelled', async () => {
  const { drag_card } = await import('../src/api.mjs');
  const make = top => ({
    style: {}, classes: new Set(),
    classList: { add(c) { this.owner.classes.add(c); }, remove(c) { this.owner.classes.delete(c); } },
    getBoundingClientRect: () => ({ top, bottom: top + 50 }),
  });
  const cards = [make(0), make(60), make(120)];
  cards.forEach(c => { c.classList.owner = c; });
  const section = { querySelectorAll: () => cards };
  cards.forEach(c => { c.closest = () => section; });
  const root = { querySelector: () => cards[0] };
  const listeners = {};
  const [add, remove] = [globalThis.addEventListener, globalThis.removeEventListener];
  globalThis.addEventListener = (k, f) => { listeners[k] = f; };
  globalThis.removeEventListener = k => { delete listeners[k]; };
  try {
    const landed = [];
    drag_card(root, 'f1', 25, i => landed.push(i));
    expect(cards[0].classes.has('dragging')).toBe(true);
    // Past the second card's middle (85): the second slides up into its place.
    listeners.pointermove({ clientY: 95 });
    expect([cards[0].style.transform, cards[1].style.transform, cards[2].style.transform]).toEqual(['translateY(70px)', 'translateY(-60px)', '']);
    listeners.pointerup({});
    expect(landed).toEqual([1]);
    expect([cards[0].classes.has('dragging'), cards[1].style.transform, listeners.pointermove]).toEqual([false, '', undefined]);
    drag_card(root, 'f1', 25, i => landed.push(i));
    listeners.pointermove({ clientY: 200 });
    listeners.keydown({ key: 'Escape' });
    expect(landed).toEqual([1, -1]);
  } finally {
    globalThis.addEventListener = add;
    globalThis.removeEventListener = remove;
  }
});

test('Tasks tab merges every project, sorts and filters, opens a task and links it inline', async () => {
  const requests = [];
  const workflow = { columns: [{ id: 'backlog', label: 'Backlog', actions: [], final: false }, { id: 'done', label: 'Done', actions: [], final: true }] };
  const blank = { body: '', rank: 1, paused_from: null, stamps: {}, links: [], runs: [], questions: [], feature: null, milestone: null, dependsOn: [], closedTime: null };
  const older = { ...blank, id: 't1', title: 'Older', column: 'done', createdTime: 50, closedTime: 100 };
  let newer = { ...blank, id: 't2', title: 'Newer', column: 'done', closedTime: 200 };
  const open = { ...blank, id: 't3', title: 'Still open', body: 'needs a search box', column: 'backlog' };
  const orphan = { ...blank, id: 't9', title: 'Homeless task', body: 'no project of its own', column: 'backlog' };
  let events, patch;
  const server = Bun.serve({ port: 0, fetch: async req => {
    const path = new URL(req.url).pathname;
    requests.push([req.method, path]);
    if (path === '/events') return new Response(new ReadableStream({ start(c) { events = c; c.enqueue(encode(
      frame({ project: '.', workflow }) + frame({ project: '.', tasks: [older, open] }) +
      frame({ project: 'web', workflow }) + frame({ project: 'web', tasks: [newer] }) +
      frame({ project: 'web', noProject: true, task: orphan }) +
      frame({ milestones: [{ id: 'm2', title: 'Empty', rank: 2 }, { id: 'm1', title: 'Launch', rank: 1 }] }) +
      frame({ features: [{ id: 'f2', title: 'Loose', body: '', milestone: null, rank: 1, state: 'proposed' }, { id: 'f1', title: 'Search', body: 'A **fast** lookup', milestone: 'm1', rank: 1, state: 'active' }] }) +
      'event: ready\ndata: {}\n\n')); } }), { headers: { 'Content-Type': 'text/event-stream' } });
    if (path === '/projects') return Response.json({ workspace: true, projects: [{ dir: '.', label: 'root', name: 'root', base: '' }, { dir: 'web', label: 'web', name: 'web', base: '/p/web' }], found: [] });
    if (path.endsWith('/workflow')) return Response.json(workflow);
    if (req.method === 'PATCH' && path === '/p/web/tasks/t2') { patch = await req.json(); newer = { ...newer, ...patch }; if (patch.feature) newer.milestone = 'm1'; return Response.json(newer); }
    return Response.json({ message: 'unexpected ' + path }, { status: 404 });
  }});
  const fetchOriginal = globalThis.fetch;
  const controllers = [];
  globalThis.fetch = (path, options) => fetchOriginal(new URL(path, server.url), options);
  // EventSource is a browser primitive absent from Bun; consume real SSE here.
  globalThis.EventSource = class {
    listeners = {};
    addEventListener(name, f) { this.listeners[name] = f; }
    close() {}
    constructor(path) {
      const controller = new AbortController(); controllers.push(controller);
      fetch(path, { signal: controller.signal }).then(async response => {
        let text = '';
        for await (const chunk of response.body) {
          text += new TextDecoder().decode(chunk);
          while (text.includes('\n\n')) {
            const at = text.indexOf('\n\n'); const block = text.slice(0, at); text = text.slice(at + 2);
            const data = block.split('\n').find(line => line.startsWith('data: '));
            const name = block.split('\n').find(line => line.startsWith('event: '));
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
  const dispatch = msg => { let effect; [model, effect] = strategy.update(model, msg); execute(effect); };
  const html = () => to_string(strategy.view(model));
  const until = async (predicate, detail = html) => {
    for (let i = 0; i < 200; i++) { if (predicate()) return; await Bun.sleep(10); }
    throw new Error('UI did not reach expected state: ' + detail());
  };
  const row = id => strategy.visible(model).toArray().find(pair => pair[1].id === id)[1];
  // A row's HTML, so a failed check prints the row instead of the whole page.
  const row_html = id => {
    const page = html();
    const title = row(id)?.title ?? id;
    const at = page.indexOf(title);
    const end = page.indexOf('</tr>', at);
    return at < 0 || end < 0 ? page.slice(0, 600) : page.slice(at, end + 5);
  };
  try {
    // `new` is a JavaScript keyword: Gleam exports it as `new$`.
    model = strategy.new$(new strategy.Tasks());
    dispatch(new strategy.Connected());
    await until(() => html().includes('Still open') && html().includes('Newer') && html().includes('Launch') && events);
    // Closed newest first, then open; column labels come from each project's workflow.
    const page = html();
    expect(page.indexOf('Newer')).toBeLessThan(page.indexOf('Older'));
    expect(page.indexOf('Older')).toBeLessThan(page.indexOf('Still open'));
    expect(page).toContain('Backlog');
    // D-130: a Created column, in the closed column's format.
    const { clock_date, clock_time } = await import('../src/api.mjs');
    expect(page).toContain('<button class="sort">Created</button></th>');
    expect(page).toContain('<td class="created"><span class="date">' + clock_date(50) + '</span><span class="time">' + clock_time(50) + '</span></td>');
    expect(page).toContain('<td class="closed"><span class="date">' + clock_date(100) + '</span><span class="time">' + clock_time(100) + '</span></td>');
    // D-129: a task with no project shows on the Tasks tab, marked, and the Project filter offers it.
    expect(page).toContain('Homeless task');
    expect(page).toContain('>No project</td>');
    dispatch(new strategy.FilterOpen(new strategy.ProjectField()));
    expect(html()).toContain('(no project)');
    dispatch(new strategy.FilterBlur(new strategy.ProjectField()));
    // Filters and search run on the client.
    dispatch(new strategy.Filter(new strategy.Filters('', 'web', '', '', '')));
    expect(html()).not.toContain('Older');
    expect(html()).not.toContain('Homeless task');
    dispatch(new strategy.Filter(new strategy.Filters('', 'no-project', '', '', '')));
    expect(html()).toContain('Homeless task');
    expect(html()).not.toContain('Older');
    dispatch(new strategy.Filter(new strategy.Filters('', '', '', '', 'homeless')));
    expect(html()).toContain('Homeless task');
    expect(html()).not.toContain('Still open');
    dispatch(new strategy.Filter(new strategy.Filters('', '', '', '', 'SEARCH box')));
    expect(html()).toContain('Still open');
    expect(html()).not.toContain('Newer');
    dispatch(new strategy.Filter(new strategy.Filters('', '', '', '', '')));
    // A row opens the task's details from its own project.
    dispatch(new strategy.Open('web', 't2'));
    expect(html()).toMatch(/<solace-task [^>]*base="\/p\/web"[^>]*dir="web"/);
    expect(html()).toMatch(/<solace-task [^>]*task="t2"/);
    // D-129: a task with no project opens at the root, in the homeless bucket.
    dispatch(new strategy.Open('no-project', 't9'));
    expect(html()).toMatch(/<solace-task [^>]*base dir="no-project"[^>]*task="t9"/);
    // A live update of the open task reaches its details and its row.
    dispatch(new strategy.Open('web', 't2'));
    newer = { ...newer, title: 'Newer, renamed' };
    events.enqueue(encode(frame({ project: 'web', task: newer }, 1)));
    await until(() => Array.from(model.live).some(m => m[1].includes('renamed')));
    expect(html()).toContain('Newer, renamed');
    // A task with no milestone shows a dash; the Feature link is one PATCH to the task's project.
    expect(row_html('t2')).toContain('<td>—</td>');
    expect(row_html('t2')).not.toContain('aria-label="Launch, milestone of Newer, renamed"');
    dispatch(new strategy.Link('web', 't2', 'feature', 'f1'));
    await until(() => row('t2').feature[0] === 'f1');
    expect(patch).toEqual({ feature: 'f1' });
    // D-188: the feature brings its milestone; the cell is a read-only record link.
    await until(() => row_html('t2').includes('aria-label="Launch, milestone of Newer, renamed"'), () => row_html('t2'));
    expect(row_html('t2')).toContain('<a aria-label="Launch, milestone of Newer, renamed" href="/milestone/m1">Launch</a>');
    expect(html()).toContain('aria-label="Feature of Newer, renamed"');
    // The Features tab groups by milestone in rank order, then "No milestone"; a feature lists its tasks from every project.
    dispatch(new strategy.Select(new strategy.Features()));
    // A compact card shows its body, tasks and new-task form only while focused.
    dispatch(new strategy.FocusFeature('f1'));
    const list = html();
    const at = text => list.indexOf(text, list.indexOf('id="panel-features"'));
    expect(at('Launch')).toBeGreaterThan(-1);
    expect(at('Launch')).toBeLessThan(at('Search'));
    expect(at('Search')).toBeLessThan(at('Empty'));
    expect(at('Empty')).toBeLessThan(at('No features.'));
    expect(at('No features.')).toBeLessThan(at('No milestone'));
    expect(at('No milestone')).toBeLessThan(at('Loose'));
    expect(list).toContain('class="state state-active">active<');
    expect(list).toContain('<strong>fast</strong>');
    expect(at('Newer')).toBeGreaterThan(at('Search'));
    expect(at('Newer')).toBeLessThan(at('Empty'));
    // Clicking a task opens it on the Tasks tab.
    dispatch(new strategy.Open('web', 't2'));
    expect(model.tab).toBeInstanceOf(strategy.Tasks);
    expect(html()).toMatch(/<solace-task [^>]*task="t2"/);
    dispatch(new strategy.Link('web', 't2', 'feature', ''));
    await until(() => patch.feature === null);
    await until(() => row('t2').feature instanceof None);
    expect(row_html('t2')).toContain('<a aria-label="Launch, milestone of Newer, renamed" href="/milestone/m1">Launch</a>');
    for (const pair of [['GET', '/projects'], ['GET', '/workflow'], ['GET', '/p/web/workflow'], ['GET', '/events'], ['PATCH', '/p/web/tasks/t2']]) expect(requests).toContainEqual(pair);
  } finally {
    controllers.forEach(c => c.abort()); server.stop(true); globalThis.fetch = fetchOriginal; delete globalThis.EventSource;
  }
});

// D-91: the vision comes with the stream; a save names the revision the edit began from.
test('Vision tab shows the document, edits it and saves over its revision', async () => {
  let vision = { text: '# Why\nFor teams.', revision: 'r1' }, put;
  const server = Bun.serve({ port: 0, fetch: async req => {
    const path = new URL(req.url).pathname;
    if (path === '/events') return new Response(encode(frame({ vision }) + 'event: ready\ndata: {}\n\n'), { headers: { 'Content-Type': 'text/event-stream' } });
    if (path === '/projects') return Response.json({ workspace: false, projects: [{ dir: '.', label: 'root', name: 'root', base: '' }], found: [] });
    if (path === '/workflow') return Response.json({ columns: [] });
    if (path === '/vision' && req.method === 'PUT') {
      put = await req.json();
      if (put.base !== vision.revision) return Response.json({ code: 'E7026', message: 'E7026 the vision changed' }, { status: 409 });
      vision = { text: put.text, revision: 'r2' };
      return Response.json(vision);
    }
    return Response.json({ message: 'unexpected ' + path }, { status: 404 });
  }});
  const fetchOriginal = globalThis.fetch;
  globalThis.fetch = (path, options) => fetchOriginal(new URL(path, server.url), options);
  globalThis.EventSource = class {
    listeners = {};
    addEventListener(name, f) { this.listeners[name] = f; }
    close() {}
    constructor(path) {
      fetch(path).then(r => r.text()).then(text => {
        for (const block of text.split('\n\n')) {
          const data = block.split('\n').find(line => line.startsWith('data: '));
          const name = block.split('\n').find(line => line.startsWith('event: '));
          if (name) this.listeners[name.slice(7)]?.({ data: data?.slice(6) });
          else if (data) this.onmessage?.({ data: data.slice(6) });
        }
      }).catch(() => {});
    }
  };
  let model;
  const noop = () => {};
  const dispatch = msg => { let effect; [model, effect] = strategy.update(model, msg); perform(effect, dispatch, noop, noop, noop, noop, noop, noop); };
  const html = () => to_string(strategy.view(model));
  const until = async predicate => {
    for (let i = 0; i < 200; i++) { if (predicate()) return; await Bun.sleep(10); }
    throw new Error('UI did not reach expected state: ' + html());
  };
  try {
    model = strategy.new$(new strategy.Vision());
    dispatch(new strategy.Connected());
    await until(() => html().includes('For teams.'));
    dispatch(new strategy.EditVision());
    expect(html()).toContain('id="vision-text"');
    dispatch(new strategy.Draft('For small teams.'));
    expect(html()).toContain('16 / 1000 character limit');
    dispatch(new strategy.SaveVision());
    await until(() => !html().includes('id="vision-text"'));
    expect(put).toEqual({ text: 'For small teams.', base: 'r1' });
    expect(html()).toContain('For small teams.');
    // Someone else saves meanwhile: this draft's save is refused, and the draft stays.
    vision = { text: 'Theirs', revision: 'r3' };
    dispatch(new strategy.EditVision());
    dispatch(new strategy.Draft('Late'));
    dispatch(new strategy.SaveVision());
    await until(() => html().includes('E7026'));
    expect(put).toEqual({ text: 'Late', base: 'r2' });
    expect(html()).toContain('id="vision-text"');
    // The editor holds the draft in its `value` property, which the HTML string doesn't show.
    expect(model.draft[0][0]).toBe('Late');
  } finally {
    server.stop(true); globalThis.fetch = fetchOriginal; delete globalThis.EventSource;
  }
});

// D-87: a create names its `request`; an edit sends only what changed; a refusal keeps the form.
test('Features tab creates a feature and edits it with only the changed fields', async () => {
  let feature = { id: 'f1', title: 'Search', body: '', milestone: null, rank: 1, state: 'proposed' }, post, patch, refuse = false;
  let tags = { id: 'f3', title: 'Tags', body: '', milestone: null, rank: 3, state: 'proposed' };
  const moves = [], deleted = [];
  const linked = { id: 't1', title: 'Index', body: '', rank: 1, column: 'backlog', paused_from: null, stamps: {}, links: [], runs: [], questions: [], feature: 'f1', milestone: null, closedTime: null };
  const server = Bun.serve({ port: 0, fetch: async req => {
    const path = new URL(req.url).pathname;
    if (path === '/events') return new Response(encode(frame({ milestones: [{ id: 'm1', title: 'Launch', rank: 1 }] }) +
      frame({ features: [feature, tags] }) + frame({ project: '.', workflow: { columns: [] } }) +
      frame({ project: '.', tasks: [linked] }) + 'event: ready\ndata: {}\n\n'), { headers: { 'Content-Type': 'text/event-stream' } });
    if (path === '/features/f3' && req.method === 'PATCH') {
      const body = await req.json();
      // A start refused while a run is open (D-100) answers with its error.
      if (refuse && body.state === 'active') return Response.json({ code: 'E7003', message: 'E7003 a run is open on this feature' }, { status: 400 });
      moves.push(body);
      tags = { ...tags, ...body };
      return Response.json(tags);
    }
    if (path === '/features/f1' && req.method === 'DELETE') { deleted.push(path); return Response.json(feature); }
    if (path === '/projects') return Response.json({ workspace: false, projects: [{ dir: '.', label: 'root', name: 'root', base: '' }], found: [] });
    if (path === '/workflow') return Response.json({ columns: [] });
    if (path === '/features' && req.method === 'POST') {
      post = await req.json();
      return Response.json({ id: 'f2', title: post.title, body: post.body, milestone: post.milestone ?? null, rank: 2, state: post.state });
    }
    if (path === '/features/f1' && req.method === 'PATCH') {
      patch = await req.json();
      if (patch.milestone === 'm9') return Response.json({ code: 'E7023', message: 'E7023 unknown milestone m9' }, { status: 400 });
      feature = { ...feature, ...patch };
      return Response.json(feature);
    }
    return Response.json({ message: 'unexpected ' + path }, { status: 404 });
  }});
  const fetchOriginal = globalThis.fetch;
  globalThis.fetch = (path, options) => fetchOriginal(new URL(path, server.url), options);
  globalThis.EventSource = class {
    listeners = {};
    addEventListener(name, f) { this.listeners[name] = f; }
    close() {}
    constructor(path) {
      fetch(path).then(r => r.text()).then(text => {
        for (const block of text.split('\n\n')) {
          const data = block.split('\n').find(line => line.startsWith('data: '));
          const name = block.split('\n').find(line => line.startsWith('event: '));
          if (name) this.listeners[name.slice(7)]?.({ data: data?.slice(6) });
          else if (data) this.onmessage?.({ data: data.slice(6) });
        }
      }).catch(() => {});
    }
  };
  let model;
  const noop = () => {};
  const dispatch = msg => { let effect; [model, effect] = strategy.update(model, msg); perform(effect, dispatch, noop, noop, noop, noop, noop, noop); };
  const html = () => to_string(strategy.view(model));
  const until = async predicate => {
    for (let i = 0; i < 200; i++) { if (predicate()) return; await Bun.sleep(10); }
    throw new Error('UI did not reach expected state: ' + html());
  };
  try {
    model = strategy.new$(new strategy.Features());
    dispatch(new strategy.Connected());
    await until(() => html().includes('Search'));
    // With a single project, a feature's new-task form has no project picker.
    // The form shows only on the focused feature's card.
    dispatch(new strategy.FocusFeature('f1'));
    await until(() => html().includes('Create task'));
    expect(html()).not.toContain('name="project"');
    // Create: the form has its own textarea, a state and a milestone select with "(none)".
    dispatch(new strategy.NewFeature());
    expect(html()).toContain('id="feature-text"');
    expect(html()).toContain('>(none)<');
    expect(html()).toContain('>canceled<');
    dispatch(new strategy.FeatureTitle('Export'));
    dispatch(new strategy.FeatureText('CSV and JSON'));
    dispatch(new strategy.FeatureMilestone('m1'));
    dispatch(new strategy.SaveFeature());
    await until(() => !html().includes('id="feature-text"'));
    expect(post).toEqual({ title: 'Export', body: 'CSV and JSON', state: 'proposed', milestone: 'm1', request: expect.any(String) });
    expect(post.request).not.toBe('');
    expect(html()).toContain('Export');
    // Edit: only the changed field is sent.
    dispatch(new strategy.EditFeature('f1'));
    dispatch(new strategy.FeatureState('active'));
    dispatch(new strategy.SaveFeature());
    await until(() => !html().includes('id="feature-text"'));
    expect(patch).toEqual({ state: 'active' });
    expect(html()).toContain('class="state state-active">active<');
    // A refusal shows its error and keeps the form.
    dispatch(new strategy.EditFeature('f1'));
    dispatch(new strategy.FeatureMilestone('m9'));
    dispatch(new strategy.SaveFeature());
    await until(() => html().includes('E7023'));
    expect(patch).toEqual({ milestone: 'm9' });
    expect(html()).toContain('id="feature-text"');
    dispatch(new strategy.CancelFeature());
    // Up moves Tags above Search in "No milestone": one PATCH, to one below its neighbour.
    const order = () => html().indexOf('Tags') < html().indexOf('Search');
    expect(order()).toBe(false);
    // The rank the Up button sends: from the group as the page draws it.
    const loose = features.groups(model.milestones, model.features).toArray().at(-1).features;
    const rank = features.move_rank(loose, 'f3', true)[0];
    dispatch(new strategy.MoveFeature('f3', rank));
    await until(() => order());
    expect(moves).toEqual([{ rank: 0 }]);
    // Start (D-154): a proposed feature's one PATCH to active; the button is gone once it is active.
    expect(html()).toContain('aria-label="Start Tags"');
    expect(html()).not.toContain('aria-label="Start Search"');
    // A refused start says why and changes nothing: its state stays, and so
    // does the order the move above answered.
    refuse = true;
    dispatch(new strategy.StartFeature('f3'));
    await until(() => html().includes('E7003'));
    expect(moves).toEqual([{ rank: 0 }]);
    expect(html()).toContain('aria-label="Start Tags"');
    expect(html()).toContain('class="state state-proposed">proposed<');
    refuse = false;
    dispatch(new strategy.StartFeature('f3'));
    await until(() => !html().includes('aria-label="Start Tags"'));
    expect(moves).toEqual([{ rank: 0 }, { state: 'active' }]);
    // Delete asks first, naming the tasks that lose their link; the events update the page.
    dispatch(new strategy.DeleteFeature('f1'));
    expect(html()).toContain('Delete <em>Search</em>? Its 1 task loses its link to it; the tasks stay.');
    dispatch(new strategy.ConfirmDeleteFeature());
    await until(() => deleted.length === 1);
    dispatch(new strategy.Live(JSON.stringify({ project: '.', task: { ...linked, feature: null } })));
    dispatch(new strategy.Live(JSON.stringify({ removedFeature: 'f1' })));
    expect(strategy.visible(model).toArray().find(pair => pair[1].id === 't1')[1].feature).toBeInstanceOf(None);
    expect(html()).not.toContain('Delete Search');
    expect(html()).toContain('Delete Tags');
    expect(deleted).toEqual(['/features/f1']);
  } finally {
    server.stop(true); globalThis.fetch = fetchOriginal; delete globalThis.EventSource;
  }
});

// The Roadmap: a feature shows only in its target milestone (D-188).
test('Roadmap tab edits milestones, adds and removes features and sets dependencies, showing E7024 and E7025', async () => {
  const task = { body: '', rank: 1, column: 'backlog', paused_from: null, stamps: {}, links: [], runs: [], questions: [], closedTime: null };
  const tasks = {
    t1: { ...task, id: 't1', title: 'Index', feature: 'f1', milestone: 'm1', dependsOn: [] },
    t2: { ...task, id: 't2', title: 'Csv', feature: 'f2', milestone: 'm2', dependsOn: [] },
    t3: { ...task, id: 't3', title: 'Docs', feature: null, milestone: 'm2', dependsOn: [] },
  };
  const milestones = { m1: { id: 'm1', title: 'Launch', body: '', rank: 1, state: 'proposed' }, m2: { id: 'm2', title: 'Later', body: '', rank: 2, targetDate: '2027-06-01', state: 'upcoming' } };
  const records = { f1: { id: 'f1', title: 'Search', body: '', milestone: 'm1', rank: 1, state: 'proposed' }, f2: { id: 'f2', title: 'Export', body: '', milestone: 'm2', rank: 1, state: 'proposed' } };
  const calls = [];
  const server = Bun.serve({ port: 0, fetch: async req => {
    const path = new URL(req.url).pathname;
    const body = ['POST', 'PATCH'].includes(req.method) ? await req.json() : null;
    if (body) calls.push([req.method, path, body]);
    if (path === '/events') return new Response(encode(frame({ milestones: Object.values(milestones) }) +
      frame({ features: Object.values(records) }) + frame({ project: '.', workflow: { columns: [] } }) +
      frame({ project: '.', tasks: Object.values(tasks) }) + 'event: ready\ndata: {}\n\n'), { headers: { 'Content-Type': 'text/event-stream' } });
    if (path === '/projects') return Response.json({ workspace: false, projects: [{ dir: '.', label: 'root', name: 'root', base: '' }], found: [] });
    if (path === '/workflow') return Response.json({ columns: [] });
    if (path === '/milestones' && req.method === 'POST') return Response.json({ id: 'm3', rank: 3, targetDate: null, ...body });
    const [, kind, id] = path.split('/');
    if (kind === 'milestones' && req.method === 'PATCH') return Response.json(milestones[id] = { ...milestones[id], ...body });
    if (kind === 'milestones' && req.method === 'DELETE') { calls.push(['DELETE', path]); return Response.json(milestones[id]); }
    if (kind === 'features' && req.method === 'PATCH') {
      const feature = records[id] = { ...records[id], ...body };
      if ('milestone' in body) for (const t of Object.values(tasks)) if (t.feature === id) t.milestone = body.milestone;
      return Response.json(feature);
    }
    if (kind === 'tasks' && req.method === 'PATCH') {
      const deps = body.dependsOn;
      if (deps.some(d => d === id || !tasks[d])) return Response.json({ code: 'E7024', message: 'E7024 a task can depend only on another existing task; ' + id + ' is not one' }, { status: 400 });
      if (deps.some(d => tasks[d].dependsOn.includes(id))) return Response.json({ code: 'E7025', message: 'E7025 these dependencies would make a cycle' }, { status: 400 });
      return Response.json(tasks[id] = { ...tasks[id], dependsOn: deps });
    }
    return Response.json({ message: 'unexpected ' + path }, { status: 404 });
  }});
  const fetchOriginal = globalThis.fetch;
  globalThis.fetch = (path, options) => fetchOriginal(new URL(path, server.url), options);
  globalThis.EventSource = class {
    listeners = {};
    addEventListener(name, f) { this.listeners[name] = f; }
    close() {}
    constructor(path) {
      fetch(path).then(r => r.text()).then(text => {
        for (const block of text.split('\n\n')) {
          const data = block.split('\n').find(line => line.startsWith('data: '));
          const name = block.split('\n').find(line => line.startsWith('event: '));
          if (name) this.listeners[name.slice(7)]?.({ data: data?.slice(6) });
          else if (data) this.onmessage?.({ data: data.slice(6) });
        }
      }).catch(() => {});
    }
  };
  let model;
  const noop = () => {};
  const dispatch = msg => { let effect; [model, effect] = strategy.update(model, msg); perform(effect, dispatch, noop, noop, noop, noop, noop, noop); };
  const panel = () => {
    const all = to_string(strategy.view(model));
    const start = all.indexOf('id="panel-roadmap"'), end = all.indexOf('id="panel-tasks"');
    return all.slice(start, end > start ? end : undefined);
  };
  const until = async predicate => {
    for (let i = 0; i < 200; i++) { if (predicate()) return; await Bun.sleep(10); }
    throw new Error('UI did not reach expected state: ' + panel());
  };
  const count = text => panel().split(text).length - 1;
  try {
    model = strategy.new$(new strategy.Roadmap());
    dispatch(new strategy.Connected());
    await until(() => panel().includes('Launch'));
    // Milestones start collapsed: the heading shows the state, date and feature count.
    expect(panel().indexOf('Later')).toBeLessThan(panel().indexOf('Launch'));
    expect(panel()).toContain('class="target">2027-06-01<');
    expect(count('1 feature')).toBe(2);
    expect(panel()).toContain('aria-label="Edit Launch"');
    expect(panel()).toContain('aria-label="Delete Launch"');
    expect(panel()).toContain('aria-expanded="false"');
    expect(panel()).not.toContain('Csv');
    expect(panel()).not.toContain('<h4>Search');
    // A heading opens its body, features and tasks; "No milestone" stays open.
    dispatch(new strategy.ToggleMilestone('m1'));
    dispatch(new strategy.ToggleMilestone('m2'));
    await until(() => panel().includes('Csv'));
    expect(panel()).toContain('aria-expanded="true"');
    // Milestones by target date, then undated ones; each feature shows once, under its target.
    expect(count('<h4>Search')).toBe(1);
    expect(count('<h4>Export')).toBe(1);
    // Dependencies are edited one task at a time.
    expect(panel()).not.toContain('class="picker"');
    // Create with a target date: the form has its own textarea and a date input.
    dispatch(new strategy.NewMilestone());
    expect(panel()).toContain('id="milestone-text"');
    expect(panel()).toContain('type="date"');
    expect(panel()).not.toContain('feature-picker');
    dispatch(new strategy.MilestoneTitle('2.0'));
    dispatch(new strategy.MilestoneTargetDate('2027-01-01'));
    dispatch(new strategy.SaveMilestone());
    await until(() => !panel().includes('id="milestone-text"'));
    expect(calls.at(-1)).toEqual(['POST', '/milestones', { title: '2.0', body: '', state: 'proposed', targetDate: '2027-01-01', request: expect.any(String) }]);
    expect(panel()).toContain('class="target">2027-01-01<');
    expect(panel().indexOf('2.0')).toBeLessThan(panel().indexOf('Later'));
    expect(panel().indexOf('Later')).toBeLessThan(panel().indexOf('Launch'));
    // Edit: only the changed field.
    dispatch(new strategy.EditMilestone('m1'));
    // The form lists the milestone's features and searches the other ones.
    expect(panel()).toContain('class="picker"');
    expect(panel()).toContain('aria-label="Remove Search"');
    expect(panel()).toContain('aria-label="Add Export"');
    dispatch(new strategy.MilestoneFeatureQuery('exp'));
    expect(panel()).toContain('aria-label="Add Export"');
    expect(panel()).not.toContain('aria-label="Add Search"');
    dispatch(new strategy.MilestoneFeatureQuery('zzz'));
    expect(panel()).toContain('No feature matches.');
    dispatch(new strategy.MilestoneFeatureQuery(''));
    // Add moves Export from Later, at once and outside Save.
    dispatch(new strategy.TargetFeature('f2', 'm1'));
    await until(() => panel().includes('aria-label="Remove Export"'));
    expect(calls.at(-1)).toEqual(['PATCH', '/features/f2', { milestone: 'm1' }]);
    expect(panel()).toContain('aria-label="Remove Search"');
    expect(panel()).toContain('No other feature to add.');
    // Remove sends null and keeps the form open.
    dispatch(new strategy.TargetFeature('f2', ''));
    await until(() => panel().includes('aria-label="Add Export"'));
    expect(calls.at(-1)).toEqual(['PATCH', '/features/f2', { milestone: null }]);
    expect(panel()).toContain('id="milestone-text"');
    dispatch(new strategy.MilestoneState('active'));
    dispatch(new strategy.SaveMilestone());
    await until(() => !panel().includes('id="milestone-text"'));
    expect(calls.at(-1)).toEqual(['PATCH', '/milestones/m1', { state: 'active' }]);
    // Rank is not a Roadmap control.
    expect(panel()).not.toContain('aria-label="Move up Launch"');
    expect(panel()).not.toContain('aria-label="Move down Launch"');
    expect(calls.filter(c => c[2] && 'rank' in c[2])).toEqual([]);
    // Dependencies: set, then the server's refusals show.
    dispatch(new strategy.SetDepends('.', 't2', toList(['t1'])));
    await until(() => panel().includes('depends on Index'));
    expect(calls.at(-1)).toEqual(['PATCH', '/tasks/t2', { dependsOn: ['t1'] }]);
    // Any task of the root is offered, in another milestone too; Csv would make a cycle, so it isn't.
    dispatch(new strategy.OpenDepends('.', 't1'));
    expect(count('class="picker"')).toBe(1);
    expect(panel()).toContain('aria-label="Depend on Docs"');
    expect(panel()).not.toContain('aria-label="Depend on Csv"');
    dispatch(new strategy.DependsQuery('csv'));
    expect(panel()).toContain('No task matches.');
    dispatch(new strategy.SetDepends('.', 't1', toList(['t3'])));
    await until(() => panel().includes('aria-label="Remove Docs"'));
    expect(calls.at(-1)).toEqual(['PATCH', '/tasks/t1', { dependsOn: ['t3'] }]);
    expect(panel()).toContain('depends on Docs');
    dispatch(new strategy.SetDepends('.', 't1', toList(['t2'])));
    await until(() => panel().includes('E7025 these dependencies would make a cycle'));
    dispatch(new strategy.SetDepends('.', 't1', toList(['t1'])));
    await until(() => panel().includes('E7024 a task can depend only on another existing task'));
    // Delete asks first, naming what loses its link.
    dispatch(new strategy.DeleteMilestone('m1'));
    expect(panel()).toContain('Delete <em>Launch</em>? Its 1 feature and 1 task lose their link to it; they stay.');
    dispatch(new strategy.ConfirmDeleteMilestone());
    await until(() => calls.at(-1)[0] === 'DELETE');
    expect(calls.at(-1)).toEqual(['DELETE', '/milestones/m1']);
    dispatch(new strategy.Live(JSON.stringify({ removedMilestone: 'm1' })));
    expect(panel()).not.toContain('Delete Launch');
  } finally {
    server.stop(true); globalThis.fetch = fetchOriginal; delete globalThis.EventSource;
  }
});

// A new task for a feature: POST to the picked project, then one PATCH for the link; each step reports its own failure.
test('Features tab adds a task to a feature, then links it; a refused link says the task exists without it', async () => {
  const calls = [];
  let refuse = false, n = 0;
  const blank = { body: '', rank: 1, column: 'backlog', paused_from: null, stamps: {}, links: [], runs: [], questions: [], feature: null, milestone: null, dependsOn: [], closedTime: null };
  const server = Bun.serve({ port: 0, fetch: async req => {
    const path = new URL(req.url).pathname;
    if (path === '/events') return new Response(encode(frame({ features: [{ id: 'f1', title: 'Search', body: '', milestone: null, rank: 1, state: 'active' }] }) +
      frame({ project: '.', workflow: { columns: [] } }) + frame({ project: 'web', workflow: { columns: [] } }) + 'event: ready\ndata: {}\n\n'), { headers: { 'Content-Type': 'text/event-stream' } });
    if (path === '/projects') return Response.json({ workspace: true, projects: [{ dir: '.', label: 'root', name: 'root', base: '' }, { dir: 'web', label: 'web', name: 'web', base: '/p/web' }], found: [] });
    if (path.endsWith('/workflow')) return Response.json({ columns: [] });
    if (req.method === 'POST' && path === '/p/web/tasks') {
      const body = await req.json(); calls.push(['POST', path, body]);
      return Response.json({ ...blank, id: 't' + ++n, title: body.title });
    }
    if (req.method === 'PATCH' && path.startsWith('/p/web/tasks/')) {
      const body = await req.json(); calls.push(['PATCH', path, body]);
      if (refuse) return Response.json({ code: 'E7023', message: 'E7023 unknown feature f1' }, { status: 400 });
      return Response.json({ ...blank, id: path.split('/').at(-1), title: 'Index', ...body });
    }
    return Response.json({ message: 'unexpected ' + path }, { status: 404 });
  }});
  const fetchOriginal = globalThis.fetch;
  globalThis.fetch = (path, options) => fetchOriginal(new URL(path, server.url), options);
  globalThis.EventSource = class {
    listeners = {};
    addEventListener(name, f) { this.listeners[name] = f; }
    close() {}
    constructor(path) {
      fetch(path).then(r => r.text()).then(text => {
        for (const block of text.split('\n\n')) {
          const data = block.split('\n').find(line => line.startsWith('data: '));
          const name = block.split('\n').find(line => line.startsWith('event: '));
          if (name) this.listeners[name.slice(7)]?.({ data: data?.slice(6) });
          else if (data) this.onmessage?.({ data: data.slice(6) });
        }
      }).catch(() => {});
    }
  };
  let model;
  const noop = () => {};
  const dispatch = msg => { let effect; [model, effect] = strategy.update(model, msg); perform(effect, dispatch, noop, noop, noop, noop, noop, noop); };
  const html = () => to_string(strategy.view(model));
  const until = async predicate => {
    for (let i = 0; i < 200; i++) { if (predicate()) return; await Bun.sleep(10); }
    throw new Error('UI did not reach expected state: ' + html());
  };
  try {
    model = strategy.new$(new strategy.Features());
    dispatch(new strategy.Connected());
    // The new-task form shows only on the focused feature's card.
    dispatch(new strategy.FocusFeature('f1'));
    await until(() => html().includes('Search') && html().includes('name="project"'));
    // In a workspace the picker defaults to the root's project.
    expect(html()).toContain('<option selected value=".">root</option>');
    dispatch(new strategy.TaskProject('web'));
    dispatch(new strategy.TaskTitle('f1', 'Index'));
    dispatch(new strategy.AddTask('f1'));
    await until(() => calls.length === 2 && html().includes('class="open">Index<'));
    expect(calls[0]).toEqual(['POST', '/p/web/tasks', { title: 'Index', body: '', request: expect.any(String) }]);
    expect(calls[1]).toEqual(['PATCH', '/p/web/tasks/t1', { feature: 'f1' }]);
    expect(model.task_titles.toArray()).toEqual([]);
    // The link is refused: the task was made, and the feature says it has no link.
    refuse = true;
    dispatch(new strategy.TaskTitle('f1', 'Ranking'));
    dispatch(new strategy.AddTask('f1'));
    await until(() => calls.filter(c => c[0] === 'PATCH').length === 2);
    await until(() => html().includes('created without the link'));
    expect(calls.filter(c => c[0] === 'POST').length).toBe(2);
    expect(calls.at(-1)).toEqual(['PATCH', '/p/web/tasks/t2', { feature: 'f1' }]);
    expect(html()).toContain('The task was created without the link to this feature: E7023 unknown feature f1');
  } finally {
    server.stop(true); globalThis.fetch = fetchOriginal; delete globalThis.EventSource;
  }
});

// The Tasks tab's own new task: POST to the picked project, then one PATCH with the feature (whose milestone it
// takes) or the milestone; with none picked no PATCH goes out. Either way the form closes and the task opens below.
test('Tasks tab creates a task, links its feature or milestone, then opens it', async () => {
  const calls = [];
  let n = 0;
  const blank = { body: '', rank: 1, column: 'backlog', paused_from: null, stamps: {}, links: [], runs: [], questions: [], feature: null, milestone: null, dependsOn: [], closedTime: null };
  const server = Bun.serve({ port: 0, fetch: async req => {
    const path = new URL(req.url).pathname;
    if (path === '/events') return new Response(encode(frame({ features: [{ id: 'f1', title: 'Search', body: '', milestone: null, rank: 1, state: 'active' }] }) +
      frame({ project: '.', workflow: { columns: [] } }) + frame({ project: 'web', workflow: { columns: [] } }) + 'event: ready\ndata: {}\n\n'), { headers: { 'Content-Type': 'text/event-stream' } });
    if (path === '/projects') return Response.json({ workspace: true, projects: [{ dir: '.', label: 'root', name: 'root', base: '' }, { dir: 'web', label: 'web', name: 'web', base: '/p/web' }], found: [] });
    if (path.endsWith('/workflow')) return Response.json({ columns: [] });
    if (req.method === 'POST' && path === '/p/web/tasks') {
      const body = await req.json(); calls.push(['POST', path, body]);
      return Response.json({ ...blank, id: 't' + ++n, title: body.title, body: body.body });
    }
    if (req.method === 'PATCH' && path.startsWith('/p/web/tasks/')) {
      const body = await req.json(); calls.push(['PATCH', path, body]);
      return Response.json({ ...blank, id: path.split('/').at(-1), title: 'Index', ...body });
    }
    return Response.json({ message: 'unexpected ' + path }, { status: 404 });
  }});
  const fetchOriginal = globalThis.fetch;
  globalThis.fetch = (path, options) => fetchOriginal(new URL(path, server.url), options);
  globalThis.EventSource = class {
    listeners = {};
    addEventListener(name, f) { this.listeners[name] = f; }
    close() {}
    constructor(path) {
      fetch(path).then(r => r.text()).then(text => {
        for (const block of text.split('\n\n')) {
          const data = block.split('\n').find(line => line.startsWith('data: '));
          const name = block.split('\n').find(line => line.startsWith('event: '));
          if (name) this.listeners[name.slice(7)]?.({ data: data?.slice(6) });
          else if (data) this.onmessage?.({ data: data.slice(6) });
        }
      }).catch(() => {});
    }
  };
  let model;
  const noop = () => {};
  const dispatch = msg => { let effect; [model, effect] = strategy.update(model, msg); perform(effect, dispatch, noop, noop, noop, noop, noop, noop); };
  const html = () => to_string(strategy.view(model));
  const until = async predicate => {
    for (let i = 0; i < 200; i++) { if (predicate()) return; await Bun.sleep(10); }
    throw new Error('UI did not reach expected state: ' + html());
  };
  try {
    model = strategy.new$(new strategy.Tasks());
    dispatch(new strategy.Connected());
    await until(() => model.workspace && model.features.toArray().length === 1);
    dispatch(new strategy.NewTaskOpen());
    // In a workspace the picker defaults to the root's project; features list with "(none)".
    expect(html()).toContain('<option selected value=".">root</option>');
    expect(html()).toContain('<option value="f1">Search</option>');
    dispatch(new strategy.NewTaskProject('web'));
    dispatch(new strategy.NewTaskTitle('Index'));
    dispatch(new strategy.NewTaskText('Some **text**'));
    dispatch(new strategy.NewTaskMilestone('m1'));
    dispatch(new strategy.NewTaskFeature('f1'));
    // The feature brings its milestone, so the task's own picker hides.
    expect(html()).not.toContain('name="milestone"');
    dispatch(new strategy.CreateTask());
    await until(() => calls.length === 2 && /<solace-task [^>]*task="t1"/.test(html()));
    expect(calls[0]).toEqual(['POST', '/p/web/tasks', { title: 'Index', body: 'Some **text**', request: expect.any(String) }]);
    expect(calls[1]).toEqual(['PATCH', '/p/web/tasks/t1', { feature: 'f1' }]);
    expect(model.new_task instanceof None).toBe(true);
    // A task with no feature keeps a milestone picked.
    dispatch(new strategy.NewTaskOpen());
    dispatch(new strategy.NewTaskProject('web'));
    dispatch(new strategy.NewTaskTitle('Loose'));
    dispatch(new strategy.NewTaskMilestone('m1'));
    dispatch(new strategy.CreateTask());
    await until(() => calls.length === 4 && /<solace-task [^>]*task="t2"/.test(html()));
    expect(calls.at(-1)).toEqual(['PATCH', '/p/web/tasks/t2', { milestone: 'm1' }]);
    // Nothing picked: the create alone, and the task opens.
    dispatch(new strategy.NewTaskOpen());
    dispatch(new strategy.NewTaskProject('web'));
    dispatch(new strategy.NewTaskTitle('Plain'));
    dispatch(new strategy.CreateTask());
    await until(() => /<solace-task [^>]*task="t3"/.test(html()));
    await Bun.sleep(30);
    expect(calls.map(c => c[0])).toEqual(['POST', 'PATCH', 'POST', 'PATCH', 'POST']);
    expect(model.new_task instanceof None).toBe(true);
  } finally {
    server.stop(true); globalThis.fetch = fetchOriginal; delete globalThis.EventSource;
  }
});

// D-93: the planning panel starts a run on the selected task, follows it live, answers its proposal and ends it.
test('Planning panel starts a run on the selection, answers a proposal, keeps across tabs and reopens past runs', async () => {
  const calls = [];
  const requests = [];
  const blank = { body: '', rank: 1, column: 'backlog', paused_from: null, stamps: {}, links: [], runs: [], questions: [], feature: null, milestone: null, dependsOn: [], closedTime: null };
  const onTask = { kind: 'task', id: 't1' };
  const past = { id: 'r0', kind: 'strategy', model: 'fake', state: 'ended', outcome: 'planned', started: 100, context: onTask, notes: [{ author: 'solace', text: 'An earlier plan' }], questions: [] };
  const proposal = { id: 'q1', text: 'Create task "Ranking"', options: ['Approve', 'Reject'], answer: null,
    proposal: { kind: 'task', fields: { title: 'Ranking', feature: 'f1' }, reopens: [{ kind: 'feature', id: 'f1' }] } };
  let r1, events;
  const server = Bun.serve({ port: 0, fetch: async req => {
    const path = new URL(req.url).pathname;
    requests.push([req.method, path]);
    const body = req.method === 'POST' ? await req.json() : null;
    if (body) calls.push([path, body]);
    if (path === '/events') return new Response(new ReadableStream({ start(c) { events = c; c.enqueue(encode(
      frame({ project: '.', workflow: { columns: [] } }) + frame({ project: '.', tasks: [{ ...blank, id: 't1', title: 'Index' }] }) +
      frame({ features: [{ id: 'f1', title: 'Search', body: '', milestone: null, rank: 1, state: 'complete' }] }) +
      frame({ strategyRuns: [{ ...past, notes: undefined }] }) + 'event: ready\ndata: {}\n\n')); } }), { headers: { 'Content-Type': 'text/event-stream' } });
    if (path === '/projects') return Response.json({ workspace: false, projects: [{ dir: '.', label: 'root', name: 'root', base: '' }], found: [] });
    if (path === '/workflow') return Response.json({ columns: [] });
    if (path === '/models') return Response.json([{ id: 'fake', label: 'Fake' }, { id: 'big', label: 'Big', available: false, reason: 'no key' }]);
    if (path === '/strategy/runs' && req.method === 'POST') {
      r1 = { id: 'r1', kind: 'strategy', model: body.model, state: 'running', started: 200, context: body.context, notes: [{ author: 'human', text: body.text }], questions: [] };
      return Response.json(r1);
    }
    if (path === '/strategy/runs/r1' && req.method === 'GET') return Response.json({ ...r1, transcript: [{ kind: 'message', text: 'Looking at Index' }, { kind: 'tool', title: 'strategy list', status: 'ok', output: '[]' }] });
    if (path === '/strategy/runs/r0' && req.method === 'GET') return Response.json({ ...past, transcript: [] });
    if (path === '/strategy/runs/r1/questions/q1') {
      r1 = { ...r1, state: 'running', questions: [{ ...proposal, answer: 'Created task t2' }] };
      return Response.json(r1);
    }
    if (path === '/strategy/runs/r1/notes') return Response.json(r1 = { ...r1, notes: [...r1.notes, { author: 'human', text: body.text }] });
    if (path === '/strategy/runs/r1/end') return Response.json(r1 = { ...r1, state: 'ended', outcome: 'planned', notes: [{ author: 'solace', text: 'The conversation' }] });
    return Response.json({ message: 'unexpected ' + path }, { status: 404 });
  }});
  const fetchOriginal = globalThis.fetch;
  const controllers = [];
  globalThis.fetch = (path, options) => fetchOriginal(new URL(path, server.url), options);
  globalThis.EventSource = class {
    listeners = {};
    addEventListener(name, f) { this.listeners[name] = f; }
    close() {}
    constructor(path) {
      const controller = new AbortController(); controllers.push(controller);
      fetch(path, { signal: controller.signal }).then(async response => {
        let text = '';
        for await (const chunk of response.body) {
          text += new TextDecoder().decode(chunk);
          while (text.includes('\n\n')) {
            const at = text.indexOf('\n\n'); const block = text.slice(0, at); text = text.slice(at + 2);
            const data = block.split('\n').find(line => line.startsWith('data: '));
            const name = block.split('\n').find(line => line.startsWith('event: '));
            if (name) this.listeners[name.slice(7)]?.({ data: data?.slice(6) });
            else if (data) this.onmessage?.({ data: data.slice(6) });
          }
        }
      }).catch(() => {});
    }
  };
  let model;
  const noop = () => {};
  const dispatch = msg => { let effect; [model, effect] = strategy.update(model, msg); perform(effect, dispatch, noop, noop, noop, noop, noop, noop); };
  const html = () => to_string(strategy.view(model));
  // The panel alone: from the aside to the first tab panel.
  const panel = () => { const all = html(); return all.slice(all.indexOf('<aside'), all.indexOf('</aside>')); };
  // The plan bar's context line, so a failed context check prints it instead of the whole panel.
  const context = () => {
    const p = panel();
    const at = p.indexOf('class="plan-context"');
    return at < 0 ? p.slice(0, 300) : p.slice(at, p.indexOf('</p>', at) + 4);
  };
  const models = () => requests.filter(([method, path]) => method === 'GET' && path === '/models').length;
  const until = async predicate => {
    for (let i = 0; i < 200; i++) { if (predicate()) return; await Bun.sleep(10); }
    throw new Error('UI did not reach expected state: ' + panel());
  };
  try {
    model = strategy.new$(new strategy.Tasks());
    dispatch(new strategy.Connected());
    await until(() => html().includes('class="open">Index<') && panel().includes('Fake') && events && models() >= 2);
    expect(models()).toBe(2);
    // Unavailable models aren't offered; the line under the pick says why.
    expect(panel()).not.toContain('value="big"');
    expect(panel()).toContain('<summary>1 more model unavailable</summary><ul><li>Big: no key</li></ul>');
    // Nothing selected: the whole project. Selecting a task leaves it there (D-202), and lists its past run.
    expect(context()).toContain('Plan context: Whole Project');
    dispatch(new strategy.Open('.', 't1'));
    expect(context()).toContain('Plan context: Whole Project');
    expect(panel()).toContain('aria-label="Earlier runs"');
    expect(panel()).not.toContain('Runs on this');
    expect(panel()).toContain('Ended: planned · Fake');
    dispatch(new strategy.PlanText('Plan the index'));
    dispatch(new strategy.StartPlan());
    await until(() => panel().includes('Looking at Index'));
    expect(calls[0]).toEqual(['/strategy/runs', { model: 'fake', text: 'Plan the index', context: { kind: 'page' } }]);
    expect(panel()).toContain('Working · Fake');
    expect(panel()).toContain('strategy list');
    // A page-context run does not mark the task row (D-202).
    expect(html()).not.toMatch(/class="open">Index<\/button><span class="planning"/);
    // The agent replies and proposes, live.
    r1 = { ...r1, state: 'waiting', notes: [...r1.notes, { author: 'agent', text: 'Shall I add a ranking task?' }], questions: [proposal] };
    requests.length = 0;
    events.enqueue(encode(frame({ strategyRun: r1 }, 1)));
    await until(() => panel().includes('Shall I add a ranking task?') && models() === 1);
    // A task update refreshes the model list too.
    requests.length = 0;
    events.enqueue(encode(frame({ project: '.', task: { ...blank, id: 't1', title: 'Index' } }, 2)));
    await until(() => models() === 1);
    expect(models()).toBe(1);
    expect(panel()).toContain('Create task<em> Ranking</em>');
    expect(panel()).toContain('Approving also sets the feature <a data-feature="f1" href="/feature/f1">Search</a> back to upcoming.');
    // Another tab: the panel and its run stay.
    dispatch(new strategy.Select(new strategy.Features()));
    expect(panel()).toContain('Shall I add a ranking task?');
    expect(context()).toContain('Plan context: Whole Project');
    dispatch(new strategy.AnswerPlan('q1', 'Approve'));
    await until(() => panel().includes('Answer: Created task t2'));
    expect(calls.at(-1)).toEqual(['/strategy/runs/r1/questions/q1', { option: 'Approve' }]);
    expect(panel()).not.toContain('>Approve</button>');
    dispatch(new strategy.PlanText('Thanks'));
    dispatch(new strategy.SendPlan());
    await until(() => calls.length === 3 && model.plan_text === '');
    expect(calls.at(-1)).toEqual(['/strategy/runs/r1/notes', { text: 'Thanks' }]);
    dispatch(new strategy.EndPlan());
    await until(() => panel().includes('Ended: planned · Fake') && panel().includes('The conversation'));
    expect(calls.at(-1)).toEqual(['/strategy/runs/r1/end', {}]);
    expect(panel()).not.toContain('id="plan-text"');
    expect(html()).not.toContain('class="planning"');
    // Back on the task: both runs under Earlier runs, newest first; the past
    // one reopens read-only, selected in the menu.
    dispatch(new strategy.Select(new strategy.Tasks()));
    dispatch(new strategy.ShowPlan(''));
    const runs = panel().slice(panel().indexOf('aria-label="Earlier runs"'), panel().indexOf('</select>'));
    expect(runs.indexOf('Ended: planned')).toBeGreaterThan(-1);
    expect(runs.split('<option value=').length - 1).toBe(2);
    expect(runs.indexOf('value="r1"')).toBeLessThan(runs.indexOf('value="r0"'));
    dispatch(new strategy.ShowPlan('r0'));
    await until(() => panel().includes('An earlier plan'));
    expect(panel()).not.toContain('id="plan-text"');
    expect(panel()).toContain('<option selected value="r0">');
    expect(panel()).toContain('>New run</button>');
    // The page refreshes its models once catch-up replays what it missed.
    requests.length = 0;
    dispatch(new strategy.CaughtUp());
    await until(() => models() === 1);
    expect(models()).toBe(1);
  } finally {
    controllers.forEach(c => c.abort()); server.stop(true); globalThis.fetch = fetchOriginal; delete globalThis.EventSource;
  }
});

// The planning panel's width is kept in the browser and read on load; blocked storage leaves the default.
test('Planning panel width is stored after a drag or a key and read back on load', () => {
  const key = 'solace-strategy-plan-width';
  const original = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  const values = new Map();
  const noop = () => {};
  const step = (model, msg) => { const [next, effect] = strategy.update(model, msg); perform(effect, noop, noop, noop, noop, noop, noop, noop); return next; };
  try {
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('blocked'); } });
    expect(strategy.init(undefined)[0].plan_width).toBe(380);
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: { getItem: k => values.get(k) ?? null, setItem: (k, v) => values.set(k, v), removeItem: k => values.delete(k) } });
    expect(strategy.init(undefined)[0].plan_width).toBe(380);
    values.set(key, 'wide');
    expect(strategy.init(undefined)[0].plan_width).toBe(380);
    values.set(key, '500');
    let model = strategy.init(undefined)[0];
    expect(model.plan_width).toBe(500);
    expect(to_string(strategy.view(model))).toContain('--plan-width:500px');
    expect(to_string(strategy.view(model))).toContain('aria-valuenow="500"');
    // A drag stores its width once it ends.
    model = step(model, new strategy.PlanWidth(450));
    expect(values.get(key)).toBe('500');
    model = step(model, new strategy.PlanWidthDone());
    expect(values.get(key)).toBe('450');
    // A key stores at once; Left widens.
    model = step(model, new strategy.PlanKey('ArrowLeft'));
    expect(values.get(key)).toBe('466');
    expect(strategy.init(undefined)[0].plan_width).toBe(466);
    // Never under the minimum.
    model = step(model, new strategy.PlanWidth(100));
    model = step(model, new strategy.PlanWidthDone());
    expect(values.get(key)).toBe('280');
  } finally {
    if (original) Object.defineProperty(globalThis, 'localStorage', original); else delete globalThis.localStorage;
  }
});

test('transport uses the base it is given; project paths stay absolute', async () => {
  const oldFetch = globalThis.fetch, oldEventSource = globalThis.EventSource;
  const paths = [];
  let source;
  globalThis.fetch = async path => { paths.push(path); return new Response('{}'); };
  globalThis.EventSource = class {
    constructor(path) { this.path = path; source = this; }
    addEventListener() {}
    close() { this.closed = true; }
  };
  try {
    const api = await import('../src/api.mjs');
    await new Promise(resolve => api.request('/p/shop', '/projects', 'GET', '', resolve));
    await new Promise(resolve => api.request('/p/shop', '/p/other/tasks/t1', 'PATCH', '{}', resolve));
    await new Promise(resolve => api.request('', '/projects', 'GET', '', resolve));
    expect(paths).toEqual(['/p/shop/projects', '/p/other/tasks/t1', '/projects']);
    expect(api.events('/p/shop', () => {})).toBe(source);
    expect(source.path).toBe('/p/shop/events');
    api.close_events(source);
    expect(source.closed).toBe(true);
  } finally {
    globalThis.fetch = oldFetch; globalThis.EventSource = oldEventSource;
  }
});

test('Personas tab lists, creates, edits over a revision and deletes', async () => {
  let persona = { id: 'p1', name: 'Ada', objectives: 'Ship it', tools: 'Code', concerns: 'Risk', other: 'More', revision: 'r1' };
  const calls = [];
  const server = Bun.serve({ port: 0, fetch: async req => {
    const path = new URL(req.url).pathname;
    const body = ['POST', 'PUT'].includes(req.method) ? await req.json() : null;
    if (body) calls.push([req.method, path, body]);
    if (path === '/events') return new Response(encode(
      frame({ personas: [persona] }) + 'event: ready\ndata: {}\n\n'), { headers: { 'Content-Type': 'text/event-stream' } });
    if (path === '/projects') return Response.json({ workspace: false, projects: [{ dir: '.', label: 'root', name: 'root', base: '' }], found: [] });
    if (path === '/workflow') return Response.json({ columns: [] });
    if (path === '/personas' && req.method === 'POST') {
      persona = { id: 'p2', ...body, revision: 'r1' };
      return Response.json(persona);
    }
    if (path === '/personas/p1' && req.method === 'PUT') {
      if (body.base !== persona.revision) return Response.json({ code: 'E7028', message: 'E7028 the persona changed since this edit began' }, { status: 409 });
      persona = { id: 'p1', ...body, revision: 'r2' };
      return Response.json(persona);
    }
    if (path === '/personas/p1' && req.method === 'DELETE') { calls.push(['DELETE', path]); return Response.json(persona); }
    return Response.json({ message: 'unexpected ' + path }, { status: 404 });
  }});
  const fetchOriginal = globalThis.fetch;
  globalThis.fetch = (path, options) => fetchOriginal(new URL(path, server.url), options);
  globalThis.EventSource = class {
    listeners = {};
    addEventListener(name, f) { this.listeners[name] = f; }
    close() {}
    constructor(path) {
      fetch(path).then(r => r.text()).then(text => {
        for (const block of text.split('\n\n')) {
          const data = block.split('\n').find(line => line.startsWith('data: '));
          const name = block.split('\n').find(line => line.startsWith('event: '));
          if (name) this.listeners[name.slice(7)]?.({ data: data?.slice(6) });
          else if (data) this.onmessage?.({ data: data.slice(6) });
        }
      }).catch(() => {});
    }
  };
  let model;
  const noop = () => {};
  const dispatch = msg => { let effect; [model, effect] = strategy.update(model, msg); perform(effect, dispatch, noop, noop, noop, noop, noop, noop); };
  const html = () => to_string(strategy.view(model));
  const until = async predicate => {
    for (let i = 0; i < 200; i++) { if (predicate()) return; await Bun.sleep(10); }
    throw new Error('UI did not reach expected state: ' + html());
  };
  try {
    model = strategy.new$(new strategy.Personas());
    dispatch(new strategy.Connected());
    await until(() => html().includes('Ada'));
    // A new persona form has the five hints, and its save sends every field.
    dispatch(new strategy.NewPersona());
    expect(html()).toContain('placeholder="Name"');
    expect(html()).toContain('placeholder="What does this persona want to accomplish?"');
    dispatch(new strategy.PersonaName('Grace'));
    dispatch(new strategy.PersonaText(new personas.ObjectivesField(), 'Lead the team'));
    dispatch(new strategy.SavePersona());
    await until(() => !html().includes('id="persona-form"'));
    expect(calls.at(-1)).toEqual(['POST', '/personas', { name: 'Grace', objectives: 'Lead the team', tools: '', concerns: '', other: '' }]);
    expect(html()).toContain('Grace');
    // An edit names the revision it began from; a refusal keeps the draft.
    dispatch(new strategy.EditPersona('p1'));
    dispatch(new strategy.PersonaName('Ada Lovelace'));
    dispatch(new strategy.SavePersona());
    await until(() => !html().includes('id="persona-form"'));
    expect(calls.at(-1)).toEqual(['PUT', '/personas/p1', { name: 'Ada Lovelace', objectives: 'Ship it', tools: 'Code', concerns: 'Risk', other: 'More', base: 'r1' }]);
    // A live change while the form is open makes the draft stale, and Reload takes the saved one.
    persona = { ...persona, name: 'Someone else', revision: 'r3' };
    dispatch(new strategy.Live(JSON.stringify({ personas: [persona] })));
    await until(() => model.persona_state.personas.toArray()[0].revision === 'r3');
    dispatch(new strategy.EditPersona('p1'));
    persona = { ...persona, name: 'Theirs', revision: 'r4' };
    dispatch(new strategy.Live(JSON.stringify({ personas: [persona] })));
    await until(() => model.persona_state.personas.toArray()[0].revision === 'r4');
    expect(html()).toContain('changed since you began editing');
    dispatch(new strategy.SavePersona());
    await until(() => html().includes('E7028') && calls.at(-1)[0] === 'PUT');
    expect(calls.at(-1)[2].base).toBe('r3');
    expect(html()).toContain('id="persona-form"');
    dispatch(new strategy.ReloadPersona());
    expect(model.persona_state.draft[0].name).toBe('Theirs');
    // Delete asks first, then one DELETE; the reply removes it.
    dispatch(new strategy.CancelPersona());
    dispatch(new strategy.DeletePersona('p1'));
    expect(html()).toContain('Delete <em>Theirs</em>?');
    dispatch(new strategy.ConfirmDeletePersona());
    await until(() => calls.some(call => call[0] === 'DELETE'));
    await until(() => !html().includes('Theirs'));
  } finally {
    server.stop(true); globalThis.fetch = fetchOriginal; delete globalThis.EventSource;
  }
});

test('the strategy draft storage stub matches the components copy', async () => {
  const local = await readFile(new URL('./draft_storage.mjs', import.meta.url), 'utf8');
  const components = await readFile(new URL('../../components/test/draft_storage.mjs', import.meta.url), 'utf8');
  expect(local.trimEnd()).toBe(components.trimEnd());
});
