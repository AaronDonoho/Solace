import { test, expect } from 'bun:test';
import {
  drag_note_box,
  listen_move,
  listen_region,
  listen_resize,
  place_note_box,
} from '../build/dev/javascript/design/api.mjs';

const sleep = ms => new Promise(done => setTimeout(done, ms));

function fakeRoot(select = 'auto') {
  const state = {
    rootListeners: {},
    selections: 0,
    boxes: [],
    appended: [],
  };
  state.body = {
    style: { userSelect: select },
    appendChild: box => { state.appended.push(box); },
  };
  const doc = {
    body: state.body,
    createElement: () => {
      const box = {
        style: {},
        attributes: {},
        removed: false,
        setAttribute(name, value) { box.attributes[name] = value; },
        remove() { box.removed = true; },
      };
      state.boxes.push(box);
      return box;
    },
    getSelection: () => ({ removeAllRanges: () => { state.selections++; } }),
  };
  const root = {
    ownerDocument: doc,
    addEventListener: (name, handler) => { state.rootListeners[name] = handler; },
    getSelection: () => ({ removeAllRanges: () => { state.selections++; } }),
  };
  return { root, state };
}

function fakeCanvas() {
  const element = (id, left, top, right, bottom) => ({
    getAttribute: name => name === 'data-design-id' ? id : null,
    getBoundingClientRect: () => ({ left, top, right, bottom }),
  });
  const elements = [
    element('screen', 10, 20, 110, 70),
    element('title', 20, 30, 80, 40),
  ];
  return {
    querySelectorAll: () => elements,
    getClientRects: () => [{ width: 100, height: 50 }],
  };
}

function fakeGlobalListeners() {
  const native = {
    add: globalThis.addEventListener,
    remove: globalThis.removeEventListener,
  };
  const listeners = [];
  globalThis.addEventListener = (name, handler, options) => {
    listeners.push({ name, handler, options });
  };
  globalThis.removeEventListener = (name, handler, options) => {
    const index = listeners.findIndex(
      listener =>
        listener.name === name &&
        listener.handler === handler &&
        listener.options === options,
    );
    if (index >= 0) listeners.splice(index, 1);
  };
  const restore = () => {
    for (const [name, value] of [
      ['addEventListener', native.add],
      ['removeEventListener', native.remove],
    ]) {
      if (value === undefined) delete globalThis[name];
      else globalThis[name] = value;
    }
  };
  return {
    listeners,
    find: name => listeners.find(listener => listener.name === name),
    restore,
  };
}

function fakeNoteDrag() {
  const { root, state } = fakeRoot('text');
  state.root = root;
  const canvas = {
    scrollLeft: 10,
    scrollTop: 20,
    scrollWidth: 400,
    scrollHeight: 500,
    getBoundingClientRect: () => ({ left: 50, top: 100 }),
  };
  const handle = {
    closest: selector => selector === '[data-note-box]' ? state.box : null,
  };
  state.box = {
    style: {},
    attributes: {},
    offsetWidth: 120,
    offsetHeight: 80,
    closest: selector => selector === '.design-canvas' ? canvas : null,
    getBoundingClientRect: () => ({ left: 100, top: 150 }),
    setAttribute(name, value) { this.attributes[name] = value; },
  };
  state.handle = handle;
  return state;
}

function noteDown(state, x, y, extra = {}) {
  state.rootListeners.pointerdown({
    button: 0,
    clientX: x,
    clientY: y,
    preventDefault: () => {},
    target: { closest: () => state.handle },
    ...extra,
  });
}

function down(state, canvas, x, y, extra = {}) {
  state.rootListeners.pointerdown({
    button: 0,
    clientX: x,
    clientY: y,
    ctrlKey: false,
    metaKey: false,
    target: { closest: () => canvas },
    ...extra,
  });
}

test('a drag reports its corners and the bounds of every drawn element', async () => {
  const { root, state } = fakeRoot();
  const canvas = fakeCanvas();
  const globals = fakeGlobalListeners();
  const reports = [];
  try {
    listen_region(root, (...args) => reports.push(args));

    down(state, canvas, 10, 10);
    expect(state.body.style.userSelect).toBe('none');
    expect(state.selections).toBe(2);

    let prevented = 0;
    globals.find('pointermove').handler({
      clientX: 50,
      clientY: 40,
      preventDefault: () => { prevented++; },
    });
    expect(prevented).toBe(1);
    expect(state.boxes.length).toBe(1);
    const box = state.boxes[0];
    expect(box.attributes['data-design-region']).toBe('');
    expect(box.style).toMatchObject({
      left: '10px',
      top: '10px',
      width: '40px',
      height: '30px',
    });

    globals.find('pointerup').handler({ clientX: 60, clientY: 45 });

    expect(reports.length).toBe(1);
    const [startX, startY, endX, endY, bounds, multi] = reports[0];
    expect([startX, startY, endX, endY, multi]).toEqual([10, 10, 60, 45, false]);
    expect(bounds.toArray()).toEqual([
      ['screen', 10, 20, 110, 70],
      ['title', 20, 30, 80, 40],
    ]);
    expect(box.removed).toBe(true);
    expect(state.body.style.userSelect).toBe('auto');
    expect(globals.listeners.map(listener => listener.name))
      .not.toContain('pointermove');

    const click = globals.find('click');
    expect(click.options).toBe(true);
    let clickPrevented = 0;
    let clickStopped = 0;
    click.handler({
      preventDefault: () => { clickPrevented++; },
      stopPropagation: () => { clickStopped++; },
    });
    expect([clickPrevented, clickStopped]).toEqual([1, 1]);

    await sleep(10);
    expect(globals.find('click')).toBeUndefined();
  } finally {
    globals.restore();
  }
});

function fakeMoveTarget(canvas) {
  const ghost = {
    style: {},
    attributes: { 'data-design-id': 'title' },
    removed: false,
    querySelectorAll: () => [],
    setAttribute(name, value) { ghost.attributes[name] = value; },
    removeAttribute(name) { delete ghost.attributes[name]; },
    remove() { ghost.removed = true; },
  };
  const element = {
    getAttribute: name => name === 'data-design-id' ? 'title' : null,
    getBoundingClientRect: () => ({ left: 20, top: 30, right: 80, bottom: 40 }),
    cloneNode: () => ghost,
  };
  const target = {
    closest: selector => selector === '[data-move]' ? canvas
      : selector === '[data-design-id]' ? element : null,
  };
  return { ghost, target };
}

function fakeResizeTarget() {
  const element = {
    style: {},
    getAttribute: name => name === 'data-design-id' ? 'title' : null,
    getBoundingClientRect: () => ({ left: 20, top: 30, right: 80, bottom: 90 }),
  };
  const target = {
    closest: selector => selector === '[data-resize]' ? element : null,
  };
  return { element, target };
}

test('an element drag reports its start, each position and the drop', async () => {
  const { root, state } = fakeRoot();
  const canvas = fakeCanvas();
  const { ghost, target } = fakeMoveTarget(canvas);
  const globals = fakeGlobalListeners();
  const reports = [];
  try {
    listen_move(
      root,
      id => reports.push(['started', id]),
      (x, y, bounds) => reports.push(['moved', x, y, bounds.toArray()]),
      () => reports.push(['dropped']),
      () => reports.push(['cancelled']),
    );

    state.rootListeners.pointerdown({ button: 0, clientX: 30, clientY: 35, target });
    globals.find('pointermove').handler({ clientX: 31, clientY: 35, preventDefault: () => {} });
    expect(reports).toEqual([]);

    globals.find('pointermove').handler({ clientX: 50, clientY: 60, preventDefault: () => {} });
    expect(reports[0]).toEqual(['started', 'title']);
    expect(reports[1][0]).toBe('moved');
    expect(reports[1].slice(1, 3)).toEqual([50, 60]);
    expect(reports[1][3]).toEqual([
      ['screen', 10, 20, 110, 70],
      ['title', 20, 30, 80, 40],
    ]);
    expect(state.appended).toEqual([ghost]);
    expect(ghost.attributes['data-design-id']).toBeUndefined();
    expect(ghost.style).toMatchObject({ left: '40px', top: '55px', opacity: '0.5', pointerEvents: 'none' });
    expect(state.body.style.userSelect).toBe('none');

    globals.find('pointerup').handler({ clientX: 52, clientY: 62 });
    expect(reports.slice(2).map(report => report.slice(0, 3))).toEqual([['moved', 52, 62], ['dropped']]);
    expect(ghost.removed).toBe(true);
    expect(state.body.style.userSelect).toBe('auto');
    expect(globals.find('pointermove')).toBeUndefined();
    expect(globals.find('click').options).toBe(true);
    await sleep(10);
    expect(globals.find('click')).toBeUndefined();
  } finally {
    globals.restore();
  }
});

test('a border drag reports the element, side and drawn size', () => {
  const { root, state } = fakeRoot();
  const { element, target } = fakeResizeTarget();
  const globals = fakeGlobalListeners();
  const reports = [];
  try {
    listen_resize(
      root,
      (id, side, width, height) => reports.push([id, side, width, height]),
      () => reports.push(['dropped']),
      () => reports.push(['cancelled']),
    );

    state.rootListeners.pointermove({ clientX: 80, clientY: 60, target });
    expect(element.style.cursor).toBe('ew-resize');
    state.rootListeners.pointerdown({ button: 0, clientX: 80, clientY: 60, target });
    globals.find('pointermove').handler({ clientX: 100, clientY: 60, preventDefault: () => {} });
    expect(reports).toEqual([['title', 'right', 80, 60]]);

    globals.find('pointerup').handler({ clientX: 100, clientY: 60 });
    expect(reports).toEqual([
      ['title', 'right', 80, 60],
      ['title', 'right', 80, 60],
      ['dropped'],
    ]);
    expect(globals.find('click').options).toBe(true);
  } finally {
    globals.restore();
  }
});

test('a press away from a handle starts no border drag', () => {
  const { root, state } = fakeRoot();
  const { target } = fakeResizeTarget();
  const globals = fakeGlobalListeners();
  try {
    listen_resize(root, () => {}, () => {}, () => {});
    state.rootListeners.pointerdown({ button: 0, clientX: 50, clientY: 60, target });
    expect(globals.find('pointermove')).toBeUndefined();
  } finally {
    globals.restore();
  }
});

test('an element press without movement stays a click, and a cancel reports no drop', () => {
  const { root, state } = fakeRoot();
  const canvas = fakeCanvas();
  const { target } = fakeMoveTarget(canvas);
  const globals = fakeGlobalListeners();
  const reports = [];
  try {
    listen_move(
      root,
      id => reports.push(['started', id]),
      () => reports.push(['moved']),
      () => reports.push(['dropped']),
      () => reports.push(['cancelled']),
    );
    state.rootListeners.pointerdown({ button: 0, clientX: 30, clientY: 35, target });
    globals.find('pointerup').handler({ clientX: 31, clientY: 35 });
    expect(reports).toEqual([]);
    expect(globals.find('click')).toBeUndefined();

    state.rootListeners.pointerdown({ button: 0, clientX: 30, clientY: 35, target });
    globals.find('pointermove').handler({ clientX: 60, clientY: 35, preventDefault: () => {} });
    globals.find('pointercancel').handler({});
    expect(reports.map(report => report[0])).toEqual(['started', 'moved', 'cancelled']);
  } finally {
    globals.restore();
  }
});

test('a press outside an edit-mode canvas starts no element drag', () => {
  const { root, state } = fakeRoot();
  const globals = fakeGlobalListeners();
  try {
    listen_move(root, () => {}, () => {}, () => {}, () => {});
    state.rootListeners.pointerdown({ button: 0, clientX: 1, clientY: 1, target: { closest: () => null } });
    expect(globals.find('pointermove')).toBeUndefined();
  } finally {
    globals.restore();
  }
});

test('a movement under the threshold stays a click', () => {
  const { root, state } = fakeRoot();
  const canvas = fakeCanvas();
  const globals = fakeGlobalListeners();
  const reports = [];
  try {
    listen_region(root, (...args) => reports.push(args));

    down(state, canvas, 10, 10);
    let prevented = 0;
    globals.find('pointermove').handler({
      clientX: 12,
      clientY: 11,
      preventDefault: () => { prevented++; },
    });
    expect(prevented).toBe(0);
    expect(state.boxes.length).toBe(0);

    globals.find('pointerup').handler({ clientX: 12, clientY: 11 });
    expect(reports.length).toBe(0);
    expect(state.body.style.userSelect).toBe('auto');
    expect(globals.find('click')).toBeUndefined();
  } finally {
    globals.restore();
  }
});

test('dragging the note handle moves the box and marks it moved', () => {
  const state = fakeNoteDrag();
  const globals = fakeGlobalListeners();
  let moves = 0;
  try {
    drag_note_box(state.root, () => { moves++; });
    noteDown(state, 100, 150);
    expect(state.body.style.userSelect).toBe('text');

    let prevented = 0;
    globals.find('pointermove').handler({
      clientX: 102,
      clientY: 151,
      preventDefault: () => { prevented++; },
    });
    expect(prevented).toBe(0);
    expect(moves).toBe(0);
    expect(state.box.style).toEqual({});

    globals.find('pointermove').handler({
      clientX: 130,
      clientY: 170,
      preventDefault: () => { prevented++; },
    });
    expect(prevented).toBe(1);
    expect(moves).toBe(1);
    expect(state.body.style.userSelect).toBe('none');
    expect(state.box.style).toMatchObject({
      left: '90px',
      top: '90px',
      right: 'auto',
    });
    expect(state.box.attributes['data-moved']).toBe('');

    globals.find('pointermove').handler({
      clientX: 1000,
      clientY: 1000,
      preventDefault: () => {},
    });
    expect(state.box.style).toMatchObject({ left: '272px', top: '412px' });

    globals.find('pointerup').handler();
    expect(state.body.style.userSelect).toBe('text');
    expect(globals.listeners.map(listener => listener.name))
      .not.toContain('pointermove');
  } finally {
    globals.restore();
  }
});

test('only a left press on the note handle starts a drag', () => {
  const state = fakeNoteDrag();
  const globals = fakeGlobalListeners();
  let moves = 0;
  try {
    drag_note_box(state.root, () => { moves++; });

    noteDown(state, 100, 150, { button: 1 });
    expect(globals.find('pointermove')).toBeUndefined();
    expect(state.body.style.userSelect).toBe('text');

    state.rootListeners.pointerdown({
      button: 0,
      clientX: 100,
      clientY: 150,
      target: { closest: () => null },
    });
    expect(globals.find('pointermove')).toBeUndefined();

    noteDown(state, 100, 150);
    globals.find('pointermove').handler({
      clientX: 120,
      clientY: 170,
      preventDefault: () => {},
    });
    expect(moves).toBe(1);
    expect(state.body.style.userSelect).toBe('none');

    globals.find('pointercancel').handler();
    expect(moves).toBe(1);
    expect(state.body.style.userSelect).toBe('text');
    expect(globals.listeners.map(listener => listener.name))
      .not.toContain('pointermove');
  } finally {
    globals.restore();
  }
});

test('a note box moved by hand is not placed on a later selection', () => {
  const canvas = {
    scrollLeft: 0,
    scrollTop: 0,
    scrollWidth: 400,
    scrollHeight: 400,
    getBoundingClientRect: () => ({ left: 0, top: 0 }),
  };
  let moved = true;
  const box = {
    style: { left: '10px', top: '20px' },
    hasAttribute: name => name === 'data-moved' && moved,
    closest: selector => selector === '.design-canvas' ? canvas : null,
    getBoundingClientRect: () => ({ width: 120, height: 80 }),
  };
  const target = {
    getAttribute: name => name === 'data-design-id' ? 'go' : null,
    getClientRects: () => [{}],
    getBoundingClientRect: () => ({ left: 40, top: 100, height: 20 }),
  };
  const root = {
    querySelector: () => box,
    querySelectorAll: () => [target],
  };

  place_note_box(root, 'go');
  expect(box.style).toEqual({ left: '10px', top: '20px' });

  place_note_box(root, '');
  expect(box.style).toEqual({ left: '10px', top: '20px' });

  moved = false;
  place_note_box(root, 'go');
  expect(box.style).toMatchObject({ left: '40px', top: '128px' });
});

test('only a left press on a visible canvas starts a drag', async () => {
  const { root, state } = fakeRoot('text');
  const canvas = fakeCanvas();
  const globals = fakeGlobalListeners();
  const reports = [];
  try {
    listen_region(root, (...args) => reports.push(args));

    down(state, canvas, 0, 0, { button: 1 });
    expect(state.selections).toBe(0);
    expect(state.body.style.userSelect).toBe('text');

    down(state, null, 0, 0);
    expect(state.selections).toBe(0);

    down(state, { getClientRects: () => [] }, 0, 0);
    expect(state.selections).toBe(0);

    down(state, canvas, 0, 0, { ctrlKey: true });
    expect(state.body.style.userSelect).toBe('none');
    globals.find('pointermove').handler({
      clientX: 10,
      clientY: 0,
      preventDefault: () => {},
    });
    globals.find('pointerup').handler({ clientX: 10, clientY: 0 });
    expect(reports.length).toBe(1);
    expect(reports[0][5]).toBe(true);
    expect(state.body.style.userSelect).toBe('text');
    await sleep(10);

    down(state, canvas, 0, 0);
    globals.find('pointermove').handler({
      clientX: 10,
      clientY: 0,
      preventDefault: () => {},
    });
    globals.find('pointercancel').handler();
    expect(reports.length).toBe(1);
    expect(state.body.style.userSelect).toBe('text');
    expect(globals.listeners.map(listener => listener.name))
      .not.toContain('pointermove');
  } finally {
    globals.restore();
  }
});
