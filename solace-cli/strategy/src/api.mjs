// Browser access only: state, decoding and rendering belong to Lustre.
// The tab shown lives in the URL (`/strategy?tab=tasks`), so a reload returns to it.
export function tab_query() { return globalThis.location?.search ?? ''; }
// replaceState, not pushState: picking a tab is not a page change, and modem sees no navigation.
export function set_tab(name) {
  if (!globalThis.location) return;
  const url = new URL(globalThis.location.href);
  url.searchParams.set('tab', name);
  globalThis.history.replaceState(globalThis.history.state, '', url);
}
// `base` is the component's API prefix; a workspace project's own paths (`/p/<dir>/...`) stay as they are.
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
// The planning panel's transcript polling: one step later.
export function after(ms, callback) { setTimeout(callback, ms); }
// A create's `request` id: a repeated save of the same draft makes one record (Spec 15).
export function request_id() { return crypto.randomUUID(); }
// The shadow root before the page has rendered: the editor's tools do nothing yet.
export function no_root() { return null; }
// A task's closedTime, in seconds, as the reader's local date and time.
export function clock(seconds) {
  return new Date(seconds * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}
