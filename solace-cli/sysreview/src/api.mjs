// Browser access only: state, decoding and rendering belong to Lustre.
// System review routes are the root's, so there is no project prefix to keep.
// The reply's status goes with its text, so Lustre can tell an error from a success;
// a request that got no reply has status 0.
export function request(path, method, body, receive) {
  fetch(path, { method, headers: { 'Content-Type': 'application/json' }, body: method === 'GET' ? undefined : body })
    .then(async response => receive(response.status, await response.text()))
    .catch(error => receive(0, JSON.stringify({ message: String(error) })));
}

export function clock(seconds) {
  return new Date(seconds * 1000).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}
