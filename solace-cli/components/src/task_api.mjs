// Browser transport for <solace-task>: state, decoding and rendering belong to Lustre.
// `base` is the task's project prefix on this server ("" for the root's; `/p/<dir>`
// for a workspace project). Root routes such as `/models` are asked with "".
// A component's shadow root before it has rendered: the helpers do nothing.
export function no_root() { return null; }
export function request(base, path, method, body, receive) {
  fetch((path.startsWith('/p/') ? '' : base) + path, { method, headers: { 'Content-Type': 'application/json' },
    ...(method === 'GET' ? {} : { body }) })
    .then(response => response.text())
    .then(
      text => receive(text),
      () => receive(JSON.stringify({
        connectionLost: true,
        message: 'Connection lost; the request may not have completed.',
      })),
    )
    .catch(error => console.error(error));
}
const streams = new WeakMap();
export function events(base, receive, ready = () => {}) {
  const state = { base, receive, ready, last: '', attempt: 0, timer: null, current: null, stopped: false };
  const open = () => {
    if (state.stopped) return null;
    // Native EventSource reconnects with Last-Event-ID. One the browser has
    // closed for good is opened afresh, and the query carries the position a
    // script can't put in that header.
    const source = new EventSource(state.base + '/events' + (state.last ? '?last=' + encodeURIComponent(state.last) : ''));
    state.current = source;
    source.onmessage = event => {
      if (event.lastEventId) state.last = event.lastEventId;
      state.receive(event.data);
    };
    source.addEventListener('ready', () => { state.attempt = 0; state.ready(); });
    source.onerror = () => {
      // While the browser is retrying, it keeps its own position.
      if (state.stopped || source.readyState !== 2) return;
      source.close();
      const wait = Math.min(1000 * 2 ** state.attempt, 30000);
      state.attempt++;
      state.timer = setTimeout(() => { state.timer = null; open(); }, wait);
    };
    return source;
  };
  const source = open();
  if (source) streams.set(source, state);
  return source;
}
export function close_events(source) {
  const state = source && streams.get(source);
  if (!state) return source?.close();
  state.stopped = true;
  if (state.timer) clearTimeout(state.timer);
  state.current?.close();
}
export function after(ms, callback) {
  setTimeout(callback, ms);
}
export function clock(seconds) {
  return seconds ? new Date(seconds * 1000).toLocaleString() : '';
}
export function confirm(question) {
  return globalThis.confirm(question);
}

// Browser storage may be missing or blocked; the task works without it.
export function load(key) {
  try { return globalThis.localStorage?.getItem(key) ?? ''; } catch { return ''; }
}
export function store(key, value) {
  try { value ? localStorage.setItem(key, value) : localStorage.removeItem(key); } catch {}
}

// A draft belongs to one editor and the record or context it edits. The editor's id and that owner
// keep two tasks, a new and an edited record, or two planning contexts apart.
export function draft_key(editor, owner) {
  return `solace-draft:${editor}:${owner}`;
}
export function load_draft(editor, owner) {
  return load(draft_key(editor, owner));
}
export function save_draft(editor, owner, text) {
  store(draft_key(editor, owner), text);
}
export function clear_draft(editor, owner) {
  store(draft_key(editor, owner), '');
}

// Agent text is untrusted: everything is escaped before the few markdown forms
// (code blocks, headings, blockquotes, tables, nested lists, `code`, **bold**, images uploaded to
// this task board, and http(s) links) are turned into HTML. Image URLs are stored
// relative to their project; `base` is the project's prefix on this server.
// Quotes nest at most `maxQuotes` deep; a deeper `>` stays text, so untrusted
// text can't exhaust the stack. `depth` is the quotes already open.
const maxQuotes = 100;
// A task, feature or milestone reference (D-98, D-122): `[label](<kind>:<id>)`, or a bare `<kind>:<id>`
// shown as itself until the shell puts the record's title in its place, links to `/<kind>/<id>`, which the
// shell opens on the board or the Strategy page. The id is a record's 16 hex digits. A link already made is
// skipped whole, so a reference inside its label isn't nested in it; an image form (`![x](task:<id>)`) stays text.
const taskRef = /(<a [^>]*>[\s\S]*?<\/a>)|(!?)\[([^\]]+)\]\((task|feature|milestone):([0-9a-f]{16})\)|(^|[^\w/])(task|feature|milestone):([0-9a-f]{16})(?!\w)/g;
const taskLink = (kind, id, label) => `<a href="/${kind}/${id}" data-${kind}="${id}">${label}</a>`;
export function markdown(text, base = '', depth = 0) {
  const escape = s => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  // A code span closes only at a later run of the same number of backticks.
  const closing = (line, from, length) => {
    for (let j = from; j < line.length;) {
      if (line[j] !== '`') { j++; continue; }
      let run = 1;
      while (line[j + run] === '`') run++;
      if (run === length) return j;
      j += run;
    }
    return -1;
  };
  // Code spans are lifted out first so bold and links leave them literal.
  const inline = s => {
    const codes = [];
    const lift = text => {
      let out = '';
      for (let j = 0; j < text.length; j++) {
        if (text[j] !== '`') { out += text[j]; continue; }
        let run = 1;
        while (text[j + run] === '`') run++;
        const end = closing(text, j + run, run);
        if (end < 0) { out += text.slice(j, j + run); j += run - 1; continue; }
        out += `\u0000${codes.push(text.slice(j + run, end)) - 1}\u0000`;
        j = end + run - 1;
      }
      return out;
    };
    return lift(escape(s.replace(/\u0000/g, '')))
      .replace(/\*\*([\s\S]+?)\*\*/g, '<strong>$1</strong>')
      .replace(/!\[([^\]]*)\]\((\/tasks\/[\w-]+\/files\/[0-9a-f]{16}\.(?:png|jpg|gif|webp))\)/g, `<img alt="$1" src="${escape(base)}$2">`)
      .replace(/(^|[^!])\[([^\]]+)\]\((https?:\/\/[^\s)\u0000]+)\)/g, '$1<a href="$3" rel="noopener noreferrer">$2</a>')
      .replace(taskRef, (all, link, bang, label, kind, id, before, bareKind, bare) =>
        link ? link : label ? (bang ? all : taskLink(kind, id, label)) : before + taskLink(bareKind, bare, `${bareKind}:${bare}`))
      .replace(/\u0000(\d+)\u0000/g, (_, n) => `<code>${codes[n]}</code>`);
  };
  const item = /^(\s*)([-*]|\d+[.)])\s+(.*)/, heading = /^(#{1,4})\s+(.*)/, quote = /^ {0,3}>[ \t]?/;
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
        // A numbered list begins at its first number; the browser counts on from there.
        const first = ordered ? Number.parseInt(mark, 10) : 1;
        html += ordered && first !== 1 && Number.isSafeInteger(first) ? `<ol start="${first}">` : `<${tag({ ordered })}>`;
      }
      html += `<li>${inline(content)}`;
    }
    while (stack.length) html += `</li></${tag(stack.pop())}>`;
    return html;
  };
  const delimiter = /^:?-+:?$/;
  // Cells are split before `inline`, so a pipe in a code span or written `\|`
  // stays in its cell.
  const escapedEnd = line => {
    let slashes = 0;
    for (let j = line.length - 2; j >= 0 && line[j] === '\\'; j--) slashes++;
    return slashes % 2 === 1;
  };
  const row = line => {
    let rest = line.trim();
    if (rest.startsWith('|')) rest = rest.slice(1);
    if (rest.endsWith('|') && !escapedEnd(rest)) rest = rest.slice(0, -1);
    const cells = [];
    let cell = '', code = 0;
    for (let j = 0; j < rest.length; j++) {
      if (rest[j] === '\\' && rest[j + 1] === '|') { cell += '|'; j++; continue; }
      if (rest[j] === '`') {
        let run = 1;
        while (rest[j + run] === '`') run++;
        if (code === run) code = 0;
        else if (!code && closing(rest, j + run, run) >= 0) code = run;
        cell += rest.slice(j, j + run);
        j += run - 1;
        continue;
      }
      if (rest[j] === '|' && !code) { cells.push(cell.trim()); cell = ''; continue; }
      cell += rest[j];
    }
    cells.push(cell.trim());
    return cells;
  };
  const alignment = cell =>
    cell.startsWith(':') && cell.endsWith(':') ? 'center'
      : cell.startsWith(':') ? 'left' : cell.endsWith(':') ? 'right' : '';
  const aligned = value => value ? ` style="text-align:${value}"` : '';
  const cellsRow = (tag, cells, aligns) =>
    `<tr>${cells.map((cell, j) => `<${tag}${aligned(aligns[j])}>${inline(cell)}</${tag}>`).join('')}</tr>`;
  const table = (head, aligns, rows) =>
    `<table><thead>${cellsRow('th', head, aligns)}</thead><tbody>${
      rows.map(cells => cellsRow('td', head.map((_, j) => cells[j] ?? ''), aligns)).join('')
    }</tbody></table>`;
  const out = [];
  const parts = text.split(/^```[^\n]*\n?/m);
  parts.forEach((part, i) => {
    if (i % 2) { out.push(`<pre><code>${escape(part.replace(/\n$/, ''))}</code></pre>`); return; }
    const lines = part.split('\n').map(l => l.replace(/\r$/, ''));
    const quoted = l => depth < maxQuotes && quote.test(l);
    const plain = l => !heading.test(l) && !item.test(l) && !quoted(l);
    // The next non-blank line at or after `from`, or lines.length.
    const skip = from => { while (from < lines.length && !lines[from].trim()) from++; return from; };
    const tableAt = from => {
      if (from + 1 >= lines.length || !lines[from].trim() || !plain(lines[from])) return false;
      const head = row(lines[from]), separator = row(lines[from + 1]);
      return (lines[from].includes('|') || lines[from + 1].includes('|'))
        && head.length === separator.length
        && separator.every(cell => delimiter.test(cell));
    };
    for (let i = skip(0); i < lines.length; i = skip(i)) {
      const head = lines[i].match(heading);
      if (head) { out.push(`<h4>${inline(head[2])}</h4>`); i++; continue; }
      const run = [];
      if (tableAt(i)) {
        const head = row(lines[i]), aligns = row(lines[i + 1]).map(alignment);
        i += 2;
        const rows = [];
        while (i < lines.length && lines[i].trim() && plain(lines[i])) rows.push(row(lines[i++]));
        out.push(table(head, aligns, rows));
      } else if (quoted(lines[i])) {
        while (i < lines.length && quoted(lines[i])) run.push(lines[i++].replace(quote, ''));
        out.push(`<blockquote>${markdown(run.join('\n'), base, depth + 1)}</blockquote>`);
      } else if (item.test(lines[i])) {
        // Blank lines inside a list don't end it: the next item continues it.
        while (i < lines.length && item.test(lines[skip(i)] ?? '')) { i = skip(i); run.push(lines[i++]); }
        out.push(list(run));
      } else {
        while (i < lines.length && lines[i].trim() && plain(lines[i]) && !tableAt(i)) run.push(lines[i++]);
        out.push(`<p>${run.map(inline).join('<br>')}</p>`);
      }
    }
  });
  return out.join('');
}

// Wraps the markdown editor `id`'s selection (or appends, without one); `root`
// is the shadow root that holds the editor.
export function wrap(root, id, text, before, after) {
  const el = root?.getElementById?.(id);
  const start = el?.selectionStart ?? text.length, end = el?.selectionEnd ?? text.length;
  return text.slice(0, start) + before + text.slice(start, end) + after + text.slice(end);
}

// Turns the markdown editor `id`'s selected lines into a block of `kind`
// ("heading", "list", "numbered", "quote" or "code"); `root` is the shadow root
// that holds the editor. The selection widens to whole lines; without one, the cursor's
// line is used. Each non-blank line gets the marker (numbered 1, 2, 3...), a
// code block is fenced on lines of its own, and blank lines set the block off
// from the text around it.
export function block(root, id, text, kind) {
  const el = root?.getElementById?.(id);
  let start = el?.selectionStart ?? text.length, end = el?.selectionEnd ?? text.length;
  // A selection ending at the start of a line leaves that line out.
  if (end > start && text[end - 1] === '\n') end--;
  start = text.lastIndexOf('\n', start - 1) + 1;
  end = text.indexOf('\n', end) < 0 ? text.length : text.indexOf('\n', end);
  const lines = text.slice(start, end).split('\n');
  const empty = lines.every(l => !l.trim());
  let n = 0;
  const marked = kind === 'code' ? ['```', ...(empty ? [''] : lines), '```']
    : empty ? [{ heading: '#### ', list: '- ', numbered: '1. ', quote: '> ' }[kind]]
    : lines.map(l => {
      if (kind === 'quote') return l.trim() ? '> ' + l : '>';
      if (!l.trim()) return '';
      return { heading: '#### ', list: '- ', numbered: `${++n}. ` }[kind] + l.trim();
    });
  let before = text.slice(0, start), after = text.slice(end);
  if (before.trim() && !before.endsWith('\n\n')) before += '\n';
  if (after.trim() && !after.startsWith('\n\n')) after = '\n' + after;
  return before + marked.join('\n') + after;
}

// Defines <solace-md-editor> (md_editor.mjs, CodeMirror) once, in a browser. Elsewhere, as in the tests, it
// does nothing, and CodeMirror is never loaded. Concurrent asks share one load.
let defining;
export function define_editor(load = () => import('./md_editor.mjs')) {
  if (!globalThis.customElements || customElements.get('solace-md-editor') || defining) return;
  defining = load()
    .then(editor => editor.define())
    .catch(error => console.error('Could not load the markdown editor.', error))
    .finally(() => { defining = undefined; });
}

// How the markdown editor takes a text set from outside it (a tool, an image, a box cleared after a send):
// only the part that differs, `from` to `to` of `old`, is replaced by `insert`, so the cursor and undo keep the
// rest. The selection before, `start` to `end`, becomes `anchor` to `head`: the selected text again where the
// new part holds it; between a pair of marks put round no selection (`****` or two backticks); else the end of
// the new part.
export function replacement(old, next, start, end) {
  const most = Math.min(old.length, next.length);
  let from = 0;
  while (from < most && old[from] === next[from]) from++;
  let tail = 0;
  while (tail < most - from && old[old.length - 1 - tail] === next[next.length - 1 - tail]) tail++;
  const to = old.length - tail, insert = next.slice(from, next.length - tail);
  const selected = old.slice(start, end);
  const at = selected ? insert.indexOf(selected) : -1;
  if (at >= 0) return { from, to, insert, anchor: from + at, head: from + at + selected.length };
  const cursor = !selected && /^(\*\*\*\*|``)$/.test(insert) ? from + insert.length / 2 : from + insert.length;
  return { from, to, insert, anchor: cursor, head: cursor };
}

// Images pasted, dropped or picked into a markdown editor are uploaded to its
// task; `insert` gets the editor's id and each image's markdown. Only editors in
// `target` itself count: an event from a shadow root nested inside it belongs to
// that root's own listener, so an image is uploaded once. An editor,
// <solace-md-editor>, keeps its text in a shadow root of its own, so it is
// found on the event's path.
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
        .then(
          response => response.json().then(
            v => {
              if (!v.url) return fail(v.message || 'Image upload failed');
              const alt = file.name.replace(/\.[^.]*$/, '').replace(/[[\]]/g, '') || 'image';
              try {
                insert(editor, `![${alt}](${v.url})`);
              } catch {
                fail('Image uploaded, but could not be inserted.');
              }
            },
            () => fail('Image upload failed'),
          ),
          () => fail('Connection lost; the image was not uploaded.'),
        );
    }
  };
  const from = e => e.composedPath?.()[0] || e.target;
  const own = e => { const root = from(e)?.getRootNode?.(); return !root || root === target; };
  const editorOf = e => {
    const el = (e.composedPath?.() || [e.target]).find(n => n?.classList?.contains('md-editor'));
    const root = el?.getRootNode?.();
    return el?.dataset?.task && (!root || root === target) ? el : null;
  };
  target.addEventListener('paste', e => { const el = editorOf(e); if (el) take(e, e.clipboardData?.files, el.dataset.task, el.id, el.dataset.base); });
  target.addEventListener('dragover', e => editorOf(e) && e.preventDefault());
  target.addEventListener('drop', e => { const el = editorOf(e); if (el) take(e, e.dataTransfer?.files, el.dataset.task, el.id, el.dataset.base); });
  target.addEventListener('change', e => {
    const el = from(e);
    if (!own(e) || !el?.classList?.contains('md-image')) return;
    take(e, el.files, el.dataset.task, el.dataset.for, el.dataset.base);
    el.value = '';
  });
}

// Copy ID: the clipboard API where the page may use it, else a hidden text box's copy. `done` hears
// whether either one copied, so the button never says Copied for a copy that failed.
export function copy_text(text, done) {
  const fallback = () => {
    if (!globalThis.document?.body) return false;
    const box = Object.assign(document.createElement('textarea'), { value: text });
    box.style.cssText = 'position:fixed;opacity:0';
    document.body.append(box);
    box.select();
    let copied = false;
    try { copied = document.execCommand('copy'); } catch {}
    box.remove();
    return copied;
  };
  const clipboard = globalThis.navigator?.clipboard;
  if (clipboard?.writeText) clipboard.writeText(text).then(() => done(true), () => done(fallback()));
  else done(fallback());
}

// Brings the task's details into view and puts focus on its selected tab, for
// a notice's button; `root` is this element's shadow root.
export function reveal_task(root) {
  const detail = root?.querySelector?.('.detail');
  if (!detail) return;
  const calm = globalThis.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches;
  detail.scrollIntoView?.({ block: 'start', behavior: calm ? 'auto' : 'smooth' });
  detail.querySelector('[role=tab][aria-selected=true]')?.focus?.({ preventScroll: true });
}

// Puts focus on the markdown editor `id`, for Add note; `root` is this
// element's shadow root.
export function focus_editor(root, id) {
  root?.getElementById?.(id)?.focus?.();
}
