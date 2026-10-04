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
  globalThis.EventSource = class { constructor(path) { requests.push(['GET', path]); source = this; } };
  let model;
  const dispatch = msg => { let effect; [model, effect] = view.update(model, msg); perform(effect, dispatch, () => {}, () => {}, () => {}, () => {}, () => {}, () => {}); };
  try {
    let effect; [model, effect] = view.init(); perform(effect, dispatch, () => {}, () => {}, () => {}, () => {}, () => {}, () => {});
    for (let i = 0; i < 50 && !Array.from(model.graph.nodes).length; i++) await Bun.sleep(5);
    expect(Array.from(model.graph.nodes)).toHaveLength(3);
    expect((to_string(view.view(model)).match(/class="view-node /g) || []).length).toBe(2);
    expect(requests).toContainEqual(['POST', '/tests']);
    review = { ...review, summary: '1 effect added, 0 test unfinished', tests: [{ id: 'fn:app.work', name: 'work_test', result: 'passing', checks: [{ call: 'app.main()', expected: '3', was: '2' }] }] };
    source.onmessage({ data: JSON.stringify({ task: { id: 't1' } }) });
    for (let i = 0; i < 50 && model.graph.review[0]?.tests.head.result !== 'passing'; i++) await Bun.sleep(5);
    expect(model.graph.review[0].tests.head.result).toBe('passing');
    const reviewHtml = to_string(view.view(model));
    expect(reviewHtml).toContain('class="view-panel view-review"');
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
    expect(detailHtml).toContain('aria-label="Node detail"');
    expect(detailHtml).toContain('Collapse connections');
    expect(detailHtml).toContain('class="view-node entry changed selected"');
    expect(detailHtml).toMatch(/class="view-expander"[^]*?>−<\/text>/);
    expect(detailHtml).toContain('<title>main calls work</title>');
    expect(detailHtml).toContain('class="view-panel-grip"');
    expect(detailHtml).toContain('Show source');
    expect(detailHtml).toContain('<title>entry: main. ' + nodes[0].summary);
    expect(detailHtml).toContain('aria-label="entry: main. ' + nodes[0].summary);
    expect(detailHtml).toContain('class="view-summary">' + nodes[0].summary);
    expect(detailHtml).toContain('class="view-signature">pub fn main(entries: List(Entry), subject: Subject) -&gt; List(Entry)');
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
