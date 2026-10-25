// Browser transport only: state, decoding and rendering belong to Lustre.
// `base` is the project prefix a request belongs to: "" for the root's routes,
// and a workspace project's `/p/<dir>` (D-57). The board's own routes stay at
// the root, even while a contract page is the one shown.
// A component's shadow root before it has rendered: the helpers do nothing.
export function no_root() { return null; }
export function request(base, path, method, body, receive) {
  fetch((path.startsWith('/p/') ? '' : base) + path, { method, headers: { 'Content-Type': 'application/json' },
    ...(method === 'GET' ? {} : { body }) })
    .then(async response => receive(await response.text()))
    .catch(error => receive(JSON.stringify({ message: String(error) })));
}
export function events(base, receive, ready = () => {}) {
  const source = new EventSource(base + '/events');
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
// or its right edge on a wide window (`side`), so the handle drags from the
// top or the left, which CSS `resize` cannot do: the browser keeps the
// pointer on the handle (`setPointerCapture`), so its pointermove and
// pointerup keep arriving as the pointer travels over the page. The sizes
// are px; the largest keeps the board's column bar on screen, or a strip of
// the board beside the band. Outside a browser (tests), the sizes fall back.
const detailMin = 120, detailKeep = 160;
export function detail_max(side = false) {
  const room = side ? globalThis.innerWidth || 1280 : globalThis.innerHeight || 800;
  return Math.max(detailMin, room - detailKeep);
}
export function detail_min() { return detailMin; }
// The side band's smallest width, 32rem, in px at the page's font size.
export function side_min() {
  const size = parseFloat(globalThis.getComputedStyle?.(globalThis.document?.documentElement)?.fontSize);
  return Math.round(32 * (size || 16));
}
// The width the lanes area keeps for its scroll bar (`scrollbar-gutter`):
// 0 with overlay scroll bars, and it differs between platforms, so it is
// measured once. Outside a browser, 16px.
let gutter;
export function scroll_gutter() {
  const doc = globalThis.document;
  if (gutter === undefined && doc?.body) {
    const probe = doc.createElement('div');
    probe.style.cssText = 'position:absolute;top:-999px;width:100px;height:100px;overflow:scroll';
    doc.body.appendChild(probe);
    gutter = probe.offsetWidth - probe.clientWidth;
    probe.remove();
  }
  return gutter ?? 16;
}
// The size a drag or a key step starts from: the task's height, or on the
// side the band's width, while it is still the CSS default (45dvh, or
// min(42vw, 720px)). Outside a browser, from an 800px high, 1280px wide window.
function detailSize(root, side) {
  if (side) {
    const band = root?.querySelector?.('.detail-band');
    return band ? band.getBoundingClientRect().width : 0;
  }
  const task = root?.querySelector?.('.detail-band > solace-task');
  return task ? task.getBoundingClientRect().height : 0;
}
export function detail_now(root, side = false) {
  const fallback = side ? Math.max(side_min(), Math.min(0.42 * (globalThis.innerWidth || 1280), 720)) : 0.45 * (globalThis.innerHeight || 800);
  return Math.round(detailSize(root, side) || fallback);
}
export function begin_resize(root, pointerId, at, side = false) {
  try { root?.querySelector?.('.detail-resize')?.setPointerCapture?.(pointerId); } catch {}
  return [Number(at) || 0, Number(detailSize(root, side)) || 0, detail_max(side)];
}
// The band's size for a pointer at `at` in a drag that began at `start` with
// the band `startSize` along the drag; the board clamps it.
export function resize_size(start, startSize, at) {
  return Math.round(startSize + (start - at));
}

// The band moves to the right edge when the window is at least 1.1 times
// as wide as it is tall. `send` hears whether it is, now and on every
// change; a root that asks again replaces its listener.
const sideWatchers = new WeakMap();
export function watch_side(root, send) {
  const media = globalThis.matchMedia?.('(min-aspect-ratio: 11/10)');
  if (!media || !root) return;
  const old = sideWatchers.get(root);
  if (old) old[0].removeEventListener('change', old[1]);
  const listener = () => send(media.matches);
  sideWatchers.set(root, [media, listener]);
  media.addEventListener('change', listener);
  send(media.matches);
}