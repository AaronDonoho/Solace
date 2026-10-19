// The Solace server's model routes, asked from the page's own origin.
export function request(path, method, body, receive) {
  fetch(path, { method, headers: { 'Content-Type': 'application/json' }, ...(method === 'GET' ? {} : { body }) })
    .then(async response => receive(await response.text()))
    .catch(error => receive(JSON.stringify({ message: String(error) })));
}
export function confirm(text) {
  return globalThis.confirm?.(text) ?? false;
}
