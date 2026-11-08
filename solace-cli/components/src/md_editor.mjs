// <solace-md-editor>: the markdown editor of `solace_task.editor`. It is CodeMirror 6, in a shadow root of its
// own so the page's view never touches its DOM. The markdown is styled as it is written, in the forms `markdown`
// in task_api.mjs draws: headings, **bold**, `code`, code blocks, quotes, lists, links and the board's images.
// Their marks show on the lines the cursor is on and hide elsewhere; the text saved is exactly what was typed.
//
// Its host sees a text box: the `value` property; an `md-input` event, with the text in `detail`, as it is typed;
// and `selectionStart` / `selectionEnd` for the toolbar's tools. Attributes: `label` (its accessible name),
// `placeholder`, and `data-task` / `data-base` for images (see `images` in task_api.mjs).
import { EditorState, Compartment } from '@codemirror/state';
import { EditorView, ViewPlugin, Decoration, WidgetType, keymap, placeholder } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { syntaxTree } from '@codemirror/language';
import { markdown } from '@codemirror/lang-markdown';
import { watchActionPad } from './editor_actions.mjs';
import { replacement } from './task_api.mjs';

const mono = "'IBM Plex Mono',ui-monospace,monospace";
// What task_api.mjs's `markdown` draws as a link, and as an image (uploaded to the board).
const linkTarget = /^(?:https?:\/\/\S+|(?:task|feature|milestone|design):[0-9a-f]{16})$/;
const imageTarget = /^\/tasks\/[\w-]+\/files\/[0-9a-f]{16}\.(?:png|jpg|gif|webp)$/;

class ImageWidget extends WidgetType {
  constructor(src, alt) { super(); this.src = src; this.alt = alt; }
  eq(other) { return other.src === this.src && other.alt === this.alt; }
  toDOM() {
    const img = document.createElement('img');
    img.src = this.src; img.alt = this.alt; img.className = 'cm-md-image';
    return img;
  }
}

const hidden = Decoration.replace({});
const marks = {
  strong: Decoration.mark({ class: 'cm-md-strong' }),
  code: Decoration.mark({ class: 'cm-md-code' }),
  link: Decoration.mark({ class: 'cm-md-link' }),
  listMark: Decoration.mark({ class: 'cm-md-list-mark' }),
};
const lines = {
  heading: Decoration.line({ class: 'cm-md-heading' }),
  quote: Decoration.line({ class: 'cm-md-quote' }),
  fence: Decoration.line({ class: 'cm-md-fence' }),
};

// The decorations of the visible text; `base` is the project's prefix for image URLs.
function decorate(view, base) {
  const { state } = view, doc = state.doc, out = [];
  // The raw markdown shows on the lines the selection touches, while the editor has focus.
  const raw = (from, to) => {
    if (!view.hasFocus) return false;
    const first = doc.lineAt(from).from, last = doc.lineAt(to).to;
    return state.selection.ranges.some(r => r.from <= last && r.to >= first);
  };
  const eachLine = (from, to, deco) => {
    for (let pos = from; pos <= to;) {
      const line = doc.lineAt(pos);
      out.push(deco.range(line.from));
      pos = line.to + 1;
    }
  };
  // Only within a line: CodeMirror refuses a plugin's decoration that hides a line break.
  const oneLine = (from, to) => to > from && doc.lineAt(from).number === doc.lineAt(to).number;
  const hide = (from, to) => { if (oneLine(from, to)) out.push(hidden.range(from, to)); };
  // A mark and the spaces after it, as `#### ` or `> `.
  const hideMark = (from, to) => {
    const end = doc.lineAt(from).to;
    while (to < end && /[ \t]/.test(doc.sliceString(to, to + 1))) to++;
    hide(from, to);
  };
  for (const { from, to } of view.visibleRanges) {
    syntaxTree(state).iterate({ from, to, enter: ref => {
      const node = ref.node, name = ref.name;
      const show = raw(ref.from, ref.to);
      if (/^ATXHeading[1-4]$/.test(name)) {
        eachLine(ref.from, ref.to, lines.heading);
        const mark = node.getChild('HeaderMark');
        if (!show && mark && mark.from === ref.from) hideMark(mark.from, mark.to);
      } else if (name === 'StrongEmphasis' && doc.sliceString(ref.from, ref.from + 2) === '**') {
        out.push(marks.strong.range(ref.from, ref.to));
        if (!show) for (const m of node.getChildren('EmphasisMark')) hide(m.from, m.to);
      } else if (name === 'InlineCode') {
        out.push(marks.code.range(ref.from, ref.to));
        if (!show) for (const m of node.getChildren('CodeMark')) hide(m.from, m.to);
      } else if (name === 'FencedCode') {
        eachLine(ref.from, ref.to, lines.fence);
        return false;
      } else if (name === 'Blockquote') {
        eachLine(ref.from, ref.to, lines.quote);
      } else if (name === 'QuoteMark') {
        if (!show) hideMark(ref.from, ref.to);
      } else if (name === 'ListMark') {
        out.push(marks.listMark.range(ref.from, ref.to));
      } else if (name === 'Link') {
        const url = node.getChild('URL'), bounds = node.getChildren('LinkMark');
        if (url && bounds.length >= 2 && linkTarget.test(doc.sliceString(url.from, url.to))) {
          const start = bounds[0].to, end = bounds[1].from;
          if (end > start) out.push(marks.link.range(start, end));
          if (!show) { hide(ref.from, start); hide(end, ref.to); }
        }
      } else if (name === 'Image') {
        const url = node.getChild('URL'), bounds = node.getChildren('LinkMark');
        const src = url ? doc.sliceString(url.from, url.to) : '';
        if (!show && bounds.length >= 2 && imageTarget.test(src) && oneLine(ref.from, ref.to)) {
          const alt = doc.sliceString(bounds[0].to, bounds[1].from);
          out.push(Decoration.replace({ widget: new ImageWidget(base + src, alt) }).range(ref.from, ref.to));
        }
        return false;
      }
    } });
  }
  return Decoration.set(out, true);
}

const styling = base => ViewPlugin.fromClass(class {
  constructor(view) { this.decorations = decorate(view, base()); }
  update(u) {
    if (u.docChanged || u.viewportChanged || u.selectionSet || u.focusChanged
      || syntaxTree(u.startState) !== syntaxTree(u.state)) this.decorations = decorate(u.view, base());
  }
}, { decorations: plugin => plugin.decorations });

// Colors come from the page's custom properties, which reach into this shadow root. A host sizes the box with
// `--md-min-height` and `--md-max-height`; it grows with its text up to the second, then scrolls.
const theme = EditorView.theme({
  '&': { color: 'var(--ink)', backgroundColor: 'var(--paper)', maxHeight: 'var(--md-max-height, none)' },
  '&.cm-focused': { outline: '3px solid var(--focus)', outlineOffset: '-3px' },
  '.cm-scroller': { fontFamily: 'inherit', lineHeight: '1.5', overflow: 'auto' },
  '.cm-content': { minHeight: 'var(--md-min-height, 110px)', padding: '6px 0', caretColor: 'var(--ink)' },
  '.cm-line': { padding: '0 calc(8px + var(--md-line-pad, 0px)) 0 8px' },
  '.cm-line:last-child': { paddingRight: 'calc(8px + max(var(--md-line-pad, 0px), var(--md-action-pad, 0px) + var(--md-action-gap, 0px)))' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--ink)' },
  '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection':
    { backgroundColor: 'var(--sel, #bfd5ce)' },
  '.cm-placeholder': { color: 'var(--soft)' },
  '.cm-md-heading': { fontWeight: '700', fontSize: '1.1em' },
  '.cm-md-strong': { fontWeight: '700' },
  '.cm-md-code': { fontFamily: mono, fontSize: '13px', backgroundColor: 'var(--code)', padding: '1px 4px' },
  '.cm-md-fence': { fontFamily: mono, fontSize: '13px', backgroundColor: 'var(--code)' },
  '.cm-md-quote': { borderLeft: '3px solid var(--line)', color: 'var(--soft)', paddingLeft: '10px' },
  '.cm-md-list-mark': { color: 'var(--soft)' },
  '.cm-md-link': { color: 'var(--focus)', textDecoration: 'underline' },
  '.cm-md-image': { display: 'inline-block', maxWidth: '100%', maxHeight: '240px', height: 'auto', border: '1px solid var(--line)', verticalAlign: 'top' },
});

// Images pasted or dropped on an editor of a task are uploaded by `images` in task_api.mjs; CodeMirror would
// otherwise read a dropped file as text.
const images = (el, data) => !!el.dataset.task && [...(data?.files || [])].some(f => f.type.startsWith('image/'));

class MdEditor extends HTMLElement {
  static observedAttributes = ['label', 'placeholder'];
  #view = null;
  #text = '';
  // Set while a text from the host is applied: it isn't sent back as typed.
  #quiet = false;
  #label = new Compartment();
  #placeholder = new Compartment();
  #stopActions = null;

  connectedCallback() {
    // A value set before this element was defined is an own property, hiding the accessor.
    if (Object.hasOwn(this, 'value')) {
      const text = this.value;
      delete this.value;
      this.#text = String(text ?? '');
    }
    if (this.#view) {
      this.#watchActions();
      return;
    }
    const root = this.shadowRoot ?? this.attachShadow({ mode: 'open' });
    this.#view = new EditorView({
      root,
      parent: root,
      state: EditorState.create({
        doc: this.#text,
        extensions: [
          history(),
          // Ctrl/Cmd+Enter belongs to the page (Plan sends with it); Ctrl+F too (find in page).
          keymap.of([...defaultKeymap.filter(b => b.key !== 'Mod-Enter'), ...historyKeymap]),
          markdown(),
          EditorView.lineWrapping,
          this.#label.of(this.#labelled()),
          this.#placeholder.of(placeholder(this.getAttribute('placeholder') || '')),
          styling(() => this.dataset.base || ''),
          theme,
          EditorView.domEventHandlers({
            paste: e => images(this, e.clipboardData),
            drop: e => images(this, e.dataTransfer),
          }),
          EditorView.updateListener.of(u => {
            if (!u.docChanged) return;
            this.#text = u.state.doc.toString();
            if (!this.#quiet) this.dispatchEvent(new CustomEvent('md-input', { detail: this.#text }));
          }),
        ],
      }),
    });
    this.#watchActions();
  }

  // A move within the page reconnects at once and keeps its state; a removal frees CodeMirror's listeners.
  disconnectedCallback() {
    queueMicrotask(() => {
      if (this.isConnected) return;
      this.#stopActions?.();
      this.#stopActions = null;
      if (!this.#view) return;
      this.#view.destroy();
      this.#view = null;
    });
  }

  #watchActions() {
    this.#stopActions?.();
    this.#stopActions = watchActionPad(this.closest('.composer'));
  }

  attributeChangedCallback(name) {
    const view = this.#view;
    if (!view) return;
    view.dispatch({ effects: name === 'label'
      ? this.#label.reconfigure(this.#labelled())
      : this.#placeholder.reconfigure(placeholder(this.getAttribute('placeholder') || '')) });
  }

  #labelled() {
    return EditorView.contentAttributes.of({ 'aria-label': this.getAttribute('label') || '' });
  }

  get value() { return this.#text; }

  // Only the part that differs is replaced, so the cursor and undo keep the rest (`replacement`).
  set value(text) {
    text = String(text ?? '');
    const view = this.#view;
    if (!view) { this.#text = text; return; }
    const old = view.state.doc.toString();
    if (text === old) return;
    const { main } = view.state.selection;
    const { from, to, insert, anchor, head } = replacement(old, text, main.from, main.to);
    this.#quiet = true;
    try { view.dispatch({ changes: { from, to, insert }, selection: { anchor, head } }); }
    finally { this.#quiet = false; }
  }

  get selectionStart() { return this.#view?.state.selection.main.from ?? this.#text.length; }
  get selectionEnd() { return this.#view?.state.selection.main.to ?? this.#text.length; }
  focus() { this.#view ? this.#view.focus() : super.focus(); }
}

export function define() {
  if (!customElements.get('solace-md-editor')) customElements.define('solace-md-editor', MdEditor);
}
