"""Browser flow for editor drafts: type, leave, return, then send or save.

Usage: draft_flow.py
SOLACE_TEST_BIN names the solace binary; without it, the script rebuilds the
frontend bundle, then the binary, because the pages are built into the binary.
Setup (once, by the human): pip install playwright; playwright install chromium.
"""
import sys
import os
from pathlib import Path
import subprocess

from layout_audit import Fixture, READY, fail


CLI = Path(__file__).resolve().parent.parent


def binary():
    if os.environ.get('SOLACE_TEST_BIN'):
        return os.environ['SOLACE_TEST_BIN']
    bundle = subprocess.run(
        ['sh', '-c', 'gleam build && bun build src/main.mjs --outfile priv/app.js --minify --target browser'],
        cwd=CLI/'frontend',
        text=True,
        capture_output=True,
    )
    if bundle.returncode != 0:
        fail('frontend build failed:\n' + bundle.stderr[-4000:], 1)
    built = subprocess.run(
        ['cargo', 'build', '--manifest-path', str(CLI/'Cargo.toml')],
        text=True,
        capture_output=True,
    )
    if built.returncode != 0:
        fail('cargo build failed:\n' + built.stderr[-4000:], 1)
    return str(CLI/'target/debug/solace')


DEEP = r"""
  const deep = (root, match) => {
    const hit = match(root);
    if (hit) return hit;
    for (const el of root.querySelectorAll('*')) {
      if (el.shadowRoot) {
        const found = deep(el.shadowRoot, match);
        if (found) return found;
      }
    }
    return null;
  };
"""

VALUE = r"""
([id, text]) => {
""" + DEEP + r"""
  const el = deep(document, root => root.querySelector('#' + id));
  return !!el && el.value === text;
}
"""

ABSENT = r"""
(id) => {
""" + DEEP + r"""
  return !deep(document, root => root.querySelector('#' + id));
}
"""

FIND = r"""
(selector) => {
""" + DEEP + r"""
  return deep(document, root => root.querySelector(selector));
}
"""

CLICK = r"""
(selector) => {
""" + DEEP + r"""
  const el = deep(document, root => root.querySelector(selector));
  if (!el) return false;
  el.click();
  return true;
}
"""

CLICK_TEXT = r"""
([selector, text]) => {
""" + DEEP + r"""
  const el = deep(document, root => {
    for (const hit of root.querySelectorAll(selector)) {
      if ((hit.textContent || '').trim().startsWith(text)) return hit;
    }
    return null;
  });
  if (!el) return false;
  el.click();
  return true;
}
"""


def wait_for_draft(page, key, text):
    page.wait_for_function(
        '([key, text]) => localStorage.getItem(key) === text',
        arg=[key, text],
        timeout=10000,
    )


def wait_for_value(page, editor, text):
    page.wait_for_function(VALUE, arg=[editor, text], timeout=10000)


def open_task(page, url):
    page.goto(url, wait_until='load')
    page.wait_for_function(READY, arg='.detail', timeout=15000)


def click(page, selector, what):
    page.wait_for_function(FIND, arg=selector, timeout=10000)
    if not page.evaluate(CLICK, selector):
        fail(f'could not find {what}')
    page.wait_for_timeout(100)


def click_text(page, selector, text, what):
    if not page.evaluate(CLICK_TEXT, [selector, text]):
        fail(f'could not find {what}')
    page.wait_for_timeout(100)


def type_in(page, editor, text):
    if not page.evaluate(
        r"""
(id) => {
""" + DEEP + r"""
          const el = deep(document, root => root.querySelector('#' + id));
          const content = el?.shadowRoot?.querySelector('.cm-content');
          if (!content) return false;
          content.focus();
          return true;
        }
""",
        editor,
    ):
        fail(f'could not focus the {editor} editor')
    page.keyboard.type(text, delay=10)


def main():
    fixture = Fixture(binary())
    try:
        fixture.start()
        task = fixture.api('POST', '/p/a/tasks', {'title': 'Draft flow', 'body': ''})
        url = fixture.url + '/task/' + task['id']
        note_key = f"solace-draft:note-text:{task['id']}"
        body_key = f"solace-draft:edit-body:{task['id']}"

        from playwright.sync_api import sync_playwright

        with sync_playwright() as pw:
            try:
                browser = pw.chromium.launch()
            except Exception as error:
                if "Executable doesn't exist" in str(error):
                    fail('Chromium for Playwright is missing: run `playwright install chromium`.')
                raise
            page = browser.new_page(viewport={'width': 1440, 'height': 900})
            try:
                open_task(page, url)
                click_text(page, '[role=tab]', 'Notes', 'the Notes tab')
                page.wait_for_function(FIND, arg='button.add-note', timeout=10000)
                click(page, 'button.add-note', 'Add note')
                type_in(page, 'note-text', 'A browser note')
                wait_for_draft(page, note_key, 'A browser note')

                page.goto(url, wait_until='load')
                page.wait_for_function(READY, arg='#note-text', timeout=15000)
                wait_for_value(page, 'note-text', 'A browser note')

                click(
                    page,
                    'button.composer-action[aria-label="Add note"]',
                    'the note send',
                )
                page.wait_for_function(ABSENT, arg='note-text', timeout=10000)
                page.wait_for_function(
                    'key => localStorage.getItem(key) === null',
                    arg=note_key,
                    timeout=10000,
                )

                page.goto(url, wait_until='load')
                page.wait_for_function(READY, arg='.detail', timeout=15000)
                page.wait_for_timeout(500)
                if page.evaluate(FIND, '#note-text'):
                    fail('a sent note draft came back after reload')

                click_text(page, 'button', 'Edit', 'Edit')
                type_in(page, 'edit-body', 'A browser description')
                wait_for_draft(page, body_key, 'A browser description')

                page.goto(url, wait_until='load')
                page.wait_for_function(READY, arg='#edit-body', timeout=15000)
                wait_for_value(page, 'edit-body', 'A browser description')

                click_text(page, 'button', 'Save', 'Save')
                page.wait_for_function(ABSENT, arg='edit-body', timeout=10000)
                page.wait_for_function(
                    'key => localStorage.getItem(key) === null',
                    arg=body_key,
                    timeout=10000,
                )
                print('draft_flow: note and description drafts restored, then cleared')
            finally:
                page.close()
                browser.close()
    finally:
        fixture.close()
    return 0


if __name__ == '__main__':
    sys.exit(main())
