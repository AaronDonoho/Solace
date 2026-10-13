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
// Browser storage may be missing or blocked; the page works without it.
export function load(key) {
  try { return globalThis.localStorage?.getItem(key) ?? ''; } catch { return ''; }
}
export function store(key, value) {
  try { value ? localStorage.setItem(key, value) : localStorage.removeItem(key); } catch {}
}
// The window's width, 0 with no window: the planning panel's bound.
export function window_width() { return Math.round(globalThis.innerWidth ?? 0); }
// Dragging the planning panel's left edge: leftwards widens it. `move` gets
// each width, then `done` once the pointer is released.
export function drag(x, width, move, done) {
  const body = globalThis.document?.body;
  const select = body?.style.userSelect;
  if (body) body.style.userSelect = 'none';
  const onMove = e => move(Math.round(width + x - e.clientX));
  const onUp = () => {
    globalThis.removeEventListener('pointermove', onMove);
    globalThis.removeEventListener('pointerup', onUp);
    globalThis.removeEventListener('pointercancel', onUp);
    if (body) body.style.userSelect = select;
    done();
  };
  globalThis.addEventListener('pointermove', onMove);
  globalThis.addEventListener('pointerup', onUp);
  globalThis.addEventListener('pointercancel', onUp);
}
// The planning panel follows its newest message (D-118): the run's end, right
// above the pinned message box, is kept at the bottom of the panel's scroll area.
const reach = 48;
export function near_end(body) {
  const end = body?.querySelector?.('.plan-end');
  if (!end) return true;
  return end.getBoundingClientRect().bottom - body.getBoundingClientRect().bottom < reach;
}
export function scroll_plan(root) {
  const body = root?.querySelector?.('.plan-body');
  const end = body?.querySelector('.plan-end');
  if (!body || !end) return;
  body.scrollTop += end.getBoundingClientRect().bottom - body.getBoundingClientRect().bottom;
}
// Shows the feature or milestone a link asked for (D-122): scrolls its card into view, focuses it and
// marks it for a moment. The stream may not have drawn it yet, so it is looked for again a few times.
export function reveal_card(root, kind, id, tries = 20) {
  if (!/^[\w-]+$/.test(id)) return;
  const card = root?.querySelector?.(`[data-${kind}-id="${id}"]`);
  if (!card) {
    if (root && tries > 0) setTimeout(() => reveal_card(root, kind, id, tries - 1), 150);
    return;
  }
  const calm = globalThis.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches;
  card.scrollIntoView?.({ block: 'start', behavior: calm ? 'auto' : 'smooth' });
  if (!card.hasAttribute?.('tabindex')) card.setAttribute?.('tabindex', '-1');
  card.focus?.({ preventScroll: true });
  card.classList?.add('revealed');
  setTimeout(() => card.classList?.remove('revealed'), 2000);
}
// The planning panel's message box grows with its text up to its CSS
// max-height, then scrolls. Where the browser has no `field-sizing`, its
// height is set from the text here.
export function fit_plan_text(root) {
  if (globalThis.CSS?.supports?.('field-sizing', 'content')) return;
  const box = root?.querySelector?.('#plan-text');
  if (!box) return;
  box.style.height = 'auto';
  box.style.height = box.scrollHeight + 2 + 'px';
}
// The shadow root before the page has rendered: the editor's tools do nothing yet.
export function no_root() { return null; }
// A task's closedTime, in seconds, as the reader's local date and time.
export function clock(seconds) {
  return new Date(seconds * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}
