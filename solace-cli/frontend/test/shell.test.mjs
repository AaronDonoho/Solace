import { test, expect } from 'bun:test';
import { guard } from '../src/shell.mjs';
import { follow_links, watch_titles } from '../src/task_link.mjs';

class Node {
  constructor(tag = 'DIV', nodeType = 1, data = '') {
    this.tagName = tag;
    this.nodeType = nodeType;
    this.data = data;
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
  get firstChild() {
    return this.children[0] || null;
  }
  get nextSibling() {
    const siblings = this.parentElement?.children || [];
    const at = siblings.indexOf(this);
    return at < 0 ? null : siblings[at + 1] || null;
  }
  get childNodes() {
    return this.children;
  }
  append(...nodes) {
    for (const node of nodes) {
      this.children.push(node);
      node.parentElement = this;
    }
  }
  replaceChildren(...nodes) {
    this.children = [];
    this.append(...nodes);
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
  getBoundingClientRect() {
    return { left: 10, top: 100, right: 80, bottom: 120, width: 70, height: 20 };
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
  class Range {
    setStart(node, offset) {
      this.startContainer = node;
      this.startOffset = offset;
    }
    setEnd(node, offset) {
      this.endContainer = node;
      this.endOffset = offset;
    }
    getBoundingClientRect() {
      return { left: 10, top: 100, right: 80, bottom: 120, width: 70, height: 20 };
    }
    getClientRects() {
      return [this.getBoundingClientRect()];
    }
  }
  const body = new Node('BODY');
  body.isConnected = true;
  const documentListeners = new Map();
  const values = {
    MutationObserver,
    ResizeObserver,
    Range,
    document: {
      body,
      activeElement: body,
      createElement: tag => new Node(tag.toUpperCase()),
      addEventListener: (name, fn) => documentListeners.set(name, fn),
      getSelection: () => null,
      elementFromPoint: () => null,
    },
    getComputedStyle: () => ({ display: 'block' }),
    innerWidth: 1000,
    innerHeight: 1000,
    scrollBy() {},
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
    documentListeners,
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

test('a contract or board link inside a shadow root is followed in place', () => {
  const dom = install();
  try {
    const records = [], pages = [];
    follow_links((kind, id) => records.push([kind, id]), href => pages.push(href));
    const click = dom.documentListeners.get('click');
    const press = (href, held = {}) => {
      const el = new Node('A');
      el.setAttribute('href', href);
      let prevented = false;
      click({
        defaultPrevented: false,
        button: 0,
        ctrlKey: false,
        metaKey: false,
        shiftKey: false,
        altKey: false,
        ...held,
        composedPath: () => [el, new Node('#shadow-root', 11)],
        preventDefault() { prevented = true; },
      });
      return prevented;
    };
    expect(press('/p/web/contracts?task=t1')).toBe(true);
    expect(press('/')).toBe(true);
    expect(pages).toEqual(['/p/web/contracts?task=t1', '/']);
    expect(press('/p/web/contracts?task=t1', { ctrlKey: true })).toBe(false);
    expect(pages).toHaveLength(2);
    const id = '0123456789abcdef';
    expect(press('/task/' + id)).toBe(true);
    expect(records).toEqual([['task', id]]);
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

test('find reaches a shadow host the document observer has not watched', () => {
  const dom = install();
  try {
    guard(new Node('DOCUMENT'));
    const host = new Node('SOLACE-TASK');
    const shadow = new Node('#shadow-root', 11);
    const paragraph = new Node('P');
    paragraph.isConnected = true;
    paragraph.append(new Node('#text', 3, 'Needle in the shadow'));
    shadow.append(paragraph);
    host.shadowRoot = shadow;
    dom.body.append(host);
    const resize = dom.resizes[0];
    expect([observed(dom, shadow), resize.targets.has(host)]).toEqual([false, false]);
    const bar = dom.body.children.find(child => child.className === 'find');
    const input = bar.querySelector('input');
    input.value = 'needle';
    expect(() => input.listeners.get('input:false')()).not.toThrow();
    expect(bar.querySelector('.find-count').textContent).toBe('1 of 1');
    expect([observed(dom, shadow), resize.targets.has(host)]).toEqual([true, true]);
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
