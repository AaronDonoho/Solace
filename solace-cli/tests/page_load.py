"""Browser load check: the built pages load, and their records arrive.

Usage: page_load.py
SOLACE_TEST_BIN names the solace binary; without it, the script rebuilds the
frontend bundle, then the binary, because the pages are built into the binary.
Setup (once, by the human): pip install playwright; playwright install chromium.
"""
import sys

from layout_audit import Fixture, READY, built_binary

WAIT_MS = 10000
SETTLE_MS = 500
VIEWPORT = {'width': 1440, 'height': 900}


def fail(line, code=2):
    print('page_load: ' + line)
    sys.exit(code)


# Uncaught worker errors do not reach the page's console. Keep them on the page
# so the check can read them after each page settles.
WATCH_WORKERS = r"""
(() => {
  window.__solaceWorkerErrors = [];
  const RealWorker = window.Worker;
  if (!RealWorker) return;
  window.Worker = class extends RealWorker {
    constructor(...args) {
      super(...args);
      this.addEventListener('error', event => {
        window.__solaceWorkerErrors.push(event.message || 'the worker failed');
      });
    }
  };
})()
"""

# A record is shown when one of its text nodes is in a visible element,
# including inside the pages' shadow roots.
SHOWS = r"""
(text) => {
  const skip = new Set(['SCRIPT', 'STYLE', 'TEMPLATE', 'NOSCRIPT']);
  const shown = el => {
    if (!el) return false;
    const r = el.getBoundingClientRect(), s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none';
  };
  const has = root => {
    for (const node of root.childNodes) {
      if (node.nodeType === Node.TEXT_NODE && node.data.includes(text) && shown(node.parentElement)) return true;
    }
    return false;
  };
  const walk = root => {
    for (const el of root.querySelectorAll('*')) {
      if (skip.has(el.localName)) continue;
      if (has(el)) return true;
      if (el.shadowRoot && walk(el.shadowRoot)) return true;
    }
    return false;
  };
  return has(document) || walk(document);
}
"""


def first_line(error):
    text = str(error).strip()
    return text.splitlines()[0] if text else type(error).__name__


def errors_seen(errors):
    return '; '.join(errors[:6]) or 'no page or worker errors'


def worker_errors(page):
    try:
        found = page.evaluate('window.__solaceWorkerErrors || []')
    except Exception:
        return []
    return [f'worker: {text}' for text in found]


def watch_worker(worker, errors):
    def console(message):
        if message.type == 'error':
            errors.append(f'worker console: {message.text}')
    try:
        worker.on('console', console)
    except Exception:
        pass
    try:
        worker.on('pageerror', lambda error: errors.append(f'worker page: {error}'))
    except Exception:
        pass


def seed(fixture):
    milestone = fixture.api('POST', '/milestones', {'title': '1.0 Beta'})
    feature = fixture.api('POST', '/features', {'title': 'Layout pass', 'milestone': milestone['id'], 'state': 'active'})
    task = fixture.api('POST', '/p/a/tasks', {'title': 'Align button heights', 'body': ''})
    base = f'/p/a/tasks/{task["id"]}'
    fixture.api('PATCH', base, {'feature': feature['id']})
    if fixture.api('GET', base)['column'] == 'backlog':
        fixture.api('POST', base + '/actions', {'action': 'plan'})
    return task


def check_page(browser, fixture, label, url, ready, text):
    from playwright.sync_api import Error as PlaywrightError, TimeoutError as PlaywrightTimeoutError
    errors = []
    context = browser.new_context(viewport=VIEWPORT, color_scheme='light')
    context.add_init_script(WATCH_WORKERS)
    page = context.new_page()
    page.on('console', lambda message: errors.append(f'console: {message.text}') if message.type == 'error' else None)
    page.on('pageerror', lambda error: errors.append(f'page: {error}'))
    page.on('worker', lambda worker: watch_worker(worker, errors))
    try:
        page.goto(fixture.url + url, wait_until='load')
        page.wait_for_function(READY, arg=ready, timeout=WAIT_MS)
        page.wait_for_function(SHOWS, arg=text, timeout=WAIT_MS)
    except PlaywrightTimeoutError as error:
        errors.extend(worker_errors(page))
        fail(f'{label}: {text!r} did not appear within {WAIT_MS // 1000}s ({first_line(error)}); {errors_seen(errors)}')
    except PlaywrightError as error:
        errors.extend(worker_errors(page))
        fail(f'{label}: {first_line(error)}; {errors_seen(errors)}')
    page.wait_for_timeout(SETTLE_MS)
    errors.extend(worker_errors(page))
    if errors:
        fail(f'{label}: {errors_seen(errors)}')
    print(f'page_load: {label}: {text!r} shown, no console or worker errors')
    page.close()
    context.close()


def main():
    try:
        from playwright.sync_api import sync_playwright, Error as PlaywrightError
    except ImportError:
        fail('Playwright is missing: run `pip install playwright`, then `playwright install chromium`.')
    fixture = Fixture(built_binary())
    try:
        fixture.start()
        task = seed(fixture)
        pages = [
            ('Board', '/', '.lane', task['title']),
            ('Strategy features', '/strategy?tab=features', 'button', 'Layout pass'),
            ('Strategy roadmap', '/strategy?tab=roadmap', '.milestone h3 .toggle', '1.0 Beta'),
        ]
        with sync_playwright() as pw:
            try:
                browser = pw.chromium.launch()
            except PlaywrightError as error:
                if "Executable doesn't exist" in str(error):
                    fail('Chromium for Playwright is missing: run `playwright install chromium`.')
                raise
            try:
                for page in pages:
                    check_page(browser, fixture, *page)
            finally:
                browser.close()
    finally:
        fixture.close()
    print('page_load: the built pages load and show the seeded task, feature and milestone')


if __name__ == '__main__':
    sys.exit(main())