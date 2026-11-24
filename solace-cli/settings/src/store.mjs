// The settings store. Each setting is kept in the browser's localStorage under
// its key; storage may be missing or blocked, and the pages work without it.
// A change is published to whoever subscribed to its key: in this window by a
// `solace-setting` event on the document, in other windows by `storage`.
// In the desktop app each setting is also kept by the app, which gives it to
// every board window it opens: a page's storage belongs to its port, and a
// board may get another port next time.
const event = 'solace-setting';

export function load(key) {
  try { return globalThis.localStorage?.getItem(key) ?? ''; } catch { return ''; }
}
function keep(key, value) {
  try { value ? localStorage.setItem(key, value) : localStorage.removeItem(key); } catch {}
  if (desktop_app()) {
    try { globalThis.__TAURI__.core.invoke('save_setting', { key, value }).catch(() => {}); } catch {}
  }
}
// Stores the value, "" to clear it, and publishes the change.
export function save(key, value) {
  keep(key, value);
  publish(key, value);
}
export function publish(key, value) {
  globalThis.document?.dispatchEvent?.(new CustomEvent(event, { detail: { key, value } }));
}
// Calls `handler` with each new value of `key`, "" when cleared; returns the
// function that unsubscribes.
export function subscribe(key, handler) {
  const here = e => { if (e.detail?.key === key) handler(e.detail.value ?? ''); };
  const other = e => { if (e.key === key) handler(e.newValue ?? ''); };
  globalThis.document?.addEventListener?.(event, here);
  globalThis.window?.addEventListener?.('storage', other);
  return () => {
    globalThis.document?.removeEventListener?.(event, here);
    globalThis.window?.removeEventListener?.('storage', other);
  };
}

// The theme: "light", "dark", or "" to follow the system. Each index.html
// applies the stored one before first paint, under this same key.
export const theme_key = 'solace-theme';
// The theme the page shows now, "" with no document.
export function theme() {
  return globalThis.document?.documentElement?.dataset?.theme ?? '';
}
export function apply_theme(value) {
  const root = globalThis.document?.documentElement;
  if (!root) return;
  root.dataset.theme = value || (globalThis.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
}
export function set_theme(value) {
  apply_theme(value);
  save(theme_key, value);
}
// The page follows a theme chosen in another window; once per page.
let following = false;
export function follow_theme() {
  if (following) return;
  following = true;
  subscribe(theme_key, apply_theme);
}

// The font size scales the whole page (D-262): every `index.html` zooms its
// root by `--font-scale`, read before first paint from the same key.
export function apply_font_scale(scale) {
  globalThis.document?.documentElement?.style?.setProperty?.('--font-scale', String(scale));
}
// Whether the page is in the desktop app's webview.
export function desktop_app() {
  return Boolean(globalThis.__TAURI__);
}
// How far the wheel turns, in pixels, for one step: a wheel notch is 50 to
// 100, so a touchpad pinch or smooth scrolling, sending many small turns,
// steps once per notch's worth.
const notch = 50;
const line = 40;
// Ctrl + mouse wheel in the desktop app calls `handler(true)` for up and
// `handler(false)` for down; the page takes the wheel, so the webview doesn't
// zoom as well. A browser keeps its own zoom. Once per page.
let wheeling = false;
export function on_ctrl_wheel(handler) {
  if (wheeling || !desktop_app()) return;
  wheeling = true;
  let turned = 0;
  globalThis.window?.addEventListener?.('wheel', e => {
    if (!e.ctrlKey) return;
    e.preventDefault();
    const delta = e.deltaMode === 1 ? e.deltaY * line : e.deltaY;
    turned = Math.sign(turned) === -Math.sign(delta) ? delta : turned + delta;
    if (Math.abs(turned) < notch) return;
    handler(turned < 0);
    turned = 0;
  }, { passive: false, capture: true });
}
