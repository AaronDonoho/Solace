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

// Dragging the note box by its handle, within its canvas. The first real move
// marks the box, so a later selection leaves it where the human put it.
let noteRoot = null;
export function drag_note_box(root, moved) {
  if (!root || noteRoot === root) return;
  noteRoot = root;
  const doc = root.ownerDocument;
  const body = doc?.body;
  let gesture = null;

  const place = (box, canvas, left, top) => {
    box.style.left = `${Math.max(0, Math.min(left, canvas.scrollWidth - box.offsetWidth - 8))}px`;
    box.style.top = `${Math.max(0, Math.min(top, canvas.scrollHeight - box.offsetHeight - 8))}px`;
  };

  function onDown(event) {
    if (event.button !== 0 || gesture) return;
    const handle = event.target?.closest?.('[data-note-handle]');
    const box = handle?.closest?.('[data-note-box]');
    const canvas = box?.closest?.('.design-canvas');
    if (!handle || !box || !canvas) return;
    event.preventDefault();
    const rect = box.getBoundingClientRect();
    const area = canvas.getBoundingClientRect();
    gesture = {
      box,
      canvas,
      x: event.clientX,
      y: event.clientY,
      left: rect.left - area.left + canvas.scrollLeft,
      top: rect.top - area.top + canvas.scrollTop,
      dragging: false,
      select: body?.style.userSelect ?? '',
    };
    globalThis.addEventListener('pointermove', onMove);
    globalThis.addEventListener('pointerup', onUp);
    globalThis.addEventListener('pointercancel', onUp);
  }

  function onMove(event) {
    if (!gesture) return;
    const dx = event.clientX - gesture.x;
    const dy = event.clientY - gesture.y;
    if (!gesture.dragging && Math.max(Math.abs(dx), Math.abs(dy)) < 4) return;
    if (!gesture.dragging) {
      gesture.dragging = true;
      if (body) body.style.userSelect = 'none';
      gesture.box.style.right = 'auto';
      gesture.box.setAttribute('data-moved', '');
      moved();
    }
    event.preventDefault();
    place(gesture.box, gesture.canvas, gesture.left + dx, gesture.top + dy);
  }

  function onUp() {
    globalThis.removeEventListener('pointermove', onMove);
    globalThis.removeEventListener('pointerup', onUp);
    globalThis.removeEventListener('pointercancel', onUp);
    if (body && gesture) body.style.userSelect = gesture.select;
    gesture = null;
  }

  root.addEventListener('pointerdown', onDown);
}

// The id and viewport bounds of every drawn element under the canvas.
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

// Keeps the click that ends a drag from also selecting or clearing.
const swallowNextClick = () => {
  const swallow = click => {
    click.preventDefault();
    click.stopPropagation();
  };
  globalThis.addEventListener('click', swallow, true);
  setTimeout(() => globalThis.removeEventListener('click', swallow, true), 0);
};

// A resize handle's grab area: the selected element's sides and corners.
const resizeHandle = 7;
const resizeCursors = {
  left: 'ew-resize',
  right: 'ew-resize',
  top: 'ns-resize',
  bottom: 'ns-resize',
  'top-left': 'nwse-resize',
  'top-right': 'nesw-resize',
  'bottom-left': 'nesw-resize',
  'bottom-right': 'nwse-resize',
};

function resizeSideAt(event) {
  const element = event?.target?.closest?.('[data-resize]');
  const rect = element?.getBoundingClientRect?.();
  if (!rect) return null;
  const width = rect.right - rect.left;
  const height = rect.bottom - rect.top;
  if (!width || !height) return null;
  const near = (a, b) => Math.abs(a - b) <= resizeHandle;
  const x = event.clientX;
  const y = event.clientY;
  const left = near(x, rect.left);
  const right = near(x, rect.right);
  const top = near(y, rect.top);
  const bottom = near(y, rect.bottom);
  const centerX = near(x, rect.left + width / 2);
  const centerY = near(y, rect.top + height / 2);
  if (left && top) return 'top-left';
  if (right && top) return 'top-right';
  if (left && bottom) return 'bottom-left';
  if (right && bottom) return 'bottom-right';
  if (left && centerY) return 'left';
  if (right && centerY) return 'right';
  if (top && centerX) return 'top';
  if (bottom && centerX) return 'bottom';
  return null;
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
    if (resizeSideAt(event)) return;
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
    swallowNextClick();
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

// Dragging an element on an edit-mode canvas: reports the element's id once
// the pointer has moved, then each pointer position with the bounds of every
// drawn element, then the drop. A faded copy follows the pointer; Gleam
// decides where the element lands. A drag with almost no movement stays a click.
let moveRoot = null;
export function listen_move(root, started, moved, dropped, cancelled) {
  if (!root || moveRoot === root) return;
  moveRoot = root;
  const doc = root.ownerDocument;
  const body = doc?.body;
  let gesture = null;

  const canvasOf = done =>
    done.canvas.isConnected === false
      ? root.querySelector?.('[data-move]') ?? done.canvas
      : done.canvas;

  const ghostOf = (element, x, y) => {
    const rect = element.getBoundingClientRect();
    const ghost = element.cloneNode(true);
    for (const marked of [ghost, ...(ghost.querySelectorAll?.('[data-design-id]') ?? [])]) {
      marked.removeAttribute?.('data-design-id');
    }
    ghost.setAttribute('data-design-ghost', '');
    ghost.style.position = 'fixed';
    ghost.style.margin = '0';
    ghost.style.width = `${rect.right - rect.left}px`;
    ghost.style.height = `${rect.bottom - rect.top}px`;
    ghost.style.opacity = '0.5';
    ghost.style.pointerEvents = 'none';
    ghost.style.zIndex = '2147483647';
    doc.body.appendChild(ghost);
    return { ghost, dx: x - rect.left, dy: y - rect.top };
  };

  const removeListeners = () => {
    globalThis.removeEventListener('pointermove', onMove);
    globalThis.removeEventListener('pointerup', onUp);
    globalThis.removeEventListener('pointercancel', onCancel);
  };

  const finish = () => {
    removeListeners();
    if (body && gesture?.dragging) body.style.userSelect = gesture.select;
    gesture?.ghost?.ghost.remove();
    const done = gesture;
    gesture = null;
    return done;
  };

  function onDown(event) {
    if (event.button !== 0 || gesture) return;
    if (resizeSideAt(event)) return;
    const canvas = event.target?.closest?.('[data-move]');
    const element = event.target?.closest?.('[data-design-id]');
    if (!canvas || !element || element === canvas) return;
    gesture = {
      canvas,
      element,
      id: element.getAttribute('data-design-id'),
      startX: event.clientX,
      startY: event.clientY,
      dragging: false,
      ghost: null,
      select: body?.style.userSelect ?? '',
    };
    globalThis.addEventListener('pointermove', onMove);
    globalThis.addEventListener('pointerup', onUp);
    globalThis.addEventListener('pointercancel', onCancel);
  }

  function onMove(event) {
    if (!gesture) return;
    const x = event.clientX;
    const y = event.clientY;
    const far = Math.max(Math.abs(x - gesture.startX), Math.abs(y - gesture.startY));
    if (!gesture.dragging && far < 4) return;
    event.preventDefault();
    if (!gesture.dragging) {
      gesture.dragging = true;
      if (body) body.style.userSelect = 'none';
      gesture.ghost = ghostOf(gesture.element, gesture.startX, gesture.startY);
      started(gesture.id);
    }
    gesture.ghost.ghost.style.left = `${x - gesture.ghost.dx}px`;
    gesture.ghost.ghost.style.top = `${y - gesture.ghost.dy}px`;
    moved(x, y, List.fromArray(elementBounds(canvasOf(gesture))));
  }

  function onUp(event) {
    if (!gesture) return;
    const canvas = canvasOf(gesture);
    const done = finish();
    if (!done.dragging) return;
    swallowNextClick();
    moved(event.clientX, event.clientY, List.fromArray(elementBounds(canvas)));
    dropped();
  }

  function onCancel() {
    const done = finish();
    if (done?.dragging) cancelled();
  }

  root.addEventListener('pointerdown', onDown);
}

// Dragging a selected element's side or corner in edit mode: reports its id,
// side and drawn size as the pointer moves, then the drop. A drag with almost
// no movement stays a click.
let resizeRoot = null;
export function listen_resize(root, resized, dropped, cancelled) {
  if (!root || resizeRoot === root) return;
  resizeRoot = root;
  const doc = root.ownerDocument;
  const body = doc?.body;
  let gesture = null;
  let hovered = null;

  const sizeOf = (done, x, y) => {
    const dx = x - done.startX;
    const dy = y - done.startY;
    let width = done.width;
    let height = done.height;
    if (done.side.includes('left')) width -= dx;
    if (done.side.includes('right')) width += dx;
    if (done.side.includes('top')) height -= dy;
    if (done.side.includes('bottom')) height += dy;
    return { width, height };
  };

  const removeListeners = () => {
    globalThis.removeEventListener('pointermove', onMove);
    globalThis.removeEventListener('pointerup', onUp);
    globalThis.removeEventListener('pointercancel', onCancel);
  };

  const finish = () => {
    removeListeners();
    if (body && gesture?.dragging) body.style.userSelect = gesture.select;
    const done = gesture;
    gesture = null;
    return done;
  };

  function onDown(event) {
    if (event.button !== 0 || gesture) return;
    const side = resizeSideAt(event);
    if (!side) return;
    const element = event.target.closest('[data-resize]');
    const rect = element.getBoundingClientRect();
    gesture = {
      element,
      id: element.getAttribute('data-design-id'),
      side,
      startX: event.clientX,
      startY: event.clientY,
      width: rect.right - rect.left,
      height: rect.bottom - rect.top,
      dragging: false,
      select: body?.style.userSelect ?? '',
    };
    globalThis.addEventListener('pointermove', onMove);
    globalThis.addEventListener('pointerup', onUp);
    globalThis.addEventListener('pointercancel', onCancel);
  }

  function onMove(event) {
    if (!gesture) return;
    const far = Math.max(
      Math.abs(event.clientX - gesture.startX),
      Math.abs(event.clientY - gesture.startY),
    );
    if (!gesture.dragging && far < 4) return;
    event.preventDefault();
    if (!gesture.dragging) {
      gesture.dragging = true;
      if (body) body.style.userSelect = 'none';
    }
    const size = sizeOf(gesture, event.clientX, event.clientY);
    resized(gesture.id, gesture.side, size.width, size.height);
  }

  function onUp(event) {
    if (!gesture) return;
    const done = finish();
    if (!done.dragging) return;
    swallowNextClick();
    const size = sizeOf(done, event.clientX, event.clientY);
    resized(done.id, done.side, size.width, size.height);
    dropped();
  }

  function onCancel() {
    const done = finish();
    if (done?.dragging) cancelled();
  }

  function onHover(event) {
    if (gesture) return;
    const element = event.target?.closest?.('[data-resize]') ?? null;
    if (hovered && hovered !== element && hovered.style) hovered.style.cursor = '';
    hovered = element;
    if (!element?.style) return;
    const side = resizeSideAt(event);
    element.style.cursor = side ? resizeCursors[side] : '';
  }

  root.addEventListener('pointerdown', onDown);
  root.addEventListener('pointermove', onHover);
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
  if (!box || !canvas || !id || box.hasAttribute('data-moved')) return;
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
