import { test, expect } from 'bun:test';
import { readFileSync } from 'node:fs';

// The About tab embeds the license, so it must stay the text the project ships.
const words = text => (text.toLowerCase().match(/[a-z0-9]+/g) ?? []).join(' ');
const plain = html => html.replace(/<[^>]*>/g, ' ').replace(/&[a-z#0-9]+;/gi, ' ');

test('the About license matches LICENSE.md', async () => {
  const settings = await import('../build/dev/javascript/settings/settings.mjs');
  const element = await import('../build/dev/javascript/lustre/lustre/element.mjs');
  const [model] = settings.update(settings.initial(false), new settings.Select(new settings.About()));
  const page = element.to_string(settings.view(model));
  const start = page.indexOf('<div class="md">', page.indexOf('<details class="about-license">'));
  expect(start).toBeGreaterThan(-1);
  const shown = page.slice(start, page.indexOf('</div>', start));
  const license = readFileSync(new URL('../../../LICENSE.md', import.meta.url), 'utf8');
  expect(words(plain(shown))).toBe(words(license));
});
