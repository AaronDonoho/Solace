// Browser transport only: state, decoding and rendering belong to Lustre.
export function request(path, method, body, receive) {
  fetch(path, { method, headers: { 'Content-Type': 'application/json' },
    ...(method === 'GET' ? {} : { body }) })
    .then(async response => receive(await response.text()))
    .catch(error => receive(JSON.stringify({ message: String(error) })));
}
export function events(receive) {
  const source = new EventSource('/events');
  source.onmessage = event => receive(event.data);
  // Native EventSource reconnects with Last-Event-ID.
  source.onerror = () => {};
}
