// The first-run checklist, as `GET /setup` answers it.
let asked = 0;
export function load_setup(receive) {
  const n = ++asked;
  fetch('/setup', { cache: 'no-store' })
    .then((response) => response.text())
    .then((text) => { if (n === asked) receive(text); })
    .catch((error) => { if (n === asked) receive(JSON.stringify({ message: String(error) })); });
}

// A tool that breaks after it passed brings the checklist back.
export function recheck_setup(receive) {
  setInterval(receive, 30000);
}

// Solace installs one of its own tools (D-264); the reply is the fresh
// checklist, or the error that says why it didn't install.
export function install_tool(tool, receive) {
  fetch('/setup/install', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ tool }),
  })
    .then((response) => response.text().then((text) => (response.ok ? text : refusal(response.status, text))))
    .then(receive)
    .catch((error) => receive(JSON.stringify({ message: String(error) })));
}

// An error reply, as a refusal with a message: its own, or its status.
function refusal(status, text) {
  try {
    const reply = JSON.parse(text);
    if (typeof reply?.message === 'string' && reply.message !== '') return text;
  } catch {}
  return JSON.stringify({ message: `HTTP ${status}` });
}
