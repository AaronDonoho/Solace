// Task links (D-98). The decisions live in frontend.gleam (route, target, update), where they are tested.
// A page's markdown draws `task:<id>` as a link to `/task/<id>`, inside the page's shadow root, where modem's
// router doesn't see it; a plain click on one is followed here, in-page. A click with a modifier key held keeps
// the browser's own behaviour: the server serves the shell at `/task/<id>`, which resolves it on load.
const TASK_LINK = /^\/task\/([0-9a-f]{16})$/;

export function follow_links(follow) {
  if (typeof document === 'undefined') return;
  document.addEventListener('click', e => {
    if (e.defaultPrevented || e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;
    for (const el of e.composedPath?.() ?? []) {
      const id = el.tagName === 'A' && el.getAttribute('href')?.match(TASK_LINK)?.[1];
      if (!id) continue;
      e.preventDefault();
      follow(id);
      return;
    }
  });
}

// Where task `id` is: the server's reply, or an error's message.
export function find_task(id, receive) {
  fetch('/tasks/find/' + encodeURIComponent(id))
    .then(response => response.text())
    .then(receive)
    .catch(error => receive(JSON.stringify({ message: String(error) })));
}

// A message in the corner that goes by itself, read out by screen readers. Plain DOM outside #app, as the
// find bar is, so Lustre never diffs it.
const TOAST_MS = 4000;
export function toast(text) {
  if (typeof document === 'undefined') return;
  let box = document.querySelector('.toasts');
  if (!box) {
    box = document.createElement('div');
    box.className = 'toasts';
    box.setAttribute('role', 'status');
    document.body.append(box);
  }
  const note = document.createElement('div');
  note.className = 'toast';
  note.textContent = text;
  box.append(note);
  setTimeout(() => note.remove(), TOAST_MS);
}