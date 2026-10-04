import { test, expect } from 'bun:test';
import * as board from '../build/dev/javascript/solace_board/solace_board.mjs';
import { perform } from '../build/dev/javascript/lustre/lustre/effect.mjs';
import { to_string } from '../build/dev/javascript/lustre/lustre/element.mjs';

const base = { top: '/work/solace-migration', original: '/work/project', copy: '/work/solace-migration',
  step: null, candidates: [], report: null, blockers: [], undo: false, flagged: [], problem: null };

// Each state the server reports while a migration stops partway (handoff.md fixes 2, 5, 12, 13).
async function render(migration) {
  const saved = Object.fromEntries(['fetch', 'setTimeout', 'EventSource'].map(k => [k, Object.getOwnPropertyDescriptor(globalThis, k)]));
  globalThis.setTimeout = () => 0;
  globalThis.EventSource = class { addEventListener() {} };
  globalThis.fetch = async path => Response.json(path === '/migration' ? migration
    : path === '/projects' ? { workspace: false, projects: [], found: [] } : path.endsWith('/workflow') ? { columns: [] } : []);
  let model;
  const dispatch = msg => { let effect; [model, effect] = board.update(model, msg); run(effect); };
  const run = effect => perform(effect, dispatch, () => {}, () => {}, () => {}, () => {}, () => {}, () => {});
  try {
    let effect; [model, effect] = board.init(); run(effect);
    for (let i = 0; i < 100 && !model.migration[0]; i++) await Bun.sleep(5);
    return to_string(board.view(model));
  } finally {
    for (const [k, d] of Object.entries(saved)) d ? Object.defineProperty(globalThis, k, d) : delete globalThis[k];
  }
}

test('confirm_paths lists what could reach the original, with Continue and Abandon', async () => {
  const html = await render({ ...base, state: 'confirm_paths',
    flagged: [{ text: "run.sh mentions the original's path", status: 'not checked' }] });
  expect(html).toContain('nothing has run yet');
  expect(html).toContain('run.sh mentions the original');
  expect(html).toContain('Continue');
  expect(html).toContain('Abandon migration');
});

test('failed shows the problem and its report; copying offers Abandon', async () => {
  const report = { checked_at: '1', current: true, warnings: ["app: doesn't build, before or after the migration"],
    checks: [{ name: 'Coverage', status: 'failed', summary: "Solace couldn't read in every Gleam file",
      items: [{ text: "app/src/bad.gleam: isn't UTF-8 text", status: 'failed' }] }] };
  let html = await render({ ...base, state: 'failed', problem: 'The migration stopped: see Coverage', report });
  expect(html).toContain('The migration stopped: see Coverage');
  expect(html).toContain('bad.gleam');
  expect(html).toContain('before or after the migration');
  expect(html).toContain('Abandon migration');
  html = await render({ ...base, state: 'copying' });
  expect(html).toContain('an earlier one was interrupted');
  expect(html).toContain('Abandon migration');
});
