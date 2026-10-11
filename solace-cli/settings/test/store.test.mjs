import { test, expect } from 'bun:test';
import * as store from '../src/store.mjs';

// A browser without a DOM: a document and a window that carry events, and
// storage kept in a map.
function fakeBrowser() {
  const saved = ['document', 'window', 'matchMedia'].map(k => [k, Object.getOwnPropertyDescriptor(globalThis, k)]);
  const savedStorage = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  const stored = new Map();
  globalThis.document = Object.assign(new EventTarget(), { documentElement: { dataset: {} } });
  globalThis.window = new EventTarget();
  globalThis.matchMedia = () => ({ matches: true });
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: k => stored.get(k) ?? null, setItem: (k, v) => stored.set(k, String(v)), removeItem: k => stored.delete(k) } });
  const restore = () => {
    for (const [k, d] of saved) d ? Object.defineProperty(globalThis, k, d) : delete globalThis[k];
    savedStorage ? Object.defineProperty(globalThis, 'localStorage', savedStorage) : delete globalThis.localStorage;
  };
  return { stored, restore };
}
const storage = (key, newValue) => Object.assign(new Event('storage'), { key, newValue });

test('setting the theme shows it, stores it and tells subscribers once', () => {
  const browser = fakeBrowser();
  try {
    const heard = [];
    const stop = store.subscribe('solace-theme', value => heard.push(value));
    store.subscribe('other', value => heard.push('other:' + value));
    store.set_theme('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(browser.stored.get('solace-theme')).toBe('dark');
    expect(store.theme()).toBe('dark');
    store.set_theme('light');
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(store.load('solace-theme')).toBe('light');
    expect(heard).toEqual(['dark', 'light']);
    // Unset follows the system, which is dark here, and clears the key.
    store.set_theme('');
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(browser.stored.has('solace-theme')).toBe(false);
    stop();
    store.set_theme('light');
    expect(heard).toEqual(['dark', 'light', '']);
  } finally { browser.restore(); }
});

test('a change made in another window reaches subscribers, and the page follows the theme', () => {
  const browser = fakeBrowser();
  try {
    const heard = [];
    store.subscribe('solace-theme', value => heard.push(value));
    store.follow_theme();
    window.dispatchEvent(storage('solace-theme', 'dark'));
    window.dispatchEvent(storage('something-else', 'x'));
    expect(heard).toEqual(['dark']);
    expect(document.documentElement.dataset.theme).toBe('dark');
    window.dispatchEvent(storage('solace-theme', 'light'));
    expect(document.documentElement.dataset.theme).toBe('light');
  } finally { browser.restore(); }
});

test('the page listens while it is in the document, once however often it is added', async () => {
  const settings = await import('../build/dev/javascript/settings/settings.mjs');
  const { perform } = await import('../build/dev/javascript/lustre/lustre/effect.mjs');
  const browser = fakeBrowser();
  // Count the document's live `solace-setting` listeners.
  let listeners = 0;
  const add = document.addEventListener.bind(document), remove = document.removeEventListener.bind(document);
  document.addEventListener = (name, f) => { if (name === 'solace-setting') listeners++; add(name, f); };
  document.removeEventListener = (name, f) => { if (name === 'solace-setting') listeners--; remove(name, f); };
  try {
    const noop = () => {};
    let model;
    const dispatch = msg => { let effect; [model, effect] = settings.update(model, msg); perform(effect, dispatch, noop, noop, noop, noop, noop, noop); };
    [model] = settings.init();
    expect(listeners).toBe(0);
    dispatch(new settings.Connected());
    dispatch(new settings.Connected());
    expect(listeners).toBe(1);
    store.set_theme('dark');
    expect(model.dark).toBe(true);
    dispatch(new settings.Disconnected());
    expect(listeners).toBe(0);
    // A change while away shows when it is added again.
    store.set_theme('light');
    expect(model.dark).toBe(true);
    dispatch(new settings.Connected());
    expect(model.dark).toBe(false);
    expect(listeners).toBe(1);
  } finally { browser.restore(); }
});

test('the store survives missing or throwing storage, and no document', () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  try {
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('blocked'); } });
    expect(store.load('k')).toBe('');
    expect(() => store.save('k', 'v')).not.toThrow();
    expect(() => store.set_theme('dark')).not.toThrow();
    expect(store.theme()).toBe('');
  } finally {
    original ? Object.defineProperty(globalThis, 'localStorage', original) : delete globalThis.localStorage;
  }
});
