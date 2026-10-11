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
export function request_id() {
  return crypto.randomUUID();
}

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

// Flips the page between light and dark; index.html applies the stored choice.
export function toggle_theme() {
  const root = globalThis.document.documentElement;
  const next = root.dataset.theme === 'dark' ? 'light' : 'dark';
  root.dataset.theme = next;
  store('solace-theme', next);
}

const dragRoots = new WeakSet();
export function prevent_drag(target) {
  if (!target?.addEventListener || dragRoots.has(target)) return;
  dragRoots.add(target);
  target.addEventListener('dragstart', e => {
    const el = e.composedPath?.()[0] || e.target;
    if (!el?.closest?.('input,textarea')) e.preventDefault();
  });
}
