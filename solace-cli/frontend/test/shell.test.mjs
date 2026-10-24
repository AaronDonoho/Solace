import { test, expect } from 'bun:test';
import { guard } from '../src/shell.mjs';
import { watch_titles } from '../src/task_link.mjs';

class Node {
  constructor(tag = 'DIV', nodeType = 1) {
    this.tagName = tag;
    this.nodeType = nodeType;
    this.children = [];
    this.listeners = new Map();
    this.shadowRoot = null;
    this.isConnected = false;
    this.hidden = false;
    this.value = '';
    this.style = {};
    this.found = {};
    this.attributes = {};
    this.textContent = '';
  }
  append(...nodes) {
    for (const node of nodes) {
      this.children.push(node);
      node.parentElement = this;
    }
  }
  addEventListener(type, fn, options) {
    const capture = typeof options === 'boolean' ? options : !!options?.capture;
    this.listeners.set(type + ':' + capture, fn);
  }
  removeEventListener(type, fn, options) {
    const capture = typeof options === 'boolean' ? options : !!options?.capture;
    const key = type + ':' + capture;
    if (this.listeners.get(key) === fn) this.listeners.delete(key);
  }
  querySelector(selector) {
    return this.found[selector] ||= new Node(selector === 'input' ? 'INPUT' : 'SPAN');
  }
  querySelectorAll(selector) {
    if (selector === '*') return this.children.flatMap(child => [child, ...child.querySelectorAll('*')]);
    return this.children.flatMap(child => [
      ...(child.tagName === 'A' ? [child] : []),
      ...child.querySelectorAll(selector),
    ]);
  }
  contains(node) {
    return node === this || this.children.some(child => child.contains(node));
  }
  getRootNode() {
    return this;
  }
  setAttribute(name, value = '') {
    this.attributes[name] = value;
  }
  getAttribute(name) {
    return this.attributes[name] ?? null;
  }
  focus() {}
}

const install = () => {
  const saved = {};
  const observers = [];
  const resizes = [];
  const frames = [];
  const timers = [];
  const fetches = [];
  class MutationObserver {
    constructor(callback) {
      this.callback = callback;
      this.targets = new Set();
      observers.push(this);
    }
    observe(target) {
      this.targets.add(target);
    }
    disconnect() {
      this.targets.clear();
    }
  }
  class ResizeObserver {
    constructor(callback) {
      this.callback = callback;
      this.targets = new Set();
      resizes.push(this);
    }
    observe(target) {
      this.targets.add(target);
    }
    unobserve(target) {
      this.targets.delete(target);
    }
  }
  const body = new Node('BODY');
  body.isConnected = true;
  const values = {
    MutationObserver,
    ResizeObserver,
    document: {
      body,
      activeElement: body,
      createElement: tag => new Node(tag.toUpperCase()),
      addEventListener() {},
      getSelection: () => null,
    },
    getComputedStyle: () => ({ display: 'block' }),
    fetch: url => {
      fetches.push(url);
      return Promise.resolve({
        ok: true,
        json: () => Promise.resolve({ title: 'First task' }),
        text: () => Promise.resolve(''),
      });
    },
    addEventListener() {},
    removeEventListener() {},
    requestAnimationFrame: callback => frames.push(callback),
    cancelAnimationFrame: id => { frames[id - 1] = null; },
    setTimeout: callback => timers.push(callback),
    clearTimeout() {},
  };
  for (const [key, value] of Object.entries(values)) {
    saved[key] = globalThis[key];
    globalThis[key] = value;
  }
  return {
    body,
    observers,
    resizes,
    frames,
    timers,
    fetches,
    restore() {
      for (const [key, value] of Object.entries(saved)) {
        if (value === undefined) delete globalThis[key];
        else globalThis[key] = value;
      }
    },
  };
};

const task = () => {
  const host = new Node('SOLACE-TASK');
  const shadow = new Node('#shadow-root', 11);
  const editor = new Node('SOLACE-MD-EDITOR');
  const editorShadow = new Node('#editor-root', 11);
  shadow.append(editor);
  editor.shadowRoot = editorShadow;
  host.shadowRoot = shadow;
  return { host, editor };
};

const documentObserver = (dom, body) => dom.observers.find(observer => observer.targets.has(body));
const observed = (dom, root) => dom.observers.some(observer => observer.targets.has(root));
const flush = async dom => {
  while (dom.frames.length) dom.frames.shift()?.();
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

test('removing a task releases the find bar observers and listeners', () => {
  const dom = install();
  try {
    guard(new Node('DOCUMENT'));
    const observer = documentObserver(dom, dom.body);
    const resize = dom.resizes[0];
    for (let i = 0; i < 3; i++) {
      const { host, editor } = task();
      observer.callback([{ type: 'childList', target: dom.body, addedNodes: [host], removedNodes: [] }]);
      expect([resize.targets.has(host), observed(dom, host.shadowRoot), observed(dom, editor.shadowRoot)]).toEqual([true, true, true]);
      expect([host.shadowRoot.listeners.size, editor.shadowRoot.listeners.size]).toEqual([2, 2]);
      observer.callback([{ type: 'childList', target: dom.body, addedNodes: [], removedNodes: [host] }]);
      expect([resize.targets.has(host), observed(dom, host.shadowRoot), observed(dom, editor.shadowRoot)]).toEqual([false, false, false]);
      expect([host.shadowRoot.listeners.size, editor.shadowRoot.listeners.size]).toEqual([0, 0]);
    }
  } finally {
    dom.restore();
  }
});

test('removing an editor releases it while its task stays open', () => {
  const dom = install();
  try {
    guard(new Node('DOCUMENT'));
    const observer = documentObserver(dom, dom.body);
    const resize = dom.resizes[0];
    const { host, editor } = task();
    observer.callback([{ type: 'childList', target: dom.body, addedNodes: [host], removedNodes: [] }]);
    const inside = dom.observers.find(o => o.targets.has(host.shadowRoot));
    inside.callback([{ type: 'childList', target: host.shadowRoot, addedNodes: [], removedNodes: [editor] }]);
    expect([resize.targets.has(host), observed(dom, host.shadowRoot), observed(dom, editor.shadowRoot)]).toEqual([true, true, false]);
    expect([host.shadowRoot.listeners.size, editor.shadowRoot.listeners.size]).toEqual([2, 0]);
  } finally {
    dom.restore();
  }
});

test('removing a task releases the title observer', () => {
  const dom = install();
  try {
    watch_titles();
    const observer = documentObserver(dom, dom.body);
    for (let i = 0; i < 3; i++) {
      const { host, editor } = task();
      observer.callback([{ type: 'childList', target: dom.body, addedNodes: [host], removedNodes: [] }]);
      expect([observed(dom, host.shadowRoot), observed(dom, editor.shadowRoot)]).toEqual([true, true]);
      observer.callback([{ type: 'childList', target: dom.body, addedNodes: [], removedNodes: [host] }]);
      expect([observed(dom, host.shadowRoot), observed(dom, editor.shadowRoot)]).toEqual([false, false]);
    }
  } finally {
    dom.restore();
  }
});

test('a title is asked for again once its cache expires', async () => {
  const dom = install();
  try {
    watch_titles();
    const observer = documentObserver(dom, dom.body);
    const id = '1111111111111111';
    const link = () => {
      const el = new Node('A');
      el.textContent = 'task:' + id;
      el.setAttribute('href', '/task/' + id);
      el.setAttribute('data-task', '');
      return el;
    };
    const add = async el => {
      dom.body.append(el);
      observer.callback([{ type: 'childList', target: dom.body, addedNodes: [el], removedNodes: [] }]);
      await flush(dom);
    };
    const first = link();
    await add(first);
    expect(dom.fetches.length).toBe(1);
    expect(first.textContent).toBe('First task');
    expect(first.getAttribute('data-bare')).toBe('');
    const cached = link();
    await add(cached);
    expect(dom.fetches.length).toBe(1);
    expect(cached.textContent).toBe('First task');
    dom.timers.shift()?.();
    const expired = link();
    await add(expired);
    expect(dom.fetches.length).toBe(2);
    expect(expired.textContent).toBe('First task');
  } finally {
    dom.restore();
  }
});
