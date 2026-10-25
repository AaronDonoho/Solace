// The page's document handlers. The decisions live in frontend.gleam (blocks_drag, keeps_new_tab, find_opens, next_match, count_text, to_markdown), where they are tested.
import { blocks_drag, keeps_new_tab, find_opens, next_match, count_text, to_markdown, Text as HtmlText, Tag } from '../build/dev/javascript/frontend/frontend.mjs';
import { toList } from '../build/dev/javascript/frontend/gleam.mjs';
import { watch_shadow_hosts } from './shadow_watch.mjs';

export function guard(root) {
  // A markdown editor, <solace-md-editor>, keeps its text in a shadow root of its own: it is found on the path.
  root.addEventListener('dragstart', e => {
    const el = e.composedPath?.()[0] || e.target;
    if (blocks_drag(!!el?.closest?.('input,textarea') || inEditor(e))) e.preventDefault();
  });

  // modem's click handler routes every link in-page; let modified clicks keep the browser's new-tab behavior.
  root.addEventListener('click', e => {
    if (keeps_new_tab(!!e.ctrlKey, !!e.metaKey, !!e.shiftKey, !!e.altKey, !!e.target.closest?.('.menu a'))) e.stopImmediatePropagation();
  }, true);

  find(root);
  copy(root);
}

const inEditor = e => !!e.composedPath?.().some(n => n?.classList?.contains?.('md-editor'));

// Copy keeps markdown: a selection that touches rendered markdown (.md, from the board's renderer) goes to the
// clipboard as its markdown source, with the HTML beside it for rich paste. Any other selection copies as usual.
const INLINE = new Set(['STRONG', 'B', 'CODE', 'A']);

const shadowRoots = (node, out = []) => {
  for (const el of node.querySelectorAll('*')) if (el.shadowRoot) { out.push(el.shadowRoot); shadowRoots(el.shadowRoot, out); }
  return out;
};

// The selected range, inside whichever shadow root holds it: Chromium has a selection per shadow root, WebKit
// gives composed ranges from the document's (taking the roots as a dictionary or, before that, as arguments).
function selected() {
  const roots = shadowRoots(document);
  for (const root of roots) {
    const sel = root.getSelection?.();
    if (sel?.rangeCount && !sel.isCollapsed) return sel.getRangeAt(0);
  }
  const sel = document.getSelection();
  if (!sel?.rangeCount) return null;
  let found = sel.getRangeAt(0);
  if (sel.getComposedRanges) {
    try { [found] = sel.getComposedRanges({ shadowRoots: roots }); } catch { [found] = sel.getComposedRanges(...roots); }
  }
  if (!found || found.collapsed) return null;
  const range = new Range();
  try { range.setStart(found.startContainer, found.startOffset); range.setEnd(found.endContainer, found.endOffset); } catch { return null; }
  return range.collapsed ? null : range;
}

const elementOf = node => node.nodeType === 1 ? node : node.parentElement;

// The DOM as frontend.Html. Attributes are read raw, so a task image keeps its relative src.
const tree = node => toList([...node.childNodes].flatMap(child =>
  child.nodeType === 3 ? [new HtmlText(child.data)]
  : child.nodeType === 1 && !SKIP.has(child.tagName)
    ? [new Tag(child.tagName.toLowerCase(), toList([...child.attributes].map(a => [a.name, a.value])), tree(child))]
    : []));

function copy(doc) {
  doc.addEventListener('copy', e => {
    if ((e.composedPath?.()[0] || e.target)?.closest?.('input,textarea') || inEditor(e) || !e.clipboardData) return;
    const range = selected();
    if (!range) return;
    const inMarkdown = node => !!elementOf(node)?.closest?.('.md');
    let node = range.cloneContents();
    if (!inMarkdown(range.startContainer) && !inMarkdown(range.endContainer) && !node.querySelector('.md')) return;
    // A selection within bold, code or a link keeps its markup, and items of one list keep the list. Block ancestors
    // aren't kept: a word in a list item copies without the item's marker, and part of a code block without backticks.
    for (let el = elementOf(range.commonAncestorContainer); el && !el.classList.contains('md'); el = el.parentElement) {
      const list = el === range.commonAncestorContainer && /^(UL|OL)$/.test(el.tagName);
      const inline = INLINE.has(el.tagName) && !(el.tagName === 'CODE' && el.parentElement?.tagName === 'PRE');
      if (!list && !inline) continue;
      const outer = el.cloneNode(false);
      outer.append(node);
      node = outer;
    }
    const box = document.createElement('div');
    box.append(node);
    e.clipboardData.setData('text/plain', to_markdown(tree(box)));
    e.clipboardData.setData('text/html', box.innerHTML);
    e.preventDefault();
  });
}

// Find in page (Ctrl+F). The webview has no find of its own, and native find misses shadow DOM, where every page lives.
// The bar and the marks are plain DOM outside #app, so Lustre never diffs them. Matches are drawn as boxes over the
// text, not with the CSS Highlight API: WebKit doesn't paint highlights on user-select:none text such as cards
// (https://bugs.webkit.org/show_bug.cgi?id=278455). A match lies within one text node: a phrase split across elements is not found.
const LIMIT = 1000;
const SKIP = new Set(['STYLE', 'SCRIPT', 'TEMPLATE', 'NOSCRIPT']);

// The element that contains a node's box, crossing out of shadow roots.
const up = el => el.parentElement || el.getRootNode?.().host || null;

// Whether something else is on top of a match's text at a point: a sticky lane header, a modal, a notice.
// The marks layer has pointer-events:none, so it is never the element found.
function covered(el, x, y) {
  const root = el.getRootNode();
  const top = (root.elementFromPoint ? root : document).elementFromPoint(x, y);
  return !!top && !el.contains(top) && !top.contains(el);
}

// Scrolls each scroll area around a range, innermost first, then the page, so the match ends up centred where
// there is room; scrolling stops at each area's ends.
function center(range) {
  const mid = (a, b) => (a + b) / 2;
  for (let el = range.startContainer.parentElement; el; el = up(el)) {
    if (el === document.body || el === document.documentElement) break;
    const s = getComputedStyle(el);
    const y = /auto|scroll/.test(s.overflowY) && el.scrollHeight > el.clientHeight;
    const x = /auto|scroll/.test(s.overflowX) && el.scrollWidth > el.clientWidth;
    if (!x && !y) continue;
    const r = range.getBoundingClientRect(), c = el.getBoundingClientRect();
    el.scrollBy(x ? mid(r.left, r.right) - mid(c.left, c.right) : 0, y ? mid(r.top, r.bottom) - mid(c.top, c.bottom) : 0);
  }
  const r = range.getBoundingClientRect();
  scrollBy(r.left < 0 || r.right > innerWidth ? mid(r.left, r.right) - innerWidth / 2 : 0, mid(r.top, r.bottom) - innerHeight / 2);
}

// The visible part of an element's area: the viewport cut by every ancestor that clips its overflow.
function clipOf(el, cache) {
  if (!el) return { top: 0, left: 0, bottom: innerHeight, right: innerWidth };
  if (cache.has(el)) return cache.get(el);
  let clip = clipOf(up(el), cache);
  const style = getComputedStyle(el);
  if (style.overflowX !== 'visible' || style.overflowY !== 'visible') {
    const r = el.getBoundingClientRect();
    clip = { top: Math.max(clip.top, r.top), left: Math.max(clip.left, r.left), bottom: Math.min(clip.bottom, r.bottom), right: Math.min(clip.right, r.right) };
  }
  cache.set(el, clip);
  return clip;
}

function find(doc) {
  const bar = document.createElement('div');
  bar.className = 'find';
  bar.setAttribute('role', 'search');
  bar.hidden = true;
  bar.innerHTML = '<label for="solace-find-input" class="find-label">Find</label>'
    + '<input id="solace-find-input" type="search" autocomplete="off" spellcheck="false">'
    + '<span class="find-count" aria-live="polite"></span>'
    + '<button type="button" data-step="back" aria-label="Previous match">↑</button>'
    + '<button type="button" data-step="next" aria-label="Next match">↓</button>'
    + '<button type="button" data-step="close" aria-label="Close find">✕</button>';
  const input = bar.querySelector('input');
  const counter = bar.querySelector('.find-count');
  // The marks layer lets every click and drag through to the page.
  const marks = document.createElement('div');
  marks.className = 'find-marks';
  marks.setAttribute('aria-hidden', 'true');
  document.body.append(marks, bar);

  let ranges = [];
  let index = -1;
  let returnTo = null;
  let timer = 0;
  // Whether the page shown changed since the last search: the next one starts from what's on screen, as a new
  // query does, since the current match went with the page left.
  let switched = false;
  // When the first change still waiting for a search came, so a steady stream of changes still re-searches.
  let waiting = 0;
  let frame = 0;
  // The per-frame check for movement, and where the current match was last drawn.
  let following = 0;
  let drawnAt = '';
  // Each open shadow root gets its own observer; a change inside one re-runs the search. Attributes count too:
  // opening or closing a <details> (notes, transcript entries) shows or hides text without a childList change.
  // A component drawn inside a page later (a task's <solace-task> in the board) brings its own root to watch.
  // Text changing where nothing is drawn (a hidden page, a closed <details>) can't move a match: it is ignored.
  // Layout can shift with no mutation at all (images loading, fonts swapping): the boxes follow.
  const resized = new ResizeObserver(() => redraw());
  const { watch, unwatch } = watch_shadow_hosts({
    observe: { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ['open', 'hidden', 'class', 'style'] },
    changed: (records, shadow) => {
      if (!bar.hidden && records.some(r => r.type === 'attributes' || drawn(r.target))) { redraw(); schedule(); }
    },
    host: (el, shadow) => {
      resized.observe(el);
      // Scroll and toggle events aren't composed: they never leave the shadow root.
      const scroll = redraw;
      const toggle = () => { if (!bar.hidden) { redraw(); schedule(); } };
      shadow.addEventListener('scroll', scroll, { capture: true, passive: true });
      shadow.addEventListener('toggle', toggle, true);
      return () => {
        resized.unobserve(el);
        shadow.removeEventListener('scroll', scroll, { capture: true });
        shadow.removeEventListener('toggle', toggle, true);
      };
    },
  });
  // The document observer discovers new components, sees a page shown or hidden (a menu tab switched: the
  // shell sets `hidden` on its pages, outside any shadow root), and sees text taken out of the page (a lane
  // collapsed): the count drops with the marks. The bar's and the marks' changes never trigger a search.
  new MutationObserver(records => {
    let changed = false;
    for (const r of records) {
      if (bar.contains(r.target) || marks.contains(r.target)) continue;
      if (r.type === 'attributes') { changed = true; switched = true; continue; }
      for (const n of r.addedNodes) if (n.nodeType === 1 && n !== marks && n !== bar) { watch(n); changed = true; }
      for (const n of r.removedNodes) {
        if (n.nodeType === 1 && !n.isConnected) unwatch(n);
        changed = true;
      }
    }
    if (changed && !bar.hidden) schedule();
  }).observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['hidden'] });

  // A search 120 ms after the last change, but no later than 500 ms after the first: a run's transcript can
  // change more often than every 120 ms for as long as it runs.
  const schedule = () => {
    clearTimeout(timer);
    const now = performance.now();
    if (!waiting) waiting = now;
    timer = setTimeout(search, Math.max(0, Math.min(120, waiting + 500 - now)));
  };

  // Whether a node is drawn: not in a hidden page, a closed <details>, or anything else display:none,
  // visibility:hidden or opacity:0. An element with display:contents has no box of its own, so its parent answers for it.
  const drawn = node => {
    let el = node.nodeType === 1 ? node : node.parentElement || node.host || node.parentNode?.host;
    while (el && getComputedStyle(el).display === 'contents') el = up(el);
    return !el || !el.checkVisibility || el.checkVisibility({ visibilityProperty: true, opacityProperty: true });
  };

  // The text nodes that can be drawn: hidden pages, display:none elements and the inside of a closed <details>
  // (all but its summary) aren't searched.
  const collect = (node, out) => {
    const closed = node.tagName === 'DETAILS' && !node.open;
    for (let child = node.firstChild; child; child = child.nextSibling) {
      if (closed && child.tagName !== 'SUMMARY') continue;
      if (child.nodeType === 3) out.push(child);
      else if (child.nodeType === 1) {
        if (child === bar || child === marks || SKIP.has(child.tagName)) continue;
        if (getComputedStyle(child).display === 'none') continue;
        if (child.shadowRoot) { watch(child); collect(child.shadowRoot, out); }
        collect(child, out);
      }
    }
    return out;
  };

  // Where the current match's text is now, and which matches are still in the page, to tell when to redraw:
  // collapsing a lane or closing a <details> takes text out, and its marks must go with it.
  const place = () => {
    const r = ranges[index]?.getBoundingClientRect();
    return r ? `${r.left},${r.top},${r.width},${r.height}` : '';
  };
  const seen = () => ranges.map(r => r.startContainer.parentElement?.isConnected ? 1 : 0).join('');

  // Draws a box over each visible part of each match, clipped to the scroll areas around it. Text taken out
  // of the page, or made invisible since the search, gets no box.
  const draw = () => {
    frame = 0;
    const boxes = [];
    const cache = new Map();
    ranges.forEach((range, i) => {
      const parent = range.startContainer.parentElement;
      if (!parent?.isConnected || !drawn(range.startContainer)) return;
      const clip = clipOf(parent, cache);
      for (const r of range.getClientRects()) {
        const top = Math.max(r.top, clip.top), left = Math.max(r.left, clip.left);
        const bottom = Math.min(r.bottom, clip.bottom), right = Math.min(r.right, clip.right);
        if (bottom <= top || right <= left) continue;
        const x = (left + right) / 2, y = (top + bottom) / 2;
        if (covered(parent, x, y) || bar.contains(document.elementFromPoint(x, y))) continue;
        const box = document.createElement('div');
        box.className = i === index ? 'find-mark current' : 'find-mark';
        box.style.cssText = `top:${top}px;left:${left}px;width:${right - left}px;height:${bottom - top}px`;
        boxes.push(box);
      }
    });
    marks.replaceChildren(...boxes);
    drawnAt = place() + '|' + seen();
  };
  const redraw = () => { if (!frame && ranges.length) frame = requestAnimationFrame(draw); };
  // Movement nothing reports (a transition, an element sliding in) shows as the current match no longer being
  // where it was drawn, and a match taken out of the page (a lane collapsed, a card removed) shows as it no
  // longer being there; the bar between them keeps a match moving from looking like one vanishing. Checked
  // every frame, only while the bar is open and there are matches.
  const follow = () => {
    following = requestAnimationFrame(follow);
    if (!frame && place() + '|' + seen() !== drawnAt) draw();
  };
  const track = () => {
    const wanted = !bar.hidden && ranges.length > 0;
    if (wanted && !following) following = requestAnimationFrame(follow);
    else if (!wanted && following) { cancelAnimationFrame(following); following = 0; }
  };

  const paint = () => {
    counter.textContent = count_text(input.value, index, ranges.length);
    cancelAnimationFrame(frame);
    draw();
    track();
  };

  const reveal = () => {
    const range = ranges[index];
    if (!range) return;
    // Only a match that isn't fully in view moves the page, so typing doesn't jump.
    if (!onScreen(range, new Map())) center(range);
    paint();
  };

  function search(keepPlace = true) {
    clearTimeout(timer);
    waiting = 0;
    if (switched) { keepPlace = false; switched = false; }
    const query = input.value.toLowerCase();
    const found = [];
    if (query) {
      for (const text of collect(document.body, [])) {
        const value = text.data.toLowerCase();
        for (let at = value.indexOf(query); at !== -1 && found.length < LIMIT; at = value.indexOf(query, at + query.length)) {
          const range = new Range();
          range.setStart(text, at);
          range.setEnd(text, at + query.length);
          // Hidden text (display:none, collapsed) has no boxes and can't be seen or scrolled to.
          // Visually hidden labels (.sr-only, 1px and clipped) can't be seen either, and text made invisible
          // (visibility:hidden, opacity:0) has boxes but is never marked.
          const box = text.parentElement?.getBoundingClientRect();
          if (range.getClientRects().length && box && box.width > 1 && box.height > 1 && drawn(text)) found.push(range);
        }
        if (found.length >= LIMIT) break;
      }
    }
    // A live update keeps the current match when its text is still there, even if text was added above it.
    const was = ranges[index];
    const same = was ? found.findIndex(r => r.startContainer === was.startContainer && r.startOffset === was.startOffset) : -1;
    ranges = found;
    index = ranges.length === 0 ? -1 : !keepPlace ? fromView() : same >= 0 ? same : Math.min(Math.max(index, 0), ranges.length - 1);
    paint();
  }

  // A new query starts from what's on screen: the first match in view, else the first below it, else the first.
  const fromView = () => {
    const cache = new Map();
    const shown = ranges.findIndex(r => onScreen(r, cache));
    if (shown >= 0) return shown;
    const below = ranges.findIndex(r => r.getBoundingClientRect().top >= 0);
    return below >= 0 ? below : 0;
  };
  const onScreen = (range, cache) => {
    const parent = range.startContainer.parentElement;
    if (!parent) return false;
    const r = range.getBoundingClientRect(), clip = clipOf(parent, cache);
    return r.top >= Math.max(clip.top, 56) && r.bottom <= clip.bottom && r.left >= clip.left && r.right <= clip.right;
  };

  const step = back => {
    index = next_match(index, ranges.length, back);
    reveal();
  };

  const open = () => {
    if (bar.hidden) {
      // Focus inside a component shows on the document as the host; keep the element itself.
      let active = document.activeElement;
      while (active?.shadowRoot?.activeElement) active = active.shadowRoot.activeElement;
      returnTo = active && active !== document.body ? active : null;
      bar.hidden = false;
      watch(document.body);
      if (input.value) search();
    }
    input.focus();
    input.select();
  };

  const close = () => {
    bar.hidden = true;
    clearTimeout(timer);
    waiting = 0;
    ranges = [];
    index = -1;
    paint();
    // Restore focus for the keyboard, without scrolling the page to the element focused before Ctrl+F.
    returnTo?.focus?.({ preventScroll: true });
    returnTo = null;
  };

  doc.addEventListener('keydown', e => {
    if (find_opens(!!e.ctrlKey, !!e.metaKey, !!e.altKey, e.key || '')) {
      e.preventDefault();
      open();
    }
  }, true);
  // Scrolling any lane or the page, or resizing, moves the text under the marks.
  doc.addEventListener('scroll', redraw, { capture: true, passive: true });
  addEventListener('resize', redraw);

  input.addEventListener('input', () => { search(false); reveal(); });
  input.addEventListener('keydown', e => {
    if (e.key === 'Enter') { e.preventDefault(); step(e.shiftKey); }
  });
  // Esc closes from anywhere in the bar, and from the page when nothing has focus; a page's own Esc (a modal) keeps it.
  bar.addEventListener('keydown', e => {
    if (e.key === 'Escape') { e.preventDefault(); close(); }
  });
  doc.addEventListener('keydown', e => {
    if (e.key === 'Escape' && !bar.hidden && (document.activeElement === document.body || !document.activeElement)) close();
  });
  bar.addEventListener('click', e => {
    const which = e.target.closest?.('button')?.dataset.step;
    if (which === 'close') close();
    else if (which) step(which === 'back');
  });

  watch(document.body);
}
