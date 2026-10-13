// Task, feature and milestone links (D-98, D-122). The decisions live in frontend.gleam (route, target, update),
// where they are tested. A page's markdown draws `task:<id>` as a link to `/task/<id>` (and `feature:<id>`,
// `milestone:<id>` the same way), inside the page's shadow root, where modem's router doesn't see it; a plain
// click on one is followed here, in-page. A click with a modifier key held keeps the browser's own behaviour:
// the server serves the shell at the link's address, which resolves it on load.
const RECORD_LINK = /^\/(task|feature|milestone)\/([0-9a-f]{16})$/;

export function follow_links(follow) {
  if (typeof document === 'undefined') return;
  document.addEventListener('click', e => {
    if (e.defaultPrevented || e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;
    for (const el of e.composedPath?.() ?? []) {
      const found = el.tagName === 'A' && el.getAttribute('href')?.match(RECORD_LINK);
      if (!found) continue;
      e.preventDefault();
      follow(found[1], found[2]);
      return;
    }
  });
}

// Record `id` of `kind`: the server's reply, or an error's message.
export function find_record(kind, id, receive) {
  fetch('/' + kind + 's/find/' + encodeURIComponent(id))
    .then(response => response.text())
    .then(receive)
    .catch(error => receive(JSON.stringify({ message: String(error) })));
}

// A bare link (`task:<id>` as its own text) shows its record's title once the server gives it (D-122), and is
// marked `data-bare`, so copying it still gives the bare reference. A labelled link keeps its label. Titles
// are kept a short while, so a page full of the same link asks once and a renamed record shows its new title.
// The links live in the pages' shadow roots: each open root gets an observer, as the find bar's do (shell.mjs).
const TITLE_MS = 30000;
const titles = new Map();
const title = (kind, id) => {
  const key = kind + ':' + id, kept = titles.get(key);
  if (kept && Date.now() - kept.at < TITLE_MS) return kept.title;
  const asked = fetch('/' + kind + 's/find/' + id)
    .then(response => (response.ok ? response.json() : null))
    .then(record => (typeof record?.title === 'string' && record.title.trim() ? record.title : null))
    .catch(() => { titles.delete(key); return null; });
  titles.set(key, { at: Date.now(), title: asked });
  return asked;
};

function fill(root) {
  for (const link of root.querySelectorAll('a[data-task], a[data-feature], a[data-milestone]')) {
    const found = link.getAttribute('href')?.match(RECORD_LINK);
    if (!found || link.textContent !== found[1] + ':' + found[2]) continue;
    title(found[1], found[2]).then(text => {
      // The link may have been drawn again meanwhile, with other text.
      if (text === null || link.textContent !== found[1] + ':' + found[2]) return;
      link.textContent = text;
      link.setAttribute('data-bare', '');
      link.title = found[1] + ':' + found[2];
    });
  }
}

export function watch_titles() {
  if (typeof document === 'undefined' || typeof MutationObserver === 'undefined') return;
  const watched = new WeakSet(), pending = new Set();
  let frame = 0;
  const later = root => {
    pending.add(root);
    frame ||= requestAnimationFrame(() => {
      frame = 0;
      for (const r of pending) fill(r);
      pending.clear();
    });
  };
  const observer = new MutationObserver(records => {
    for (const r of records) {
      for (const n of r.addedNodes) if (n.nodeType === 1) watch(n);
      later(r.target.getRootNode());
    }
  });
  const watch = node => {
    for (const el of node.querySelectorAll ? [node, ...node.querySelectorAll('*')] : []) {
      const shadow = el.shadowRoot;
      if (!shadow || watched.has(shadow)) continue;
      watched.add(shadow);
      observer.observe(shadow, { childList: true, subtree: true, characterData: true });
      watch(shadow);
      later(shadow);
    }
  };
  observer.observe(document.body, { childList: true, subtree: true });
  watch(document.body);
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
