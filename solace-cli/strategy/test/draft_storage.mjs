// A browser's localStorage for the Gleam tests: each install starts empty.
// components and strategy each keep a copy: Gleam does not build a dependency's test files.
const values = new Map();
let previous;

export function install() {
  values.clear();
  previous = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: key => values.get(key) ?? null,
      setItem: (key, value) => values.set(key, String(value)),
      removeItem: key => values.delete(key),
    },
  });
}

export function restore() {
  values.clear();
  if (previous) Object.defineProperty(globalThis, 'localStorage', previous);
  else delete globalThis.localStorage;
  previous = undefined;
}