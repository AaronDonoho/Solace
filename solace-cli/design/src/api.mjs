// Browser access only: decoding and rendering belong to Lustre.
export function request(base, path, method, body, receive) {
  fetch((path.startsWith('/p/') ? '' : base) + path, { method, headers: { 'Content-Type': 'application/json' },
    ...(method === 'GET' ? {} : { body }) })
    .then(async response => receive(await response.text()))
    .catch(error => receive(JSON.stringify({ message: String(error) })));
}

export function clock(seconds) {
  return seconds ? new Date(seconds * 1000).toLocaleString() : '';
}

export function stamp(seconds) {
  return seconds ? new Date(seconds * 1000).toISOString() : '';
}
