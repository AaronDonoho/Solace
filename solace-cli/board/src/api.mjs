// Browser transport only: state, decoding and rendering belong to Lustre.
// A workspace project's contract view is at `/p/<dir>/view` and asks its own
// project (D-57); the board passes each lane's prefix itself.
const page = globalThis.location?.pathname.match(/^(.*)\/view$/)?.[1] ?? '';
export function request(path, method, body, receive) {
  fetch((path === '/migration' || path.startsWith('/migration/') ? '' : page) + path, { method, headers: { 'Content-Type': 'application/json' },
    ...(method === 'GET' ? {} : { body }) })
    .then(async response => receive(await response.text()))
    .catch(error => receive(JSON.stringify({ message: String(error) })));
}
export function events(receive, ready = () => {}) {
  const source = new EventSource(page + '/events');
  source.onmessage = event => receive(event.data);
  source.addEventListener('ready', () => ready());
  // Native EventSource reconnects with Last-Event-ID.
  source.onerror = () => {};
}
export function after(ms, callback) {
  setTimeout(callback, ms);
}
export function clock(seconds) {
  return seconds ? new Date(seconds * 1000).toLocaleString() : '';
}
// Agent text is untrusted: everything is escaped before the few markdown forms
// (code blocks, headings, lists, `code`, **bold**, images uploaded to this task
// board, and http(s) links) are turned into HTML. Image URLs are stored
// relative to their project; `base` is the project's prefix on this server.
export function markdown(text, base = '') {
  const escape = s => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  const inline = s => escape(s).replace(/`([^`]+)`/g, '<code>$1</code>').replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/!\[([^\]]*)\]\((\/tasks\/[\w-]+\/files\/[0-9a-f]{16}\.(?:png|jpg|gif|webp))\)/g, `<img alt="$1" src="${escape(base)}$2">`)
    .replace(/(^|[^!])\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, '$1<a href="$3" rel="noopener noreferrer">$2</a>');
  const out = [];
  const parts = text.split(/^```[^\n]*\n?/m);
  parts.forEach((part, i) => {
    if (i % 2) { out.push(`<pre><code>${escape(part.replace(/\n$/, ''))}</code></pre>`); return; }
    for (const block of part.split(/\n\s*\n/)) {
      const lines = block.split('\n').filter(l => l.trim());
      if (!lines.length) continue;
      const heading = lines.length === 1 && lines[0].match(/^(#{1,4})\s+(.*)/);
      if (heading) out.push(`<h4>${inline(heading[2])}</h4>`);
      else if (lines.every(l => /^\s*[-*]\s+/.test(l))) out.push(`<ul>${lines.map(l => `<li>${inline(l.replace(/^\s*[-*]\s+/, ''))}</li>`).join('')}</ul>`);
      else if (lines.every(l => /^\s*\d+[.)]\s+/.test(l))) out.push(`<ol>${lines.map(l => `<li>${inline(l.replace(/^\s*\d+[.)]\s+/, ''))}</li>`).join('')}</ol>`);
      else out.push(`<p>${lines.map(inline).join('<br>')}</p>`);
    }
  });
  return out.join('');
}

export function reload() {
  globalThis.location?.reload();
}

export function confirm(question) {
  return globalThis.confirm(question);
}

// Browser storage may be missing or blocked; the board works without it.
export function load(key) {
  try { return globalThis.localStorage?.getItem(key) ?? ''; } catch { return ''; }
}
export function store(key, value) {
  try { value ? localStorage.setItem(key, value) : localStorage.removeItem(key); } catch {}
}

// Wraps the markdown editor `id`'s selection (or appends, without one).
export function wrap(id, text, before, after) {
  const el = globalThis.document?.getElementById(id);
  const start = el?.selectionStart ?? text.length, end = el?.selectionEnd ?? text.length;
  return text.slice(0, start) + before + text.slice(start, end) + after + text.slice(end);
}

// Images pasted, dropped or picked into a markdown editor are uploaded to its
// task; `insert` gets the editor's id and each image's markdown.
export function images(insert, fail) {
  if (!globalThis.document) return;
  const take = (event, files, task, editor, base = '') => {
    const list = [...(files || [])].filter(f => f.type.startsWith('image/'));
    if (!list.length || !task) return;
    event.preventDefault();
    for (const file of list) {
      fetch(`${base}/tasks/${task}/files`, { method: 'POST', body: file })
        .then(async response => {
          const v = await response.json();
          if (!v.url) return fail(v.message || 'Image upload failed');
          const alt = file.name.replace(/\.[^.]*$/, '').replace(/[[\]]/g, '') || 'image';
          insert(editor, `![${alt}](${v.url})`);
        })
        .catch(error => fail(String(error)));
    }
  };
  const inEditor = e => e.target?.classList?.contains('md-editor') && e.target.dataset.task;
  document.addEventListener('paste', e => inEditor(e) && take(e, e.clipboardData?.files, e.target.dataset.task, e.target.id, e.target.dataset.base));
  document.addEventListener('dragover', e => inEditor(e) && e.preventDefault());
  document.addEventListener('drop', e => inEditor(e) && take(e, e.dataTransfer?.files, e.target.dataset.task, e.target.id, e.target.dataset.base));
  document.addEventListener('change', e => {
    if (!e.target?.classList?.contains('md-image')) return;
    take(e, e.target.files, e.target.dataset.task, e.target.dataset.for, e.target.dataset.base);
    e.target.value = '';
  });
}
