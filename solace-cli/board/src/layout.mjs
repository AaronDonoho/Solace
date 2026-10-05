import dagre from '@dagrejs/dagre';
// `root` below is the contract view's own shadow root, passed by its component.
const eventElement = event => event.composedPath?.()[0] || event.target;

export const nodeWidth = 220;
export const nodeHeight = 120;
export const minZoom = 0.8;
let memory = {};
let memoryKey = '';

export function viewport_width(root) {
  return root?.querySelector?.('.view-drawing')?.clientWidth || Math.max(240, (globalThis.innerWidth || 1440) - 48);
}
export function viewport_height(root) {
  return root?.querySelector?.('.view-drawing')?.clientHeight || Math.max(240, (globalThis.innerHeight || 900) - ((globalThis.innerWidth || 1440) < 700 ? 248 : 360));
}
export function layout(nodes, edges, overview = false, anchor = '', root = null, base = '') {
  // v2 retires the old single-column coordinates; viewport width separates phone and desktop sheets.
  const key = 'solace-view-positions:v2:' + (base || globalThis.location?.pathname.replace(/\/?view$/, '') || '') + (globalThis.location?.search || '') + ':' + Math.round(viewport_width(root));
  if (key !== memoryKey) {
    memoryKey = key;
    memory = {};
    try { memory = JSON.parse(localStorage.getItem(key) || '{}'); } catch {}
  }
  const saved = overview ? {} : { ...memory };
  const valid = point => point && Number.isFinite(point.x) && Number.isFinite(point.y);
  const ids = new Set(nodes.map(node => node.id));
  const links = edges.filter(edge => ids.has(edge.from) && ids.has(edge.to));
  const connected = new Set(links.flatMap(edge => [edge.from, edge.to]));
  const fresh = {};
  let gridY = 0;
  if (connected.size) {
    const graph = new dagre.graphlib.Graph();
    graph.setGraph({ rankdir: 'LR', nodesep: 36, ranksep: 100 });
    graph.setDefaultEdgeLabel(() => ({}));
    for (const id of connected) graph.setNode(id, { width: nodeWidth, height: nodeHeight });
    for (const edge of links) graph.setEdge(edge.from, edge.to);
    dagre.layout(graph);
    for (const id of connected) {
      const point = graph.node(id);
      fresh[id] = { x: point.x - nodeWidth / 2, y: point.y - nodeHeight / 2 };
    }
    gridY = graph.graph().height + 40;
  }
  const isolated = nodes.filter(node => !connected.has(node.id));
  // Columns fill the sheet width at full size; extra rows run below and are reached by panning.
  const columns = Math.max(1, Math.floor((viewport_width(root) - 32) / 252));
  isolated.forEach((node, index) => {
    fresh[node.id] = { x: (index % columns) * 252, y: gridY + Math.floor(index / columns) * 156 };
  });
  if (!overview && valid(saved[anchor])) beside(saved, anchor, links, ids, valid);
  const occupied = nodes.map(node => saved[node.id]).filter(valid);
  const positions = nodes.map(node => {
    const stored = valid(saved[node.id]);
    const point = stored ? saved[node.id] : fresh[node.id];
    if (!stored) {
      // ponytail: linear collision scan for the visible subgraph; use a spatial index if it grows large.
      while (occupied.some(other => Math.abs(other.x - point.x) < 240 && Math.abs(other.y - point.y) < 140)) point.y += 156;
      occupied.push(point);
    }
    saved[node.id] = point;
    return { id: node.id, ...point };
  });
  memory = saved;
  try { localStorage.setItem(key, JSON.stringify(memory)); } catch {}
  return positions;
}
// A newly opened node's neighbours go next to it: callees in a column to its right, callers to its
// left, tests below. Everything beyond moves over to make room, so nothing already drawn changes order.
function beside(saved, anchor, links, ids, valid) {
  const a = saved[anchor];
  const isNew = id => ids.has(id) && !valid(saved[id]);
  const unique = list => [...new Set(list)];
  const callees = unique(links.filter(e => e.from === anchor && e.kind !== 'tests' && isNew(e.to)).map(e => e.to));
  const callers = unique(links.filter(e => e.to === anchor && e.kind !== 'tests' && isNew(e.from)).map(e => e.from)).filter(id => !callees.includes(id));
  const tests = unique(links.filter(e => e.to === anchor && e.kind === 'tests' && isNew(e.from)).map(e => e.from));
  const W = 252, H = 156;
  const shift = (pred, dx, dy) => { for (const p of Object.values(saved)) if (valid(p) && p !== a && pred(p)) { p.x += dx; p.y += dy; } };
  const place = (list, x, y0) => list.forEach((id, i) => { saved[id] = { x, y: y0 + i * H }; });
  if (callees.length) { shift(p => p.x > a.x + 1, W, 0); place(callees, a.x + W, a.y); }
  if (callers.length) { shift(p => p.x < a.x - 1, -W, 0); place(callers, a.x - W, a.y); }
  if (tests.length) { shift(p => Math.abs(p.x - a.x) < 1 && p.y > a.y + 1, 0, H * tests.length); place(tests, a.x, a.y + H); }
}
export function fit(positions, width = viewport_width(), height = viewport_height(), panel = false) {
  if (!positions.length) return { zoom: 1, x: 0, y: 0 };
  const available = Math.max(240, width - (panel && (globalThis.innerWidth || 1440) >= 700 ? panel_width() : 0));
  const left = Math.min(...positions.map(point => point.x)) - 16;
  const top = Math.min(...positions.map(point => point.y)) - 16;
  const right = Math.max(...positions.map(point => point.x)) + nodeWidth + 16;
  const bottom = Math.max(...positions.map(point => point.y)) + nodeHeight + 16;
  // Never shrink below a readable size: a large drawing is panned, not squeezed.
  // Height never forces shrinking: tall drawings scroll by panning.
  const zoom = Math.max(minZoom, Math.min(1, available / (right - left), (bottom - top) <= height * 1.25 ? height / (bottom - top) : 1));
  const fitsWide = (right - left) * zoom <= available;
  return { zoom, x: fitsWide ? (left + right - available / zoom) / 2 : left, y: top };
}
export function placement(nodes_json, edges_json, overview, anchor, root, base) {
  return JSON.stringify(layout(JSON.parse(nodes_json), JSON.parse(edges_json), overview, anchor, root, base));
}
export function fitted(positions_json, panel, root) { return JSON.stringify(fit(JSON.parse(positions_json), viewport_width(root), viewport_height(root), panel)); }
export function task_id() { return new URLSearchParams(location.search).get('task') || ''; }

// Fit after Lustre has painted the new legend/note and the lettering has loaded.
export function after_draw(callback) {
  const frame = globalThis.requestAnimationFrame || (fn => setTimeout(fn, 0));
  (globalThis.document?.fonts?.ready || Promise.resolve()).then(() => frame(() => frame(callback)));
}

// Diagram keys: arrows/WASD move between boxes; with Shift they pan, Ctrl+Up/Down zoom;
// Space expands, C collapses (with Shift: all), Enter opens. Typing in a field or pressing a button is left alone.
const directions = { ArrowLeft: 'left', a: 'left', ArrowRight: 'right', d: 'right', ArrowUp: 'up', w: 'up', ArrowDown: 'down', s: 'down' };
const keyRoots = new WeakSet();
export function keys(target, send) {
  if (!target?.addEventListener || keyRoots.has(target)) return;
  keyRoots.add(target);
  target.addEventListener('keydown', event => {
    if (event.altKey || event.metaKey || eventElement(event)?.closest?.('input,textarea,select,button,a,summary,[contenteditable],[role=separator]')) return;
    if (!target.querySelector?.('.view-diagram')) return; // a file is open in place of the drawing
    const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
    const direction = directions[key];
    let action = '';
    // Ctrl+W and Ctrl+S stay the browser's: Ctrl+W closes the tab and can't be intercepted.
    if (event.ctrlKey) action = { ArrowUp: 'zoom-in', ArrowDown: 'zoom-out' }[key] || '';
    else if (direction) action = (event.shiftKey ? 'pan-' : '') + direction;
    else if (event.shiftKey) action = { ' ': 'expand-all', c: 'collapse-all' }[key] || '';
    else action = { ' ': 'expand', c: 'collapse', Enter: 'open' }[key] || '';
    if (!action) return;
    event.preventDefault();
    send(action);
  });
}
export function focus_node(root, id) {
  const frame = globalThis.requestAnimationFrame || (fn => setTimeout(fn, 0));
  frame(() => root?.querySelector?.(`[data-node="${CSS.escape(id)}"]`)?.focus({ preventScroll: true }));
}

// The side panel's width: drag its left edge, or focus the edge and press Left/Right. Kept per browser.
const widthKey = 'solace-view-panel-width';
export function panel_width() {
  const value = parseFloat(globalThis.document?.documentElement.style.getPropertyValue('--panel-width'));
  return Number.isFinite(value) ? value : 410;
}
function setPanelWidth(width) {
  const max = Math.max(320, (globalThis.innerWidth || 1440) - 240);
  const clamped = Math.round(Math.min(max, Math.max(320, width)));
  document.documentElement.style.setProperty('--panel-width', clamped + 'px');
  try { localStorage.setItem(widthKey, String(clamped)); } catch {}
}
const resizeRoots = new WeakSet();
export function resizer(target) {
  const document = globalThis.document;
  if (!document || !target?.addEventListener || resizeRoots.has(target)) return;
  resizeRoots.add(target);
  try { const saved = parseFloat(localStorage.getItem(widthKey)); if (Number.isFinite(saved)) setPanelWidth(saved); } catch {}
  target.addEventListener('pointerdown', event => {
    if (!eventElement(event)?.closest?.('.view-panel-grip')) return;
    event.preventDefault();
    const move = e => setPanelWidth(globalThis.innerWidth - e.clientX);
    const stop = () => { document.removeEventListener('pointermove', move); document.removeEventListener('pointerup', stop); (target.host || document.body).classList.remove('view-resizing'); };
    (target.host || document.body).classList.add('view-resizing');
    document.addEventListener('pointermove', move);
    document.addEventListener('pointerup', stop);
  });
  target.addEventListener('keydown', event => {
    if (!eventElement(event)?.closest?.('.view-panel-grip')) return;
    const step = { ArrowLeft: 32, ArrowRight: -32 }[event.key];
    if (!step) return;
    event.preventDefault();
    setPanelWidth(panel_width() + step);
  });
}
