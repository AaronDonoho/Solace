// Browser transport for the root migration API. No project path prefix.
export function request(path, method, body, receive) {
  fetch(path, { method, headers: { 'Content-Type': 'application/json' },
    ...(method === 'GET' ? {} : { body }) })
    .then(async response => receive(await response.text()))
    .catch(error => receive(JSON.stringify({ message: String(error) })));
}

// One timer slot per component instance. A canceled callback stays inert even if
// the browser had already queued it when the component disconnected.
let nextPollId = 0;
const timers = new Map();
export function new_poll_id() { return ++nextPollId; }
export function cancel_poll(pollId) {
  const timer = timers.get(pollId);
  if (timer !== undefined) clearTimeout(timer);
  timers.delete(pollId);
}
export function after(pollId, ms, callback) {
  cancel_poll(pollId);
  const timer = setTimeout(() => {
    if (timers.get(pollId) !== timer) return;
    timers.delete(pollId);
    callback();
  }, ms);
  timers.set(pollId, timer);
}
export function clock(seconds) { return new Date(seconds * 1000).toLocaleString(); }
export function confirm(question) { return globalThis.confirm(question); }
export function reload() { globalThis.location?.reload(); }
