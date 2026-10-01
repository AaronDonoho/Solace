import dagre from '@dagrejs/dagre';

export const nodeWidth = 220;
export const nodeHeight = 120;
export const minZoom = 0.8;
let memory = {};
let memoryKey = '';

export function viewport_width() {
  return globalThis.document?.querySelector('.view-drawing')?.clientWidth || Math.max(240, (globalThis.innerWidth || 1440) - 48);
}
export function viewport_height() {
  return globalThis.document?.querySelector('.view-drawing')?.clientHeight || Math.max(240, (globalThis.innerHeight || 900) - ((globalThis.innerWidth || 1440) < 700 ? 248 : 360));
}
export function layout(nodes, edges, overview = false, anchor = '') {
  // v2 retires the old single-column coordinates; viewport width separates phone and desktop sheets.
  const key = 'solace-view-positions:v2:' + (globalThis.location?.search || '') + ':' + Math.round(viewport_width());
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
  const columns = Math.max(1, Math.floor((viewport_width() - 32) / 252));
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
  const available = Math.max(240, width - (panel && (globalThis.innerWidth || 1440) >= 700 ? 410 : 0));
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
export function placement(nodes_json, edges_json, overview, anchor) {
  return JSON.stringify(layout(JSON.parse(nodes_json), JSON.parse(edges_json), overview, anchor));
}
export function fitted(positions_json, panel) { return JSON.stringify(fit(JSON.parse(positions_json), undefined, undefined, panel)); }
export function task_id() { return new URLSearchParams(location.search).get('task') || ''; }

// Fit after Lustre has painted the new legend/note and the lettering has loaded.
export function after_draw(callback) {
  const frame = globalThis.requestAnimationFrame || (fn => setTimeout(fn, 0));
  (globalThis.document?.fonts?.ready || Promise.resolve()).then(() => frame(() => frame(callback)));
}
