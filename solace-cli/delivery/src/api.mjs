// Browser access only: state, decoding and rendering belong to Lustre.
// Build routes are the root's, so there is no project prefix to keep.
export function request(path, method, body, receive) {
  fetch(path, { method, headers: { 'Content-Type': 'application/json' },
    ...(method === 'GET' ? {} : { body }) })
    .then(async response => receive(await response.text()))
    .catch(error => receive(JSON.stringify({ message: String(error) })));
}

// The global is injected only where the app grants IPC: a browser tab has none.
export function desktop() {
  return typeof window !== 'undefined'
    && Boolean(window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke);
}

export function open_folder(version, receive) {
  const invoke = typeof window !== 'undefined'
    && window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;
  if (!invoke) { receive(''); return; }
  invoke('open_release_folder', { version })
    .then(() => receive(''))
    .catch(error => receive(String(error)));
}
