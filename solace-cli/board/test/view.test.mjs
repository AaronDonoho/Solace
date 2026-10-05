import { test, expect } from 'bun:test';
import * as view from '../build/dev/javascript/solace_board/solace_view.mjs';
import { perform } from '../build/dev/javascript/lustre/lustre/effect.mjs';
import { to_string } from '../build/dev/javascript/lustre/lustre/element.mjs';

test('contract view renders visible SVG nodes and uses the view API', async () => {
  const oldLocation = globalThis.location;
  const oldFetch = globalThis.fetch;
  const oldEvents = globalThis.EventSource;
  globalThis.location = { pathname: '/view', search: '?task=t1' };
  const requests = [];
  const nodes = [
    { id: 'fn:app.main', kind: 'entry', name: 'main', module: 'app', summary: 'Entries about one subject, oldest first, with every matching event and its recorded description', signature: 'pub fn main(entries: List(Entry), subject: Subject) -> List(Entry)', known: 'derived', tests: 0, changed: true },
    { id: 'fn:app.work', kind: 'logic', name: 'work', module: 'app', summary: 'Does work', signature: 'work() -> Nil', known: 'derived', tests: 1 },
    { id: 'effect:prints', kind: 'effect', name: 'prints', module: '', summary: '', signature: '', known: 'derived', tests: 0 },
  ];
  const edges = [{ from: 'fn:app.main', to: 'fn:app.work', kind: 'calls', direct: true, known: 'derived' }, { from: 'fn:app.main', to: 'effect:prints', kind: 'uses' }];
  let review = { summary: '1 effect added, tests not run yet', entries: [], signatures: [], tests: [{ id: 'fn:app.work', name: 'work_test', result: 'not run yet', checks: [{ call: 'app.main()', expected: '3', was: '2' }] }], files: [], effects: { added: ['effect:prints'], removed: [] } };
  globalThis.fetch = async (path, opts) => {
    requests.push([opts.method, path.split('?')[0]]);
    if (path.startsWith('/graph')) return new Response(JSON.stringify({ nodes, edges, review, project: { name: 'pilot_large', fingerprint: '0123456789abcdef0123456789abcdef' } }));
    if (path.startsWith('/nodes')) return new Response(JSON.stringify({ ...nodes[0], tests: ['fn:app.work_test'], ...(path.includes('source=1') ? { source: 'pub fn main() { Nil }', diff: [['-', 'pub fn main() { 1 }'], ['+', 'pub fn main() { Nil }']] } : {}) }));
    return new Response(JSON.stringify({ state: 'running' }));
  };
  let source;
  globalThis.EventSource = class { constructor(path) { requests.push(['GET', path]); source = this; } addEventListener() {} };
  let model;
  const dispatch = msg => { let effect; [model, effect] = view.update(model, msg); perform(effect, dispatch, () => {}, () => {}, () => {}, () => {}, () => {}, () => {}); };
  try {
    let effect; [model, effect] = view.init(); perform(effect, dispatch, () => {}, () => {}, () => {}, () => {}, () => {}, () => {});
    dispatch(new view.Connected());
    for (let i = 0; i < 50 && !Array.from(model.graph.nodes).length; i++) await Bun.sleep(5);
    expect(Array.from(model.graph.nodes)).toHaveLength(3);
    expect((to_string(view.view(model)).match(/class="view-node /g) || []).length).toBe(2);
    expect(requests).toContainEqual(['POST', '/tests']);
    review = { ...review, summary: '1 effect added, 0 test unfinished', tests: [{ id: 'fn:app.work', name: 'work_test', result: 'passing', checks: [{ call: 'app.main()', expected: '3', was: '2' }] }] };
    source.onmessage({ data: JSON.stringify({ task: { id: 't1' } }) });
    for (let i = 0; i < 50 && model.graph.review[0]?.tests.head.result !== 'passing'; i++) await Bun.sleep(5);
    expect(model.graph.review[0].tests.head.result).toBe('passing');
    const reviewHtml = to_string(view.view(model));
    expect(reviewHtml).toContain('Revision review');
    expect(reviewHtml).toMatch(/aria-selected="true"[^>]*>Review<\/button>/);
    expect(reviewHtml).toContain('class="view-node entry changed"');
    expect(reviewHtml).not.toContain('view-cloud');
    expect(reviewHtml).not.toContain('view-titleblock');
    expect(reviewHtml).toContain('REV B');
    expect(reviewHtml).toContain('REV A');
    expect(reviewHtml).toContain('→ 3, was 2');
    expect(reviewHtml).toContain('pilot_large · reviewing task t1');
    // Collapsed main shows how many connections are hidden; connectors read along their arrow.
    expect(reviewHtml).toMatch(/class="view-expander"[^]*?>\+1<\/text>/);
    expect(reviewHtml).toContain('<title>main reaches prints</title>');
    expect(reviewHtml).toContain('points from a function to what it calls, uses or tests');
    const originalPositions = Array.from(model.positions);
    // The click that ends a pan selects nothing.
    dispatch(new view.PanStart(100, 100)); dispatch(new view.PanMove(140, 100)); dispatch(new view.PanEnd());
    dispatch(new view.Click('fn:app.main'));
    expect(model.selected).toBe('');
    // A plain click opens the node without moving the sheet (it's already in view).
    dispatch(new view.PanStart(100, 100)); dispatch(new view.PanEnd());
    const camera = [model.pan_x, model.pan_y, model.zoom];
    dispatch(new view.Click('fn:app.main'));
    expect(model.selected).toBe('fn:app.main');
    expect([model.pan_x, model.pan_y, model.zoom]).toEqual(camera);
    for (let i = 0; i < 50 && model.detail[0]?.tests !== 1; i++) await Bun.sleep(5);
    expect(model.error).toBe('');
    expect(model.detail[0].tests).toBe(1);
    const detailHtml = to_string(view.view(model));
    expect(detailHtml).toMatch(/aria-selected="true"[^>]*>Details<\/button>/);
    expect(detailHtml).toMatch(/>Review<\/button>/);
    expect(detailHtml).toContain('Collapse connections');
    expect(detailHtml).toContain('class="view-node entry changed selected"');
    expect(detailHtml).toMatch(/class="view-expander"[^]*?>−<\/text>/);
    expect(detailHtml).toContain('<title>main calls work</title>');
    expect(detailHtml).toContain('class="view-panel-grip"');
    // Expand all draws every node and connection; collapse all returns to the overview.
    dispatch(new view.ExpandAll());
    expect(Array.from(model.visible).sort()).toEqual(['effect:prints', 'fn:app.main', 'fn:app.work']);
    expect(to_string(view.view(model))).not.toMatch(/>\+\d*<\/text>/);
    dispatch(new view.CollapseAll());
    expect(Array.from(model.expanded)).toEqual([]);
    dispatch(new view.Select('fn:app.main'));
    expect(detailHtml).toContain('Show source');
    expect(detailHtml).toContain('<title>entry: main. ' + nodes[0].summary);
    expect(detailHtml).toContain('aria-label="entry: main. ' + nodes[0].summary);
    expect(detailHtml).toContain('class="view-summary">' + nodes[0].summary);
    // The signature is in the panel, not on the box; no kind word on boxes or connection rows (D-55).
    expect(detailHtml).toContain('class="view-code">pub fn main(entries: List(Entry), subject: Subject) -&gt; List(Entry)');
    expect(detailHtml).not.toContain('view-signature');
    expect(detailHtml).not.toMatch(/class="view-kind"/);
    expect(detailHtml).toContain('<summary>Calls (1)</summary><button class="view-row">work<small>Does work</small>');
    expect(detailHtml).not.toContain('logic · ');
    expect((detailHtml.match(/class="view-node /g) || []).length).toBe(3);
    const wires = detailHtml.match(/<path class="view-wire"[^>]+>/g).filter(path => path.includes('marker-end'));
    expect(wires).toHaveLength(2);
    for (const wire of wires) {
      expect(wire).toMatch(/d="M [^"]+ H [^"]+ V [^"]+ H [^"]+"/);
      expect(wire).toContain('marker-end="url(#arrow)"');
      expect(wire).toContain('stroke-dasharray="6 5"');
    }
    // The opened node stays put; nodes beyond it slide over to make room but keep their order.
    const now = id => Array.from(model.positions).find(p => p.id === id);
    expect(now('fn:app.main')).toEqual(originalPositions.find(p => p.id === 'fn:app.main'));
    const order = list => [...list].sort((p, q) => p.x - q.x || p.y - q.y).map(p => p.id);
    expect(order(originalPositions.map(p => now(p.id)))).toEqual(order(originalPositions));
    dispatch(new view.SelectSource('fn:app.main'));
    for (let i = 0; i < 50 && !model.source; i++) await Bun.sleep(5);
    expect(model.source).toContain('pub fn main');
    const sourceHtml = to_string(view.view(model));
    expect(sourceHtml).toContain('<span class="removed">− pub fn main() { 1 }');
    expect(sourceHtml).toContain('<span class="added">+ pub fn main() { Nil }');
    expect(requests.some(([_, path]) => path === '/nodes/fn:app.main')).toBe(true);
    // Keys: C collapses the focused node, Space expands it again; arrows move focus; Ctrl+Up zooms.
    dispatch(new view.Focused('fn:app.main'));
    dispatch(new view.Key('collapse'));
    expect(Array.from(model.expanded)).toEqual([]);
    dispatch(new view.Key('expand'));
    expect(Array.from(model.expanded)).toEqual(['fn:app.main']);
    const at = id => Array.from(model.positions).find(p => p.id === id);
    dispatch(new view.Key('right'));
    expect(at(model.focus).x).toBeGreaterThan(at('fn:app.main').x);
    const before = model.zoom;
    dispatch(new view.Key('zoom-in'));
    expect(model.zoom).toBeCloseTo(before * 1.2);
    const panned = model.pan_x;
    dispatch(new view.Key('pan-right'));
    expect(model.pan_x).toBeGreaterThan(panned);
    expect(requests).toContainEqual(['GET', '/graph']);
    expect(requests).toContainEqual(['GET', '/nodes/fn:app.main']);
    expect(requests.every(([_, path]) => ['/graph', '/nodes/fn:app.main', '/events', '/tests'].includes(path))).toBe(true);
  } finally { globalThis.location = oldLocation; globalThis.fetch = oldFetch; globalThis.EventSource = oldEvents; }
});

test('expanded graph keeps stored node coordinates', async () => {
  const { layout } = await import('../build/dev/javascript/solace_board/layout.mjs');
  const oldLocation = globalThis.location;
  const oldStorage = globalThis.localStorage;
  const values = new Map();
  globalThis.location = { pathname: '/view', search: '' };
  globalThis.localStorage = { getItem: key => values.get(key), setItem: (key, value) => values.set(key, value) };
  try {
    const nodes = [{ id: 'entry' }, { id: 'effect' }];
    const first = layout(nodes, [{ from: 'entry', to: 'effect' }]);
    const expanded = layout([...nodes, { id: 'logic' }], [{ from: 'entry', to: 'logic' }, { from: 'logic', to: 'effect' }]);
    expect(expanded.slice(0, 2)).toEqual(first);
    expect(expanded[2]).toHaveProperty('id', 'logic');
  } finally { globalThis.location = oldLocation; globalThis.localStorage = oldStorage; }
});

test('33 isolated entries fill the sheet width at a readable size; extra rows pan', () => {
  const originals = [globalThis.location, globalThis.innerWidth, globalThis.innerHeight];
  try {
    for (const [width, height] of [[1440, 900], [390, 844]]) {
      globalThis.location = { pathname: '/view', search: '' };
      globalThis.innerWidth = width;
      globalThis.innerHeight = height;
      const nodes = ['entry', 'logic', 'test', 'file'].flatMap((kind, i) => Array.from({ length: [33, 205, 98, 6][i] }, (_, n) => ({ id: `${kind}:${n}`, kind, name: `${kind}_${n}` })));
      let [model] = view.init();
      [model] = view.update(model, new view.Loaded(JSON.stringify({ nodes, edges: [], project: { name: 'pilot_large', fingerprint: 'abcdef0123456789' } })));
      const positions = Array.from(model.positions);
      expect(positions).toHaveLength(33);
      // Without a task the view opens on the outline; the drawing is one switch away.
      expect(model.outline).toBe(true);
      [model] = view.update(model, new view.ShowOutline(false));
      expect(new Set(positions.map(p => p.x)).size).toBe(width < 700 ? 1 : Math.floor((width - 48 - 32) / 252));
      const initial = [model.pan_x, model.pan_y, model.zoom];
      const drawingWidth = Math.max(240, width - 48);
      const drawingHeight = Math.max(240, height - (width < 700 ? 248 : 360));
      for (const point of positions) {
        expect((point.x - model.pan_x) * model.zoom).toBeGreaterThanOrEqual(0);
        expect((point.y - model.pan_y) * model.zoom).toBeGreaterThanOrEqual(0);
        expect((point.x + 220 - model.pan_x) * model.zoom).toBeLessThanOrEqual(drawingWidth);
      }
      // Readable, not squeezed: the drawing pans for rows below the fold.
      expect(model.zoom).toBeGreaterThanOrEqual(0.8);
      const html = to_string(view.view(model));
      expect((html.match(/class="view-node /g) || []).length).toBe(33);
      expect(html).toContain('No effects found: nothing here prints, writes files or talks to the network (derived)');
      expect(html).toContain('view-swatch entry');
      expect(html).toContain('view-swatch file');
      expect(html).toContain('Observed / declared');
      expect(html).toContain('view-revision-sample');
      expect(html).toContain('<details class="view-legend-mobile">');
      expect(html).not.toContain('ENTRY  ·  derived');
      [model] = view.update(model, new view.Zoom(1.8));
      [model] = view.update(model, new view.Reset());
      expect([model.pan_x, model.pan_y, model.zoom]).toEqual(initial);
    }
  } finally { [globalThis.location, globalThis.innerWidth, globalThis.innerHeight] = originals; }
});

test('opening a node puts callees right, callers left and tests below it, keeping the grid in order', async () => {
  const { layout } = await import('../build/dev/javascript/solace_board/layout.mjs');
  globalThis.location = { pathname: '/view', search: '?beside' };
  const grid = ['a', 'b', 'c'].map(id => ({ id }));
  const before = Object.fromEntries(layout(grid, [], true).map(p => [p.id, p]));
  const nodes = [...grid, { id: 'callee1' }, { id: 'callee2' }, { id: 'caller' }, { id: 't' }];
  const edges = [
    { from: 'b', to: 'callee1', kind: 'calls' }, { from: 'b', to: 'callee2', kind: 'calls' },
    { from: 'caller', to: 'b', kind: 'calls' }, { from: 't', to: 'b', kind: 'tests' },
  ];
  const at = Object.fromEntries(layout(nodes, edges, false, 'b').map(p => [p.id, p]));
  const b = at.b;
  expect(b).toEqual(before.b);
  expect([at.callee1.x, at.callee1.y]).toEqual([b.x + 252, b.y]);
  expect([at.callee2.x, at.callee2.y]).toEqual([b.x + 252, b.y + 156]);
  expect([at.caller.x, at.caller.y]).toEqual([b.x - 252, b.y]);
  expect([at.t.x, at.t.y]).toEqual([b.x, b.y + 156]);
  expect(at.a.x).toBeLessThan(at.caller.x);
  expect(at.c.x).toBeGreaterThan(at.callee1.x);
});

test('files tab groups files by category and opens one read-only in place of the drawing', async () => {
  const oldLocation = globalThis.location;
  const oldFetch = globalThis.fetch;
  globalThis.location = { pathname: '/view', search: '' };
  const requests = [];
  globalThis.fetch = async path => { requests.push(path); return new Response(JSON.stringify({ path: 'README.md', category: 'watched', text: 'line one\nline two', size: 17, truncated: false })); };
  try {
    let [model] = view.init();
    const files = [['README.md', 'watched'], ['gleam.toml', 'watched'], ['solace.toml', 'protected'], ['logo.png', 'unwatched (binary)'], ['build/', 'unwatched; 495 files']]
      .map(([name, tier]) => ({ id: 'file:' + name, kind: 'file', name, tier, sentence: 'long sentence per file' }));
    [model] = view.update(model, new view.Loaded(JSON.stringify({ nodes: [{ id: 'fn:a.main', kind: 'entry', name: 'main' }, ...files], edges: [] })));
    expect(model.pane).toBe(false);
    let html = to_string(view.view(model));
    expect(html).toContain('class="view-pane-folded"');
    [model] = view.update(model, new view.Tab('files'));
    html = to_string(view.view(model));
    const order = ['<h3>Watched</h3>', 'README.md', 'gleam.toml', '<h3>Protected</h3>', 'solace.toml', '<h3>Unwatched</h3>', 'logo.png', 'build/'].map(text => html.indexOf(text));
    expect(order.every(i => i > 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    expect(html).not.toContain('long sentence per file');
    expect(html).toContain('<small>495 files</small>');
    expect(html).toContain('<small>binary</small>');
    const dispatch = msg => { let effect; [model, effect] = view.update(model, msg); perform(effect, dispatch, () => {}, () => {}, () => {}, () => {}, () => {}, () => {}); };
    dispatch(new view.OpenFile('README.md'));
    for (let i = 0; i < 50 && model.file[0] === undefined; i++) await Bun.sleep(5);
    expect(requests).toEqual(['/files/README.md']);
    html = to_string(view.view(model));
    expect(html).toContain('class="view-reader"');
    expect(html).toContain('<span>line two</span>');
    expect(html).toContain('Watched · read-only');
    expect(html).not.toContain('class="view-diagram"');
    expect(html).toMatch(/aria-current="true"[^>]*>README.md/);
    [model] = view.update(model, new view.CloseFile());
    expect(to_string(view.view(model))).toContain('class="view-outline"');
    [model] = view.update(model, new view.Close());
    expect(model.pane).toBe(false);
  } finally { globalThis.location = oldLocation; globalThis.fetch = oldFetch; }
});

test('outline: areas, requests, a task for missing descriptions, and opening a function on the drawing', async () => {
  const oldLocation = globalThis.location;
  const oldFetch = globalThis.fetch;
  globalThis.location = { pathname: '/view', search: '' };
  const posts = [];
  globalThis.fetch = async (path, opts) => {
    if (opts.method === 'POST') { posts.push([path, JSON.parse(opts.body)]); return new Response(JSON.stringify({ id: 'task1' })); }
    return new Response(JSON.stringify({ id: 'fn:api.handle', kind: 'entry', name: 'handle', examples: [{ call: 'api.handle(db, env, GetOrder(1))', expected: '#(db, ok)' }] }));
  };
  try {
    const entry = (i, module) => ({ id: `fn:${module}.e${i}`, kind: 'entry', name: `e${i}`, module, public: true, summary: i === 1 ? '' : 'Does a thing', takes: ['Store', 'id'], gives: 'Payment or an error', tests: i % 2 });
    const nodes = [
      { id: 'fn:api.handle', kind: 'entry', name: 'handle', module: 'app/api', public: true, summary: 'Handles a request', takes: ['Store', 'Request'], gives: 'Store and Response', tests: 3,
        requests: [{ name: 'Create order', fields: ['customer id'] }, { name: 'Add item', fields: ['order id', 'sku', 'quantity'] }] },
      ...Array.from({ length: 13 }, (_, i) => entry(i + 1, i < 7 ? 'app/orders' : 'app/reports')),
      { id: 'fn:orders.helper', kind: 'logic', name: 'helper', module: 'app/orders', public: false, summary: '' },
      { id: 'effect:changes Store', kind: 'effect', name: 'changes Store', summary: "Takes the program's Store and returns a changed one." },
    ];
    const edges = [{ from: 'fn:api.handle', to: 'effect:changes Store', kind: 'uses', direct: true }, { from: 'fn:app/orders.e3', to: 'effect:changes Store', kind: 'uses', direct: true }];
    const areas = [{ id: 'app/api', name: 'api', summary: 'The request handler.' }, { id: 'app/orders', name: 'orders', summary: 'Orders and their lines.' }, { id: 'app/reports', name: 'reports', summary: '' }];
    let [model] = view.init();
    const dispatch = msg => { let effect; [model, effect] = view.update(model, msg); perform(effect, dispatch, () => {}, () => {}, () => {}, () => {}, () => {}, () => {}); };
    dispatch(new view.Loaded(JSON.stringify({ nodes, edges, areas, project: { name: 'shop', fingerprint: 'f' } })));
    let html = to_string(view.view(model));
    expect(html).toContain('class="view-outline"');
    expect(html).not.toContain('class="view-diagram"');
    expect(html).toContain('<h2>What shop does</h2>');
    expect(html).toContain('14 functions in 3 areas.');
    // The entry with requests comes first, its requests in words.
    expect(html.indexOf('Main ways in')).toBeLessThan(html.indexOf('By area'));
    expect(html).toContain('Requests it handles (2)');
    expect(html).toContain('<li>Add item<small> order id, sku, quantity</small></li>');
    // Areas fold, with their doc summary and counts; rows say what goes in and out, effects and tests.
    expect(html).toContain('<span class="view-area-name">orders</span><span class="view-area-counts">7 functions · 3 not checked</span><span class="view-area-summary">Orders and their lines.</span>');
    expect(html).toContain('Store, id → Payment or an error · changes Store · checked by 1 test');
    expect(html).toContain('Store, id → Payment or an error · not checked');
    expect(html).not.toContain('>helper<');
    expect(html).not.toContain('No summary recorded');
    // One button asks an agent to describe what has no description.
    expect(html).toContain('1 public function has no description yet.');
    dispatch(new view.AskForSummaries());
    for (let i = 0; i < 50 && model.asked !== 'asked'; i++) await Bun.sleep(5);
    expect(posts).toHaveLength(1);
    expect(posts[0][0]).toBe('/tasks');
    expect(posts[0][1].title).toBe('Describe undocumented functions');
    expect(posts[0][1].body).toContain('- app/orders.e1');
    expect(posts[0][1].body).not.toContain('helper');
    expect(to_string(view.view(model))).toContain('Task added to the board');
    // Opening a row shows the drawing with that function selected, with examples from its tests.
    dispatch(new view.Show('fn:api.handle'));
    for (let i = 0; i < 50 && !Array.from(model.detail[0]?.examples || []).length; i++) await Bun.sleep(5);
    expect(model.outline).toBe(false);
    expect(model.selected).toBe('fn:api.handle');
    html = to_string(view.view(model));
    expect(html).toContain('class="view-diagram"');
    expect(html).toContain('<h3>Examples</h3>');
    expect(html).toContain('api.handle(db, env, GetOrder(1)) → #(db, ok)');
    // The drawing's overview folds 14 entry points into 3 area boxes; expanding one frames its entry points.
    dispatch(new view.CollapseAll());
    html = to_string(view.view(model));
    expect((html.match(/class="view-node area/g) || []).length).toBe(3);
    expect(html).not.toMatch(/class="view-node entry/);
    expect(html).toContain('7 ways in');
    expect(html).toContain('7 ways in · changes Store');
    expect(html).not.toContain('class="view-link"');
    dispatch(new view.Toggle('area:app/orders'));
    html = to_string(view.view(model));
    expect((html.match(/class="view-node entry/g) || []).length).toBe(7);
    expect(html).toMatch(/<g class="view-frame"><rect[^>]*><\/rect><\/g>/);
  } finally { globalThis.location = oldLocation; globalThis.fetch = oldFetch; }
});

test('diagram helpers use the view shadow root and ignore typing in an inner field', async () => {
  const oldDocument = globalThis.document, oldFrame = globalThis.requestAnimationFrame, oldCSS = globalThis.CSS;
  const listeners = new Map(), actions = [];
  let focused = false;
  const input = { closest: selector => selector.includes('input') ? input : null };
  const node = { focus: () => { focused = true; } };
  const shadowRoot = {
    addEventListener: (name, fn) => listeners.set(name, fn),
    querySelector: selector => selector === '.view-drawing' ? { clientWidth: 612, clientHeight: 450 }
      : selector === '.view-diagram' ? {} : selector.startsWith('[data-node=') ? node : null,
  };
  // Nested in a shell's own shadow root, the view can't be found from document.
  globalThis.document = { querySelector: () => null };
  globalThis.requestAnimationFrame = fn => fn();
  globalThis.CSS = { escape: value => value };
  try {
    const layout = await import('../src/layout.mjs');
    expect(layout.viewport_width(shadowRoot)).toBe(612);
    expect(layout.viewport_height(shadowRoot)).toBe(450);
    expect(layout.viewport_width(null)).toBe(Math.max(240, (globalThis.innerWidth || 1440) - 48));
    layout.keys(shadowRoot, action => actions.push(action));
    const keydown = listeners.get('keydown');
    keydown({ key: 'ArrowRight', target: {}, composedPath: () => [input], preventDefault() {} });
    expect(actions).toEqual([]);
    keydown({ key: 'ArrowRight', target: {}, composedPath: () => [{ closest: () => null }], preventDefault() {} });
    expect(actions).toEqual(['right']);
    layout.focus_node(shadowRoot, 'one');
    expect(focused).toBe(true);
  } finally {
    globalThis.document = oldDocument; globalThis.requestAnimationFrame = oldFrame; globalThis.CSS = oldCSS;
  }
});
