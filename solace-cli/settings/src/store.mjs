// The settings store. Each setting is kept in the browser's localStorage under
// its key; storage may be missing or blocked, and the pages work without it.
// A change is published to whoever subscribed to its key: in this window by a
// `solace-setting` event on the document, in other windows by `storage`.
const event = 'solace-setting';

export function load(key) {
  try { return globalThis.localStorage?.getItem(key) ?? ''; } catch { return ''; }
}
function keep(key, value) {
  try { value ? localStorage.setItem(key, value) : localStorage.removeItem(key); } catch {}
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
