// Browser transport only: state, decoding and rendering belong to Lustre.
// Each component instance passes its own `base` attribute; without one, a
// workspace project's contract page at `/p/<dir>/contracts` asks its own
// project (D-57). The board passes each lane's prefix itself. The open task's
// details, with their markdown and editor helpers, are `<solace-task>` in the
// components project.
const fallbackBase = () => globalThis.location?.pathname.match(/^(.*)\/contracts$/)?.[1] ?? '';
const prefix = base => base || fallbackBase();
// A component's shadow root before it has rendered: the helpers do nothing.
export function no_root() { return null; }
export function request(base, path, method, body, receive) {
  fetch((path.startsWith('/p/') ? '' : prefix(base)) + path, { method, headers: { 'Content-Type': 'application/json' },
    ...(method === 'GET' ? {} : { body }) })
    .then(async response => receive(await response.text()))
    .catch(error => receive(JSON.stringify({ message: String(error) })));
}
export function events(base, receive, ready = () => {}) {
  const source = new EventSource(prefix(base) + '/events');
  source.onmessage = event => receive(event.data);
  source.addEventListener('ready', () => ready());
  // Native EventSource reconnects with Last-Event-ID.
  source.onerror = () => {};
  return source;
}
export function close_events(source) { source?.close(); }
export function reload() {
  globalThis.location?.reload();
}

// Browser storage may be missing or blocked; the board works without it.
export function load(key) {
  try { return globalThis.localStorage?.getItem(key) ?? ''; } catch { return ''; }
}
export function store(key, value) {
  try { value ? localStorage.setItem(key, value) : localStorage.removeItem(key); } catch {}
}

const dragRoots = new WeakSet();
export function prevent_drag(target) {
  if (!target?.addEventListener || dragRoots.has(target)) return;
  dragRoots.add(target);
  // A markdown editor, <solace-md-editor>, keeps its text in a shadow root of its own: it is found on the path.
  target.addEventListener('dragstart', e => {
    const el = e.composedPath?.()[0] || e.target;
    const editor = e.composedPath?.().some(n => n?.classList?.contains?.('md-editor'));
    if (!el?.closest?.('input,textarea') && !editor) e.preventDefault();
  });
}

// The detail band's resize handle. The band sits at the page's bottom edge,
// so the handle drags from the top, which CSS `resize` cannot do: the
// browser keeps the pointer on the handle (`setPointerCapture`), so its
// pointermove and pointerup keep arriving as the pointer travels over the
// page. The heights are px; the largest keeps the board's column bar on
// screen. Outside a browser (tests), the sizes fall back.
const detailMin = 120, detailKeep = 160;
export function detail_max() {
  return Math.max(detailMin, (globalThis.innerHeight || 800) - detailKeep);
}
export function detail_min() { return detailMin; }
// The band's task height now: a key step starts from it while the height
// is still the CSS default (45dvh). Outside a browser, 45% of 800px.
export function detail_now(root) {
  const task = root?.querySelector?.('.detail-band > solace-task');
  const height = task ? task.getBoundingClientRect().height : 0;
  return Math.round(height || 0.45 * (globalThis.innerHeight || 800));
}
export function begin_resize(root, pointerId, clientY) {
  try { root?.querySelector?.('.detail-resize')?.setPointerCapture?.(pointerId); } catch {}
  const task = root?.querySelector?.('.detail-band > solace-task');
  const height = task ? task.getBoundingClientRect().height : 0;
  return [Number(clientY) || 0, Number(height) || 0, detail_max()];
}
export function resize_height(startY, startH, y, max) {
  const height = Math.round(startH + (startY - y));
  return Math.min(Math.max(height, detailMin), Math.max(max, detailMin));
}