// The window's scroll for each page. Pages keep their own state by staying mounted (hidden while another
// shows); the window's scroll is shared, so the shell keeps it per page. Outside a browser these do nothing.
const saved = new Map();

export function leave(page) {
  if (typeof window === 'undefined') return;
  saved.set(page, [window.scrollX, window.scrollY]);
}

export function enter(page) {
  if (typeof window === 'undefined') return;
  const [x, y] = saved.get(page) || [0, 0];
  window.scrollTo(x, y);
}
