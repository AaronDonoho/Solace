import { test, expect } from 'bun:test';
import * as store from '../src/store.mjs';
import * as api from '../src/api.mjs';

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
    // One listener each for the theme and the font size.
    expect(listeners).toBe(2);
    store.set_theme('dark');
    expect(model.dark).toBe(true);
    settings.set_font_size(20);
    expect(model.font_size).toBe(20);
    dispatch(new settings.Disconnected());
    expect(listeners).toBe(0);
    // A change while away shows when it is added again.
    store.set_theme('light');
    settings.set_font_size(24);
    expect(model.dark).toBe(true);
    expect(model.font_size).toBe(20);
    dispatch(new settings.Connected());
    expect(model.dark).toBe(false);
    expect(model.font_size).toBe(24);
    expect(listeners).toBe(2);
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
    expect(() => store.apply_font_scale(2)).not.toThrow();
  } finally {
    original ? Object.defineProperty(globalThis, 'localStorage', original) : delete globalThis.localStorage;
  }
});

// The root's style, as `index.html` zooms it by `--font-scale`.
function rootStyle() {
  const properties = new Map();
  document.documentElement.style = { setProperty: (k, v) => properties.set(k, v) };
  return properties;
}

test('a font size scales the page, is stored, and reaches every subscriber', async () => {
  const settings = await import('../build/dev/javascript/settings/settings.mjs');
  const browser = fakeBrowser();
  try {
    const scale = rootStyle();
    const heard = [];
    store.subscribe('solace-font-size', value => heard.push(value));
    settings.set_font_size(20);
    expect(scale.get('--font-scale')).toBe(String(20 / 15));
    expect(browser.stored.get('solace-font-size')).toBe('20');
    // The default clears the key and draws the page unscaled.
    settings.set_font_size(15);
    expect(scale.get('--font-scale')).toBe('1');
    expect(browser.stored.has('solace-font-size')).toBe(false);
    expect(heard).toEqual(['20', '']);
  } finally { browser.restore(); }
});

test('Ctrl + wheel steps the font size in the desktop app, and the webview does not zoom', async () => {
  const settings = await import('../build/dev/javascript/settings/settings.mjs');
  const browser = fakeBrowser();
  const wheel = (deltaY, ctrlKey = true) => {
    const e = Object.assign(new Event('wheel', { cancelable: true }), { deltaY, ctrlKey });
    window.dispatchEvent(e);
    return e.defaultPrevented;
  };
  try {
    rootStyle();
    // A browser keeps its own zoom.
    settings.follow_font_size();
    expect(wheel(-100)).toBe(false);
    globalThis.__TAURI__ = {};
    const page = settings.initial(false);
    expect(page.font_size).toBe(15);
    settings.follow_font_size();
    expect(wheel(-100)).toBe(true);
    expect(browser.stored.get('solace-font-size')).toBe('16');
    expect(wheel(100)).toBe(true);
    expect(wheel(100)).toBe(true);
    expect(browser.stored.get('solace-font-size')).toBe('14');
    for (let i = 0; i < 20; i++) wheel(-100);
    expect(browser.stored.get('solace-font-size')).toBe('36');
    for (let i = 0; i < 20; i++) wheel(100);
    expect(browser.stored.get('solace-font-size')).toBe('12');
    // A wheel without Ctrl scrolls as before.
    expect(wheel(-100, false)).toBe(false);
    expect(browser.stored.get('solace-font-size')).toBe('12');
    // Another window's choice scales this page too.
    const scale = rootStyle();
    window.dispatchEvent(storage('solace-font-size', '24'));
    expect(scale.get('--font-scale')).toBe(String(24 / 15));
    // A pinch's small turns step once per notch's worth, not once each.
    browser.stored.set('solace-font-size', '20');
    for (let i = 0; i < 4; i++) expect(wheel(-10)).toBe(true);
    expect(browser.stored.get('solace-font-size')).toBe('20');
    wheel(-10);
    expect(browser.stored.get('solace-font-size')).toBe('24');
    // Turning back starts over: the turn up so far doesn't cancel it.
    for (let i = 0; i < 4; i++) wheel(-10);
    wheel(60);
    expect(browser.stored.get('solace-font-size')).toBe('20');
    wheel(-60);
    expect(browser.stored.get('solace-font-size')).toBe('24');
    // A wheel that counts in lines steps once a notch.
    const lines = Object.assign(new Event('wheel', { cancelable: true }), { deltaY: -3, deltaMode: 1, ctrlKey: true });
    window.dispatchEvent(lines);
    expect(browser.stored.get('solace-font-size')).toBe('28');
  } finally { delete globalThis.__TAURI__; browser.restore(); }
});

test('registering Settings shows only a size the dropdown offers', async () => {
  const settings = await import('../build/dev/javascript/settings/settings.mjs');
  const browser = fakeBrowser();
  try {
    const scale = rootStyle();
    browser.stored.set('solace-font-size', '500');
    settings.follow_font_size();
    expect(scale.get('--font-scale')).toBe('1');
    expect(settings.initial(false).font_size).toBe(15);
    browser.stored.set('solace-font-size', '24');
    settings.follow_font_size();
    expect(scale.get('--font-scale')).toBe(String(24 / 15));
  } finally { browser.restore(); }
});

test('in the desktop app a saved or cleared setting is also kept by the app; a browser keeps it alone', () => {
  const browser = fakeBrowser();
  const calls = [];
  try {
    store.save('solace-font-size', '20');
    expect(calls).toEqual([]);
    globalThis.__TAURI__ = { core: { invoke: (command, args) => { calls.push([command, args]); return Promise.resolve(); } } };
    store.save('solace-font-size', '24');
    store.set_theme('');
    expect(calls).toEqual([
      ['save_setting', { key: 'solace-font-size', value: '24' }],
      ['save_setting', { key: 'solace-theme', value: '' }],
    ]);
    expect(browser.stored.get('solace-font-size')).toBe('24');
    globalThis.__TAURI__ = { core: { invoke: () => Promise.reject(new Error('refused')) } };
    expect(() => store.save('solace-theme', 'dark')).not.toThrow();
    expect(browser.stored.get('solace-theme')).toBe('dark');
  } finally { delete globalThis.__TAURI__; browser.restore(); }
});

test('the skills link scrolls the page\'s own Skills section into view', () => {
  const calls = [];
  const root = {
    querySelector: selector => ({ scrollIntoView: options => calls.push([selector, options]) }),
  };
  api.scroll_into_view(root, '#skills');
  expect(calls).toEqual([['#skills', { block: 'start' }]]);
  expect(() => api.scroll_into_view(undefined, '#skills')).not.toThrow();
  expect(() => api.scroll_into_view({ querySelector: () => null }, '#skills')).not.toThrow();
});
