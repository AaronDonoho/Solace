// Browser transport only: state, decoding and rendering belong to Lustre.
// `base` is the project prefix this instance shows: "" for the root's, and a
// workspace project's `/p/<dir>` (D-57).
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

const dragRoots = new WeakSet();
export function prevent_drag(target) {
  if (!target?.addEventListener || dragRoots.has(target)) return;
  dragRoots.add(target);
  target.addEventListener('dragstart', e => {
    const el = e.composedPath?.()[0] || e.target;
    if (!el?.closest?.('input,textarea')) e.preventDefault();
  });
}
