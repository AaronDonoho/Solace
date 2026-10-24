// The shell's history entries (D-189): a record's page focus is kept with its entry.
export function navigate(path, query, state, move) {
  if (typeof globalThis.history === 'undefined') return;
  const url = query ? path + '?' + query : path;
  globalThis.history[move === 'replace' ? 'replaceState' : 'pushState'](JSON.parse(state), '', url);
}

export function state() {
  if (typeof globalThis.history === 'undefined') return '{}';
  return JSON.stringify(globalThis.history.state ?? {});
}
