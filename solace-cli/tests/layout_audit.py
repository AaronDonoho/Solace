"""Layout audit: measured numbers for the visual alignment pass, plus screenshots.

Agents can't see pages, so this loads each page of a fixture Solace instance in
headless Chromium, in the light and the dark theme, and prints what a reviewer
would look at: control heights (a label styled as a button, the editor's Image
tool, counts as one), label-to-control baselines, padding, the tab bar's
position, and how the sticky lane header lines up with its lanes, at the top
and again with the lanes scrolled under it (a short viewport, extra PNG), and
the width of every task element. The board audit also opens a task in the
board's band at several window sizes: on the side, at its smallest width,
collapsed on the side, and at the bottom with two columns and with one, and
measures the band, its bar and where the bar's items sit in it.

Usage: layout_audit.py [page]
Pages: board, strategy (all tabs), strategy-vision, strategy-features,
strategy-roadmap, strategy-tasks, settings, task. No argument audits them all.
SOLACE_TEST_BIN names the solace binary; without it, the script builds one with
cargo, because the pages are built into the binary.
Setup (once, by the human): pip install playwright; playwright install chromium.
"""
import datetime
import http.client
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
CLI = HERE.parent
FAKE = str(HERE/'fake_agent.py')
STRATEGY_TABS = ['vision', 'features', 'roadmap', 'tasks']
PAGES = ['board', *('strategy-' + t for t in STRATEGY_TABS), 'settings', 'task']
THEMES = ['light', 'dark']
VIEWPORT = {'width': 1440, 'height': 900}

def fail(line, code=2):
    print('layout_audit: ' + line)
    sys.exit(code)

def pages_named(arg):
    if arg is None: return PAGES
    if arg == 'strategy': return [p for p in PAGES if p.startswith('strategy-')]
    if arg in PAGES: return [arg]
    fail(f'unknown page "{arg}"; pages: board, strategy, {", ".join(PAGES[1:])}', 1)

def binary():
    if os.environ.get('SOLACE_TEST_BIN'): return os.environ['SOLACE_TEST_BIN']
    p = subprocess.run(['cargo', 'build', '--manifest-path', str(CLI/'Cargo.toml')], text=True, capture_output=True)
    if p.returncode != 0: fail('cargo build failed:\n' + p.stderr[-4000:], 1)
    return str(CLI/'target/debug/solace')

class Fixture:
    """A temp Solace root with projects a and b, as workspace_acceptance builds one."""
    def __init__(self, bin):
        self.bin = bin
        self.tmp = tempfile.mkdtemp(prefix='solace-layout-')
        self.server = None
    def start(self):
        self.root = Path(self.tmp)/'fixture'
        self.root.mkdir()
        (self.root/'.git').mkdir()
        config = self.root/'.git/system.toml'
        config.write_text('task_timeout=20\n[[models]]\nid="fake"\nlabel="Fake agent"\nagent="codex"\nrun=["python3",' + json.dumps(FAKE) + ']\n')
        self.env = {**os.environ, 'SOLACE_CONFIG': str(config)}
        for d in ['a', 'b']:
            (self.root/d/'src').mkdir(parents=True)
            (self.root/d/'gleam.toml').write_text(f'name="{d}"\nversion="1.0.0"\ntarget="javascript"\n')
            (self.root/d/'src/app.gleam').write_text('pub fn one() { 1 }\n')
        self.cli('migrate', '--yes', '--project', 'a', '--project', 'b')
        self.cli('migrate', '--adopt', '--yes')
        self.log = open(Path(self.tmp)/'serve.log', 'w')
        self.server = subprocess.Popen([self.bin, 'serve', '--port', '0'], cwd=self.root, env=self.env, stdout=subprocess.PIPE, stderr=self.log, text=True)
        line = self.server.stdout.readline()
        if 'listening' not in line: fail('solace serve did not start: ' + line + (Path(self.tmp)/'serve.log').read_text(), 1)
        self.port = int(line.strip().rsplit(':', 1)[1])
        self.url = f'http://127.0.0.1:{self.port}'
    def cli(self, *args):
        p = subprocess.run([self.bin, *args], cwd=self.root, env=self.env, text=True, capture_output=True, timeout=120)
        if p.returncode != 0: fail(f'solace {" ".join(args)} failed: ' + p.stdout + p.stderr, 1)
    def api(self, method, url, body=None):
        c = http.client.HTTPConnection('127.0.0.1', self.port, timeout=30)
        c.request(method, url, None if body is None else json.dumps(body), {'Content-Type': 'application/json'})
        r = c.getresponse(); v = json.loads(r.read()); c.close()
        if r.status != 200: fail(f'{method} {url} answered {r.status}: {v}', 1)
        return v
    def seed(self):
        """Content for every page; returns the task the detail page opens."""
        vision = self.api('GET', '/vision')
        self.api('PUT', '/vision', {'text': '# Why\nA calm board for agent work.\n\n- Humans set direction\n- Agents do the work', 'base': vision['revision']})
        m = self.api('POST', '/milestones', {'title': '1.0 Beta'})
        # An active feature's tasks show on the board; an upcoming one's stay on Strategy (D-128).
        layout = self.api('POST', '/features', {'title': 'Layout pass', 'milestone': m['id'], 'state': 'active'})
        themes = self.api('POST', '/features', {'title': 'Themes', 'milestone': m['id']})
        tasks = []
        for base, title, feature in [('/p/a', 'Align button heights', layout), ('/p/a', 'Sticky lane header', layout),
                                     ('/p/b', 'Settings spacing', None), ('/p/b', 'Dark theme contrast', themes),
                                     ('/p/a', 'Backlog idea', None)]:
            t = self.api('POST', base + '/tasks', {'title': title, 'body': '[]'})
            if feature: self.api('PATCH', f'{base}/tasks/{t["id"]}', {'feature': feature['id']})
            tasks.append((base, t))
        for base, t in tasks[:4]:
            # A task linked to an active feature has left Backlog already.
            if self.api('GET', f'{base}/tasks/{t["id"]}')['column'] == 'backlog':
                self.api('POST', f'{base}/tasks/{t["id"]}/actions', {'action': 'plan'})
        # One finished task: its card carries the Archive button (D-95).
        base, t = tasks[2]
        self.api('PATCH', f'{base}/tasks/{t["id"]}', {'body': json.dumps([{'command': 'task done Spacing aligned.'}])})
        self.api('POST', f'{base}/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        end = time.monotonic() + 30
        while self.api('GET', f'{base}/tasks/{t["id"]}')['column'] != 'needs_review':
            if time.monotonic() > end: fail('the fixture run did not finish', 1)
            time.sleep(.1)
        self.api('POST', f'{base}/tasks/{t["id"]}/actions', {'action': 'approve'})
        self.api('POST', f'{base}/tasks/{t["id"]}/actions', {'action': 'complete'})
        # One task with a finished run, notes and a report, for the detail page.
        base, t = tasks[0]
        self.api('POST', f'{base}/tasks/{t["id"]}/notes', {'text': 'Buttons in the detail band look **taller** than the tabs.'})
        self.api('PATCH', f'{base}/tasks/{t["id"]}', {'body': json.dumps([{'command': 'task done Measured and aligned.'}])})
        self.api('POST', f'{base}/tasks/{t["id"]}/actions', {'action': 'assign', 'model': 'fake'})
        end = time.monotonic() + 30
        while self.api('GET', f'{base}/tasks/{t["id"]}')['column'] != 'needs_review':
            if time.monotonic() > end: fail('the fixture run did not finish', 1)
            time.sleep(.1)
        return t
    def close(self):
        if self.server:
            self.server.terminate()
            try: self.server.wait(timeout=3)
            except subprocess.TimeoutExpired: self.server.kill()
            self.server.stdout.close(); self.log.close()
        shutil.rmtree(self.tmp, ignore_errors=True)

# Runs in the page. The components draw into shadow roots, so every query walks
# them; a selector names the shadow hosts on the way, then the element.
MEASURE = r"""
() => {
  const all = [];
  const walk = (root, hosts) => {
    for (const el of root.querySelectorAll('*')) {
      all.push([el, hosts]);
      if (el.shadowRoot) walk(el.shadowRoot, [...hosts, el.localName]);
    }
  };
  walk(document, []);
  const r1 = x => Math.round(x * 10) / 10;
  const shown = el => { const r = el.getBoundingClientRect(), s = getComputedStyle(el); return r.width > 0 && r.height > 0 && s.visibility !== 'hidden'; };
  const simple = el => {
    let s = el.localName + [...el.classList].slice(0, 2).map(c => '.' + c).join('');
    if (el.localName === 'input') s += '[type=' + (el.getAttribute('type') || 'text') + ']';
    const p = el.parentElement;
    return p && p.classList.length ? '.' + p.classList[0] + ' ' + s : s;
  };
  const path = (el, hosts) => [...hosts, simple(el)].join(' › ');
  const ctx = document.createElement('canvas').getContext('2d');
  const ascent = el => {
    const s = getComputedStyle(el);
    ctx.font = `${s.fontStyle} ${s.fontWeight} ${s.fontSize} ${s.fontFamily}`;
    const m = ctx.measureText('x');
    return [m.fontBoundingBoxAscent, m.fontBoundingBoxDescent];
  };
  // The baseline of an element's first line of text, in page coordinates.
  const textBaseline = (el, skip) => {
    const w = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    for (let n = w.nextNode(); n; n = w.nextNode()) {
      if (!n.data.trim() || (skip && skip.contains(n))) continue;
      const range = document.createRange(); range.selectNodeContents(n);
      const rect = range.getClientRects()[0];
      if (!rect) continue;
      return { y: rect.top + ascent(n.parentElement)[0], x: rect.left, text: n.data.trim().slice(0, 24) };
    }
    return null;
  };
  // A text field draws its line centered in its content box.
  const controlBaseline = el => {
    const t = el.localName === 'button' ? textBaseline(el) : null;
    if (t) return t.y;
    const r = el.getBoundingClientRect(), s = getComputedStyle(el), px = k => parseFloat(s[k]) || 0;
    const top = r.top + px('borderTopWidth') + px('paddingTop');
    const h = r.height - px('borderTopWidth') - px('borderBottomWidth') - px('paddingTop') - px('paddingBottom');
    const [a, d] = ascent(el);
    return top + (h - (a + d)) / 2 + a;
  };

  const controls = {};
  for (const [el, hosts] of all) {
    // A label styled as a button (the editor's Image tool) is a control, like the buttons.
    const label_button = el.localName === 'label' && el.classList.contains('button');
    if (!['button', 'input', 'select'].includes(el.localName) && !label_button) continue;
    if (!shown(el)) continue;
    if (el.localName === 'input' && ['hidden', 'checkbox', 'radio', 'file'].includes(el.type)) continue;
    const h = r1(el.getBoundingClientRect().height), sel = path(el, hosts);
    (controls[h] ??= {})[sel] = ((controls[h] ??= {})[sel] || 0) + 1;
  }

  const labels = [];
  for (const [el, hosts] of all) {
    if (el.localName !== 'label' || !shown(el)) continue;
    const control = el.control || el.querySelector('input,select,textarea,button');
    if (!control || !shown(control) || ['file', 'hidden'].includes(control.type)) continue;
    const t = textBaseline(el, control);
    if (!t) continue;
    const c = control.getBoundingClientRect();
    labels.push({ label: path(el, hosts), text: t.text, control: simple(control),
                  dy: r1(controlBaseline(control) - t.y), dx: r1(c.left - t.x),
                  place: c.top >= t.y ? 'below' : 'beside' });
  }

  const padding = {};
  for (const [el, hosts] of all) {
    if (!el.matches('.card,.detail,.editor,.lane,.lane-head,.column,.note,.run,.panel,section,article') || !shown(el)) continue;
    const s = getComputedStyle(el), key = path(el, hosts) + '  ' + [s.paddingTop, s.paddingRight, s.paddingBottom, s.paddingLeft].join(' ');
    padding[key] = (padding[key] || 0) + 1;
  }

  const tabs = [];
  for (const [el, hosts] of all) {
    if (!el.matches('.tabs,[role=tablist]') || !shown(el)) continue;
    const r = el.getBoundingClientRect();
    tabs.push({ sel: path(el, hosts), top: r1(r.top + scrollY), height: r1(r.height) });
  }

  const lanes = [];
  for (const [el, hosts] of all) {
    if (!el.classList.contains('lane-head') || !shown(el)) continue;
    const heads = [...el.children].filter(c => c.classList.contains('column'));
    const within = el.parentElement.closest('.lane');
    const container = within || el.parentElement;
    const rows = within ? [within] : [...container.querySelectorAll('.lane')];
    for (const lane of rows) {
      // The folder only: the package name and target badge sit beside it.
      const name = (lane.querySelector('.lane-title strong')?.textContent || '').trim().slice(0, 30);
      const board = [...lane.querySelectorAll('.board')].find(b => !b.classList.contains('lane-head'));
      const cols = board ? [...board.children].filter(c => c.classList.contains('column')) : [];
      const area = el.closest('.lanes-area');
      heads.forEach((h, i) => {
        const hr = h.getBoundingClientRect(), c = cols[i]?.getBoundingClientRect();
        lanes.push({ head: path(el, hosts), column: (h.textContent || '').trim(), lane: name,
                     hx: r1(hr.left), hw: r1(hr.width), ht: r1(hr.top),
                     at: area ? r1(area.getBoundingClientRect().top) : null,
                     lx: c ? r1(c.left) : null, lw: c ? r1(c.width) : null });
      });
    }
  }
  // A task element sized by its content would shrink to nothing: its
  // styles give it `container-type:inline-size`.
  const hosts = [];
  for (const [el, on] of all) {
    if (el.localName === 'solace-task') hosts.push({ sel: path(el, on), width: r1(el.getBoundingClientRect().width) });
  }
  return { controls, labels, padding, tabs, lanes, hosts };
}
"""

READY = r"""
(sel) => {
  const find = root => { if (root.querySelector(sel)) return true;
    for (const el of root.querySelectorAll('*')) if (el.shadowRoot && find(el.shadowRoot)) return true;
    return false; };
  return find(document);
}
"""

# Scrolls the board's lanes area, the scroll container the lane head sticks in;
# returns how far it went (0 when the lanes fit and nothing scrolls).
SCROLL = r"""
(by) => {
  const find = root => { const hit = root.querySelector('.lanes-area'); if (hit) return hit;
    for (const el of root.querySelectorAll('*')) { const f = el.shadowRoot && find(el.shadowRoot); if (f) return f; }
    return null; };
  const area = find(document);
  if (!area) return 0;
  area.scrollTop = Math.min(by, area.scrollHeight - area.clientHeight);
  return area.scrollTop;
}
"""
# A short viewport, so three small lanes overflow and the head has to stick.
SCROLLED_VIEWPORT = {'width': 1440, 'height': 520}
SCROLL_BY = 160

# The board's detail band with a task open: its layout and size, its bar's
# height, the task element's width and whether its details show in two
# columns, and for each item in the bar the space above it (to the band's top
# border) and below it (to the details' top edge).
BAND = r"""
() => {
  const find = (root, sel) => { const hit = root.querySelector(sel); if (hit) return hit;
    for (const el of root.querySelectorAll('*')) { const f = el.shadowRoot && find(el.shadowRoot, sel); if (f) return f; }
    return null; };
  const r1 = x => Math.round(x * 10) / 10;
  const band = find(document, '.detail-band');
  if (!band) return null;
  const work = band.parentElement, b = band.getBoundingClientRect();
  const host = band.querySelector('solace-task'), shadow = host?.shadowRoot;
  const detail = shadow?.querySelector('.detail')?.getBoundingClientRect();
  const bar = band.querySelector('.detail-bar');
  const top = b.top + parseFloat(getComputedStyle(band).borderTopWidth);
  const under = detail && detail.height > 0 ? detail.top : null;
  const items = bar ? [...bar.children].filter(c => c.getBoundingClientRect().height > 0).map(c => {
    const r = c.getBoundingClientRect();
    return { name: `${c.className || c.localName} "${(c.textContent || '').trim().slice(0, 16)}"`,
             above: r1(r.top - top), below: under === null ? null : r1(under - r.bottom), height: r1(r.height) };
  }) : [];
  const act = shadow?.querySelector('.act')?.getBoundingClientRect();
  const record = shadow?.querySelector('.record')?.getBoundingClientRect();
  const two = act && record && record.height > 0 && record.left >= act.right;
  const side = work.classList.contains('side');
  return { layout: (side ? 'side' : 'bottom') + (band.classList.contains('collapsed') ? ', collapsed' : ''),
           band_w: r1(b.width), band_h: r1(b.height), bar_h: bar ? r1(bar.getBoundingClientRect().height) : 0,
           rem: parseFloat(getComputedStyle(document.documentElement).fontSize),
           host_w: host ? r1(host.getBoundingClientRect().width) : null,
           columns: act && record && record.height > 0 ? (two ? 'two' : 'one') : 'hidden',
           gap: two ? r1(record.left - act.right) : null,
           lanes_margin: r1(parseFloat(getComputedStyle(work.querySelector('.lanes-area')).marginRight) || 0),
           items };
}
"""
# (name, window, stored band width, collapse after opening). A window wider
# than 1.4 times its height puts the band on the side.
BAND_CASES = [
    ('side', {'width': 1440, 'height': 900}, None, False),
    ('side at its smallest width', {'width': 1440, 'height': 900}, '100', False),
    ('side, collapsed', {'width': 1440, 'height': 900}, None, True),
    ('bottom, wide', {'width': 1600, 'height': 1200}, None, False),
    ('bottom, narrow', {'width': 1200, 'height': 1000}, None, False),
]

def report_band(name, window, m, shot):
    size = f'{window["width"]}×{window["height"]}'
    print(f'\n== task band · {name} · {size} ==')
    if not m:
        print('  no detail band')
        return
    rem = m['rem']
    print(f'  layout {m["layout"]}: band {m["band_w"]:.1f}×{m["band_h"]:.1f}px '
          f'(side minimum 32rem = {32 * rem:.0f}px), board margin {m["lanes_margin"]:.1f}px')
    gap = f', {m["gap"]:.1f}px apart (2rem = {2 * rem:.0f}px)' if m['gap'] is not None else ''
    print(f'  task element {m["host_w"]}px wide (two columns from 90rem = {90 * rem:.0f}px): {m["columns"]} column(s){gap}')
    print(f'  bar {m["bar_h"]:.1f}px high; items (space above to the band\'s top border, below to the details):')
    for i in m['items']:
        below = f'{i["below"]:.1f}' if i['below'] is not None else 'no details shown'
        print(f'    {i["name"]:<36} height {i["height"]:.1f} above {i["above"]:.1f} below {below}')
    print(f'screenshot {shot.name}')

def audit_band(browser, fixture, task, out):
    """Opens the task on the board in each of BAND_CASES and reports its band."""
    from playwright.sync_api import Error as PlaywrightError
    for i, (name, window, width, collapse) in enumerate(BAND_CASES):
        context = browser.new_context(viewport=window, color_scheme='light')
        stored = f"localStorage.setItem('solace-board-detail-width', '{width}')" if width else "localStorage.removeItem('solace-board-detail-width')"
        context.add_init_script(f"try {{ localStorage.setItem('solace-theme', 'light'); localStorage.setItem('solace-board-detail', '0'); localStorage.removeItem('solace-board-detail-height'); {stored} }} catch {{}}")
        page = context.new_page()
        page.goto(fixture.url + '/', wait_until='load')
        try:
            page.wait_for_function(READY, arg='.lane', timeout=10000)
            page.locator('button.card', has_text=task['title']).first.click(timeout=5000)
            page.wait_for_function(READY, arg='.detail', timeout=10000)
            if collapse: page.locator('button.detail-toggle').first.click(timeout=5000)
        except PlaywrightError as e:
            print(f'\nlayout_audit: task band · {name}: could not open the task: {str(e).splitlines()[0]}')
        page.evaluate('document.fonts.ready.then(() => 0)')
        page.wait_for_timeout(600)
        shot = out/f'band-{i + 1}.png'
        page.screenshot(path=str(shot))
        report_band(name, window, page.evaluate(BAND), shot)
        context.close()

def address(page, task):
    if page == 'board': return '/', '.lane'
    if page.startswith('strategy-'): return '/strategy?tab=' + page.split('-', 1)[1], 'button'
    if page == 'settings': return '/settings', 'button'
    return '/task/' + task['id'], '.detail'

def signed(x):
    return f'{x:+.1f}'

def report(name, theme, m, shot):
    print(f'\n== {name} · {theme} ==')
    print('controls (height ×count: selectors)')
    for h in sorted(m['controls'], key=float):
        groups = m['controls'][h]
        print(f'  {float(h):5.1f}px ×{sum(groups.values()):<3} ' + ', '.join(f'{s} ({n})' for s, n in sorted(groups.items())))
    if not m['controls']: print('  none')
    print('labels (control baseline − label baseline)')
    for l in sorted(m['labels'], key=lambda l: (l['label'], l['text'])) or []:
        print(f'  {l["label"]} "{l["text"]}" → {l["control"]}: dy {signed(l["dy"])} dx {signed(l["dx"])} ({l["place"]})')
    if not m['labels']: print('  none')
    print('padding (top right bottom left ×count)')
    for k in sorted(m['padding']): print(f'  {k} ×{m["padding"][k]}')
    print('tab bars (top in page, height)')
    for t in m['tabs']: print(f'  {t["sel"]}: top {t["top"]:.1f} height {t["height"]:.1f}')
    if not m['tabs']: print('  none')
    print('lane heads (head column vs lane column)')
    lane_heads(m['lanes'])
    print('task elements (width)')
    for t in m['hosts']: print(f'  {t["sel"]}: {t["width"]:.1f}px')
    if not m['hosts']: print('  none')
    print(f'screenshot {shot.name}')

def lane_heads(lanes, stuck=False):
    for l in lanes:
        lane = f'x {l["lx"]:.1f} w {l["lw"]:.1f} (dx {signed(l["lx"] - l["hx"])} dw {signed(l["lw"] - l["hw"])})' if l['lx'] is not None else 'no columns (collapsed)'
        top = f' top {signed(l["ht"] - l["at"])} from the area' if stuck and l['at'] is not None else ''
        print(f'  {l["column"]:<14} head x {l["hx"]:.1f} w {l["hw"]:.1f}{top} | {l["lane"] or "lane"}: {lane}')
    if not lanes: print('  none')

def report_scrolled(by, m, shot):
    """The lane head while the lanes scroll under it: it should stay at the
    lanes area's top (top +0.0) and keep lining up with the columns."""
    size = f'{SCROLLED_VIEWPORT["width"]}×{SCROLLED_VIEWPORT["height"]}'
    if not by:
        print(f'lane heads scrolled: the lanes area does not scroll at {size}')
        return
    print(f'lane heads scrolled {by:.0f}px at {size} (head top from the lanes area top)')
    lane_heads(m['lanes'], stuck=True)
    print(f'screenshot {shot.name}')

def main(args):
    pages = pages_named(args[0] if args else None)
    try:
        from playwright.sync_api import sync_playwright, Error as PlaywrightError
    except ImportError:
        fail('Playwright is missing: run `pip install playwright`, then `playwright install chromium`.')
    out = CLI/'target/layout-audit'/datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    with sync_playwright() as pw:
        try:
            browser = pw.chromium.launch()
        except PlaywrightError as e:
            if "Executable doesn't exist" in str(e): fail('Chromium for Playwright is missing: run `playwright install chromium`.')
            raise
        fixture = Fixture(binary())
        try:
            fixture.start()
            task = fixture.seed()
            out.mkdir(parents=True, exist_ok=True)
            for name in pages:
                url, ready = address(name, task)
                for theme in THEMES:
                    context = browser.new_context(viewport=VIEWPORT, color_scheme=theme)
                    context.add_init_script(f"try {{ localStorage.setItem('solace-theme', '{theme}') }} catch {{}}")
                    page = context.new_page()
                    page.goto(fixture.url + url, wait_until='load')
                    try: page.wait_for_function(READY, arg=ready, timeout=10000)
                    except PlaywrightError: print(f'\nlayout_audit: {name} · {theme}: no {ready} after 10 s; measured as it was')
                    page.evaluate('document.fonts.ready.then(() => 0)')
                    page.wait_for_timeout(600)
                    shot = out/f'{name}-{theme}.png'
                    page.screenshot(path=str(shot), full_page=True)
                    m = page.evaluate(MEASURE)
                    report(name, theme, m, shot)
                    if m['lanes']:
                        page.set_viewport_size(SCROLLED_VIEWPORT)
                        page.wait_for_timeout(300)
                        by = page.evaluate(SCROLL, SCROLL_BY)
                        page.wait_for_timeout(300)
                        shot = out/f'{name}-{theme}-scrolled.png'
                        page.screenshot(path=str(shot))
                        report_scrolled(by, page.evaluate(MEASURE), shot)
                    context.close()
            if 'board' in pages: audit_band(browser, fixture, task, out)
        finally:
            fixture.close()
            browser.close()
    print(f'\nscreenshots: {out}')

if __name__ == '__main__':
    main(sys.argv[1:])
