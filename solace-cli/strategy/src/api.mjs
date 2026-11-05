// Browser access only: state, decoding and rendering belong to Lustre.
// The tab shown lives in the URL (`/strategy?tab=tasks`), so a reload returns to it.
export function tab_query() { return globalThis.location?.search ?? ''; }
// `base` is the component's API prefix; a workspace project's own paths (`/p/<dir>/...`) stay as they are.
export function request(base, path, method, body, receive) {
  fetch((path.startsWith('/p/') ? '' : base) + path, { method, headers: { 'Content-Type': 'application/json' },
    ...(method === 'GET' ? {} : { body }) })
    .then(async response => receive(await response.text()))
    .catch(error => receive(JSON.stringify({ message: String(error) })));
}
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
// Dragging a feature card by its grip within its milestone: the card follows
// the pointer and the others slide aside to show where it would land. `done`
// gets the index it lands at among the group's cards, -1 when cancelled. The
// cards are measured once, so the target stays steady while they move.
export function drag_card(root, id, y, done) {
  if (!/^[\w-]+$/.test(id)) return;
  const card = root?.querySelector?.(`[data-feature-id="${id}"]`);
  const section = card?.closest?.('section.milestone');
  if (!section) return done(-1);
  const cards = [...section.querySelectorAll(':scope > [data-feature-id]')];
  const from = cards.indexOf(card);
  const rects = cards.map(c => c.getBoundingClientRect());
  const own = rects[from];
  const gap = from + 1 < rects.length ? rects[from + 1].top - own.bottom
    : from > 0 ? own.top - rects[from - 1].bottom : 0;
  const shift = own.bottom - own.top + gap;
  const others = cards.map((c, i) => ({ c, i, mid: (rects[i].top + rects[i].bottom) / 2 })).filter(o => o.i !== from);
  const body = globalThis.document?.body;
  const select = body?.style.userSelect;
  if (body) body.style.userSelect = 'none';
  card.classList.add('dragging');
  let to = from;
  const onMove = e => {
    const dy = e.clientY - y;
    const centre = (own.top + own.bottom) / 2 + dy;
    to = others.filter(o => o.mid < centre).length;
    card.style.transform = `translateY(${dy}px)`;
    others.forEach((o, j) => {
      o.c.style.transform = o.i < from && j >= to ? `translateY(${shift}px)`
        : o.i > from && j < to ? `translateY(${-shift}px)` : '';
    });
  };
  const end = index => {
    globalThis.removeEventListener('pointermove', onMove);
    globalThis.removeEventListener('pointerup', onUp);
    globalThis.removeEventListener('pointercancel', onCancel);
    globalThis.removeEventListener('keydown', onKey);
    if (body) body.style.userSelect = select;
    card.classList.remove('dragging');
    // The cards take their new places at once, without sliding back first.
    cards.forEach(c => { c.style.transition = 'none'; c.style.transform = ''; });
    globalThis.requestAnimationFrame?.(() => cards.forEach(c => { c.style.transition = ''; }));
    done(index);
  };
  const onUp = () => end(to);
  const onCancel = () => end(-1);
  const onKey = e => { if (e.key === 'Escape') end(-1); };
  globalThis.addEventListener('pointermove', onMove);
  globalThis.addEventListener('pointerup', onUp);
  globalThis.addEventListener('pointercancel', onCancel);
  globalThis.addEventListener('keydown', onKey);
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
// Keeps the active combobox option in view as the arrow keys move it.
export function scroll_option(root, id) {
  root?.getElementById?.(id)?.scrollIntoView?.({ block: 'nearest' });
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
// The shadow root before the page has rendered: the editor's tools do nothing yet.
export function no_root() { return null; }
// A task's closedTime, in seconds, as the reader's local date and time.
export function clock(seconds) {
  return new Date(seconds * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}
// The two lines of a date column: its date, then its time.
export function clock_date(seconds) {
  return new Date(seconds * 1000).toLocaleDateString(undefined, { dateStyle: 'medium' });
}
export function clock_time(seconds) {
  return new Date(seconds * 1000).toLocaleTimeString(undefined, { timeStyle: 'short' });
}
