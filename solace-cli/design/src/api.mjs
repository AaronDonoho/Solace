// Browser access only: decoding and rendering belong to Lustre.
export function request(base, path, method, body, receive) {
  fetch((path.startsWith('/p/') ? '' : base) + path, { method, headers: { 'Content-Type': 'application/json' },
    ...(method === 'GET' ? {} : { body }) })
    .then(async response => receive(await response.text()))
    .catch(error => receive(JSON.stringify({ message: String(error) })));
}

export function clock(seconds) {
  return seconds ? new Date(seconds * 1000).toLocaleString() : '';
}

export function stamp(seconds) {
  return seconds ? new Date(seconds * 1000).toISOString() : '';
}

// The agent panel's transcript polling: one step later.
export function after(ms, callback) { setTimeout(callback, ms); }

// Browser storage may be missing or blocked; the page works without it.
export function load(key) {
  try { return globalThis.localStorage?.getItem(key) ?? ''; } catch { return ''; }
}
export function store(key, value) {
  try { value ? localStorage.setItem(key, value) : localStorage.removeItem(key); } catch {}
}

// The window's width, 0 with no window: the panel's bound.
export function window_width() { return Math.round(globalThis.innerWidth ?? 0); }

// Dragging the panel's left edge: leftwards widens it. `move` gets each width,
// then `done` once the pointer is released.
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

// Scrolls the panel to its end once it has rendered.
export function scroll_plan(root) {
  const body = root?.querySelector?.('.plan-body');
  const end = body?.querySelector('.plan-end');
  if (!body || !end) return;
  body.scrollTop += end.getBoundingClientRect().bottom - body.getBoundingClientRect().bottom;
}

// The shadow root before the page has rendered: the editor's tools do nothing yet.
export function no_root() { return null; }
