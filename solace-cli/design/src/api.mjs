// Browser access only: decoding and rendering belong to Lustre.
import { List } from './gleam.mjs';

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

// Escape clears the design selection anywhere on the page but a text field.
let escapeListener = null;
function typing(event) {
  const path = event.composedPath?.() || [event.target];
  return path.some(node =>
    Boolean(node?.closest?.('input, textarea, [contenteditable]:not([contenteditable="false"])')));
}
export function listen_escape(dispatch) {
  if (escapeListener) return;
  escapeListener = event => {
    if (event.key === 'Escape' && !typing(event)) dispatch();
  };
  globalThis.addEventListener('keydown', escapeListener);
}
export function ignore_escape() {
  if (!escapeListener) return;
  globalThis.removeEventListener('keydown', escapeListener);
  escapeListener = null;
}

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

// Dragging across a design's canvas: reports where the drag started and ended
// with the bounds of every drawn element under the canvas. Gleam decides which
// elements are inside. A drag with almost no movement stays a click.
let regionRoot = null;
export function listen_region(root, report) {
  if (!root || regionRoot === root) return;
  regionRoot = root;
  const doc = root.ownerDocument;
  const body = doc?.body;
  let gesture = null;

  const clearSelection = () => {
    doc?.getSelection?.()?.removeAllRanges();
    root?.getSelection?.()?.removeAllRanges();
  };

  const elementBounds = canvas =>
    Array.from(canvas.querySelectorAll('[data-design-id]'), element => {
      const rect = element.getBoundingClientRect();
      return [
        element.getAttribute('data-design-id'),
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
      ];
    });

  const removeListeners = () => {
    globalThis.removeEventListener('pointermove', onMove);
    globalThis.removeEventListener('pointerup', onUp);
    globalThis.removeEventListener('pointercancel', onCancel);
  };

  const finish = () => {
    removeListeners();
    if (body && gesture) body.style.userSelect = gesture.select;
    if (gesture?.box) gesture.box.remove();
    const done = gesture;
    gesture = null;
    return done;
  };

  function onDown(event) {
    if (event.button !== 0 || gesture) return;
    const canvas = event.target?.closest?.('[data-region]');
    if (!canvas || !canvas.getClientRects().length) return;
    gesture = {
      canvas,
      startX: event.clientX,
      startY: event.clientY,
      x: event.clientX,
      y: event.clientY,
      multi: Boolean(event.ctrlKey || event.metaKey),
      dragging: false,
      box: null,
      select: body?.style.userSelect ?? '',
    };
    if (body) body.style.userSelect = 'none';
    clearSelection();
    globalThis.addEventListener('pointermove', onMove);
    globalThis.addEventListener('pointerup', onUp);
    globalThis.addEventListener('pointercancel', onCancel);
  }

  function onMove(event) {
    if (!gesture) return;
    gesture.x = event.clientX;
    gesture.y = event.clientY;
    const moved = Math.max(
      Math.abs(gesture.x - gesture.startX),
      Math.abs(gesture.y - gesture.startY),
    );
    if (!gesture.dragging && moved < 4) return;
    gesture.dragging = true;
    event.preventDefault();
    if (body) body.style.userSelect = 'none';
    clearSelection();
    if (!gesture.box) {
      gesture.box = doc.createElement('div');
      gesture.box.setAttribute('data-design-region', '');
      gesture.box.style.cssText =
        'position:fixed;box-sizing:border-box;border:1px solid #174f78;' +
        'background:transparent;pointer-events:none;z-index:2147483647';
      doc.body.appendChild(gesture.box);
    }
    gesture.box.style.left = `${Math.min(gesture.startX, gesture.x)}px`;
    gesture.box.style.top = `${Math.min(gesture.startY, gesture.y)}px`;
    gesture.box.style.width = `${Math.abs(gesture.x - gesture.startX)}px`;
    gesture.box.style.height = `${Math.abs(gesture.y - gesture.startY)}px`;
  }

  function onUp(event) {
    if (!gesture) return;
    gesture.x = event.clientX;
    gesture.y = event.clientY;
    const done = finish();
    if (!done?.dragging) return;
    const swallow = click => {
      click.preventDefault();
      click.stopPropagation();
    };
    globalThis.addEventListener('click', swallow, true);
    setTimeout(
      () => globalThis.removeEventListener('click', swallow, true),
      0,
    );
    report(
      done.startX,
      done.startY,
      done.x,
      done.y,
      List.fromArray(elementBounds(done.canvas)),
      done.multi,
    );
  }

  function onCancel() {
    finish();
  }

  root.addEventListener('pointerdown', onDown);
}

// Scrolls the panel to its end once it has rendered.
export function scroll_plan(root) {
  const body = root?.querySelector?.('.plan-body');
  const end = body?.querySelector('.plan-end');
  if (!body || !end) return;
  body.scrollTop += end.getBoundingClientRect().bottom - body.getBoundingClientRect().bottom;
}

// Places the note box just below the first selected element, inside the
// canvas's scroll area. With no selected element on screen, it stays put.
export function place_note_box(root, id) {
  const box = root?.querySelector?.('[data-note-box]');
  const canvas = box?.closest?.('.design-canvas');
  if (!box || !canvas || !id) return;
  const target = Array.from(root.querySelectorAll('[data-design-id]')).find(
    element =>
      element.getAttribute('data-design-id') === id &&
      element.getClientRects().length,
  );
  if (!target) return;
  const area = canvas.getBoundingClientRect();
  const rect = target.getBoundingClientRect();
  const boxRect = box.getBoundingClientRect();
  const left = rect.left - area.left + canvas.scrollLeft;
  const top = rect.top - area.top + canvas.scrollTop + rect.height + 8;
  box.style.left = `${Math.max(0, Math.min(left, canvas.scrollWidth - boxRect.width - 8))}px`;
  box.style.top = `${Math.max(0, Math.min(top, canvas.scrollHeight - boxRect.height - 8))}px`;
}

// The shadow root before the page has rendered: the editor's tools do nothing yet.
export function no_root() { return null; }
