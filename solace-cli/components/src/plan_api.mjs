// The planning panel's browser helpers: a run's time and its scroll end.
export function clock(seconds) {
  return new Date(seconds * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}
const reach = 48;
export function near_end(body) {
  const end = body?.querySelector?.('.plan-end');
  if (!end) return true;
  return end.getBoundingClientRect().bottom - body.getBoundingClientRect().bottom < reach;
}
