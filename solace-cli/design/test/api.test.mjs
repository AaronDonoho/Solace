import { test, expect } from 'bun:test';
import { listen_region } from '../build/dev/javascript/design/api.mjs';

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
