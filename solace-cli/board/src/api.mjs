// Browser transport only: state, decoding and rendering belong to Lustre.
// Each component instance passes its own `base` attribute; without one, a
// workspace project's contract view at `/p/<dir>/view` asks its own project
// (D-57). The board passes each lane's prefix itself.
const fallbackBase = () => globalThis.location?.pathname.match(/^(.*)\/view$/)?.[1] ?? '';
const prefix = base => base || fallbackBase();
// A component's shadow root before it has rendered: the helpers do nothing.
export function no_root() { return null; }
export function request(base, path, method, body, receive) {
  fetch((path === '/migration' || path.startsWith('/migration/') || path.startsWith('/p/') ? '' : prefix(base)) + path, { method, headers: { 'Content-Type': 'application/json' },
    ...(method === 'GET' ? {} : { body }) })
    .then(async response => receive(await response.text()))
    .catch(error => receive(JSON.stringify({ message: String(error) })));
}
export function events(base, receive, ready = () => {}) {
  const source = new EventSource(prefix(base) + '/events');
  source.onmessage = event => receive(event.data);
  source.addEventListener('ready', () => ready());
  // Native EventSource reconnects with Last-Event-ID.
  source.onerror = () => {};
  return source;
}
export function close_events(source) { source?.close(); }
export function after(ms, callback) {
  setTimeout(callback, ms);
}
export function clock(seconds) {
  return seconds ? new Date(seconds * 1000).toLocaleString() : '';
}
// Agent text is untrusted: everything is escaped before the few markdown forms
// (code blocks, headings, nested lists, `code`, **bold**, images uploaded to this task
// board, and http(s) links) are turned into HTML. Image URLs are stored
// relative to their project; `base` is the project's prefix on this server.
export function markdown(text, base = '') {
  const escape = s => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  // Code spans are lifted out first so bold and links leave them literal.
  const inline = s => {
    const codes = [];
    return escape(s.replace(/\u0000/g, '')).replace(/`([^`]+)`/g, (_, c) => `\u0000${codes.push(c) - 1}\u0000`)
      .replace(/\*\*([\s\S]+?)\*\*/g, '<strong>$1</strong>')
      .replace(/!\[([^\]]*)\]\((\/tasks\/[\w-]+\/files\/[0-9a-f]{16}\.(?:png|jpg|gif|webp))\)/g, `<img alt="$1" src="${escape(base)}$2">`)
      .replace(/(^|[^!])\[([^\]]+)\]\((https?:\/\/[^\s)\u0000]+)\)/g, '$1<a href="$3" rel="noopener noreferrer">$2</a>')
      .replace(/\u0000(\d+)\u0000/g, (_, n) => `<code>${codes[n]}</code>`);
  };
  const item = /^(\s*)([-*]|\d+[.)])\s+(.*)/, heading = /^(#{1,4})\s+(.*)/;
  // One run of list lines; deeper indentation nests inside the current item.
  const list = run => {
    let html = '';
    const stack = [];
    const tag = l => l.ordered ? 'ol' : 'ul';
    for (const line of run) {
      const [, space, mark, content] = line.match(item);
      const indent = space.replace(/\t/g, '    ').length, ordered = /\d/.test(mark);
      while (stack.length && indent < stack.at(-1).indent) html += `</li></${tag(stack.pop())}>`;
      const top = stack.at(-1);
      if (top && indent === top.indent && top.ordered === ordered) html += '</li>';
      else {
        if (top && indent === top.indent) html += `</li></${tag(stack.pop())}>`;
        stack.push({ indent, ordered });
        html += `<${tag({ ordered })}>`;
      }
      html += `<li>${inline(content)}`;
    }
    while (stack.length) html += `</li></${tag(stack.pop())}>`;
    return html;
  };
  const out = [];
  const parts = text.split(/^```[^\n]*\n?/m);
  parts.forEach((part, i) => {
    if (i % 2) { out.push(`<pre><code>${escape(part.replace(/\n$/, ''))}</code></pre>`); return; }
    for (const block of part.split(/\n\s*\n/)) {
      const lines = block.split('\n').map(l => l.replace(/\r$/, '')).filter(l => l.trim());
      for (let i = 0; i < lines.length;) {
        const head = lines[i].match(heading);
        if (head) { out.push(`<h4>${inline(head[2])}</h4>`); i++; continue; }
        const run = [];
        const plain = l => !heading.test(l) && !item.test(l);
        if (item.test(lines[i])) {
          while (i < lines.length && item.test(lines[i])) run.push(lines[i++]);
          out.push(list(run));
        } else {
          while (i < lines.length && plain(lines[i])) run.push(lines[i++]);
          out.push(`<p>${run.map(inline).join('<br>')}</p>`);
        }
      }
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

// Flips the page between light and dark; index.html applies the stored choice.
export function toggle_theme() {
  const root = globalThis.document.documentElement;
  const next = root.dataset.theme === 'dark' ? 'light' : 'dark';
  root.dataset.theme = next;
  store('solace-theme', next);
}

// Wraps the markdown editor `id`'s selection (or appends, without one); `root`
// is the board's shadow root.
export function wrap(root, id, text, before, after) {
  const el = root?.getElementById?.(id);
  const start = el?.selectionStart ?? text.length, end = el?.selectionEnd ?? text.length;
  return text.slice(0, start) + before + text.slice(start, end) + after + text.slice(end);
}

// Images pasted, dropped or picked into a markdown editor are uploaded to its
// task; `insert` gets the editor's id and each image's markdown.
const imageRoots = new WeakSet();
export function images(target, insert, fail) {
  if (!target?.addEventListener || imageRoots.has(target)) return;
  imageRoots.add(target);
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
  const from = e => e.composedPath?.()[0] || e.target;
  const inEditor = e => from(e)?.classList?.contains('md-editor') && from(e).dataset.task;
  target.addEventListener('paste', e => { const el = from(e); if (inEditor(e)) take(e, e.clipboardData?.files, el.dataset.task, el.id, el.dataset.base); });
  target.addEventListener('dragover', e => inEditor(e) && e.preventDefault());
  target.addEventListener('drop', e => { const el = from(e); if (inEditor(e)) take(e, e.dataTransfer?.files, el.dataset.task, el.id, el.dataset.base); });
  target.addEventListener('change', e => {
    const el = from(e);
    if (!el?.classList?.contains('md-image')) return;
    take(e, el.files, el.dataset.task, el.dataset.for, el.dataset.base);
    el.value = '';
  });
}

const dragRoots = new WeakSet();
export function prevent_drag(target) {
  if (!target?.addEventListener || dragRoots.has(target)) return;
  dragRoots.add(target);
  target.addEventListener('dragstart', e => {
    const el = e.composedPath?.()[0] || e.target;
    if (!el?.closest?.('input,textarea')) e.preventDefault();
  });
}
