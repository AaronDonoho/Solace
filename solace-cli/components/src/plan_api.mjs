// The planning panel's browser helpers: a run's time, its scroll end and its
// conversation's selection.
export function clock(seconds) {
  return new Date(seconds * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}
const reach = 48;
export function near_end(body) {
  const end = body?.querySelector?.('.plan-end');
  if (!end) return true;
  return end.getBoundingClientRect().bottom - body.getBoundingClientRect().bottom < reach;
}
export function select_conversation(event) {
  if (event?.key?.toLowerCase() !== 'a') return;
  if (!(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey) return;
  const body = event.target?.closest?.('.plan-body') ?? event.currentTarget;
  if (!body) return;
  event.preventDefault();
  globalThis.getSelection?.()?.selectAllChildren(body);
}
