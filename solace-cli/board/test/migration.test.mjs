import { test, expect } from 'bun:test';
import * as board from '../build/dev/javascript/solace_board/solace_board.mjs';
import { perform } from '../build/dev/javascript/lustre/lustre/effect.mjs';
import { to_string } from '../build/dev/javascript/lustre/lustre/element.mjs';

const original = '/work/project';
const copy = '/work/solace-migration';
const state = (state, extra = {}) => ({ state, top: original, original: null, copy: null,
  step: null, candidates: [], report: null, blockers: [], undo: false, ...extra });
const report = { checked_at: "2026-10-04T18:00:00Z", current: false, checks: [
  { name: 'Coverage', status: 'verified', summary: 'Both projects copied', items: [{ text: 'app', status: null, detail: null }] },
  { name: 'Reconstruction', status: 'failed', summary: 'Unknown difference', items: [{ text: 'src/app.gleam', status: 'failed', detail: '- old\n+ <script>new</script>' }] },
  { name: 'Behaviour', status: 'not checked', summary: 'No smoke commands', items: [{ text: 'App behaviour' }] },
  { name: 'Paths', status: 'out of date', summary: 'Copy changed', items: [{ text: 'Script', status: 'not checked', detail: 'Review manually' }] },
] };

test('migration states, report, confirmations and recovery use the root HTTP API', async () => {
  const saved = Object.fromEntries(['fetch', 'setTimeout', 'EventSource', 'confirm', 'location'].map(k => [k, Object.getOwnPropertyDescriptor(globalThis, k)]));
  const requests = [], timers = [], confirmations = [];
  let reloads = 0, accept = false, offline = false;
  let migration = state('available', { candidates: [{ dir: '.', name: 'app' }, { dir: 'web', name: 'web' }] });
  let postReply = { code: 'E7020', message: 'A migration already exists' };
  let model;
  const noop = () => {};
  const execute = effect => perform(effect, dispatch, noop, noop, noop, noop, noop, noop);
  const dispatch = msg => { let effect; [model, effect] = board.update(model, msg); execute(effect); };
  const html = () => to_string(board.view(model));
  const until = async predicate => {
    for (let i = 0; i < 100; i++) { if (predicate()) return; await Bun.sleep(5); }
    throw new Error('Migration did not reach expected state: ' + html());
  };
  const poll = async () => {
    expect(timers).toHaveLength(1);
    timers.shift()();
    await until(() => timers.length === 1);
  };
  const posts = () => requests.filter(r => r.method === 'POST');
  Object.defineProperty(globalThis, 'location', { configurable: true, value: { pathname: '/', reload() { reloads++; } } });
  globalThis.confirm = question => { confirmations.push(question); return accept; };
  globalThis.setTimeout = (callback, ms) => { expect(ms).toBe(model.migration[0]?.state === "running" ? 1000 : 5000); timers.push(callback); return 0; };
  globalThis.EventSource = class { addEventListener() {} };
  globalThis.fetch = async (path, options) => {
    requests.push({ path, method: options.method, body: options.body && JSON.parse(options.body) });
    if (path === '/migration') {
      if (offline) throw new Error('Server restarting');
      return Response.json(migration);
    }
    if (path.startsWith('/migration/')) return Response.json(postReply, { status: postReply.code ? 409 : 200 });
    if (path === '/projects') return Response.json({ workspace: true,
      projects: [{ dir: 'web', name: 'web', base: '/p/web' }],
      found: [{ dir: 'old', name: 'old', imported: false, member: false }, { dir: 'stored', name: 'stored', imported: true, member: false }] });
    return Response.json(path.endsWith('/workflow') ? { columns: [] } : []);
  };
  try {
    let effect; [model, effect] = board.init(); execute(effect);
    await until(() => model.migration[0]?.state && model.listed && timers.length);
    expect(requests.some(r => r.path === '/migration' && r.method === 'GET')).toBe(true);
    expect(html()).toContain('Migrate this project');
    dispatch(new board.ToggleProjects());
    expect(html()).toContain('old (not set up)');
    expect(html()).toContain('Existing code is brought in by migrating.');
    expect(html().match(/>Add<\/button>/g)).toHaveLength(1);
    dispatch(new board.AddProject('old'));
    expect(posts()).toHaveLength(0);
    dispatch(new board.ToggleMigration());
    expect(html().match(/<input(?=[^>]*type="checkbox")(?=[^>]*checked)[^>]*>/g)).toHaveLength(2);
    expect(html()).toContain('never changes the original');
    expect(html()).toContain('must exit 0');
    dispatch(new board.MigrationProject('.', false));
    dispatch(new board.MigrationProject('web', false));
    expect(html()).toMatch(/disabled[^>]*>Start<\/button>/);
    dispatch(new board.MigrationAction('start'));
    expect(posts()).toHaveLength(0);
    dispatch(new board.MigrationProject('web', true));
    dispatch(new board.MigrationSmoke('  bun run smoke\r\n\n gleam test  \n'));
    dispatch(new board.MigrationAction('start'));
    dispatch(new board.MigrationAction('start'));
    await until(() => !model.migration_busy);
    expect(posts()).toHaveLength(1);
    expect(posts()[0]).toEqual({ path: '/migration/start', method: 'POST', body: { projects: ['web'], smoke: ['bun run smoke', 'gleam test'] } });
    expect(html()).toContain('E7020: A migration already exists');
    expect(model.migration_open).toBe(true);
    postReply = { state: 'running' };
    dispatch(new board.MigrationAction('start'));
    await until(() => !model.migration_busy);
    expect(html()).toContain('Waiting for the server');
    expect(html()).not.toContain('role="dialog"');
    // A GET started before the POST must not reload with its stale response.
    [model] = board.update(model, new board.MigrationLoaded(model.migration_generation - 1, JSON.stringify(migration)));
    expect(reloads).toBe(0);
    offline = true; await poll(); await poll();
    expect(reloads).toBe(0);
    expect(html()).toContain('Migration in progress');
    offline = false;
    migration = state('running', { step: 'Building the baseline', original, copy });
    await poll();
    expect(html()).toContain('Building the baseline');
    expect(reloads).toBe(0);
    migration = state('in_progress', { top: copy, original, copy, report, blockers: ['Reconstruction failed', 'Paths out of date'] });
    await poll();
    expect(reloads).toBe(1);
    expect(html()).toContain(copy);
    expect(html()).toContain(original);
    expect(html()).toContain('Migration report');
    for (const status of ['verified', 'failed', 'not checked', 'out of date']) expect(html()).toContain('>' + status + '</span>');
    expect(html()).toContain('Both projects copied');
    expect(html()).toContain(report.checked_at);
    expect(html()).toContain('<summary>Details</summary><pre');
    expect(html()).toContain('&lt;script&gt;new&lt;/script&gt;');
    expect(html()).toContain('Reconstruction failed');
    expect(html()).toMatch(/disabled[^>]*>Adopt<\/button>/);
    const blockedPosts = posts().length;
    dispatch(new board.MigrationAction('adopt'));
    expect(posts()).toHaveLength(blockedPosts);
    expect(confirmations).toHaveLength(0);
    migration = { ...migration, blockers: [], report: { ...report, current: true, checks: report.checks.filter(c => ['verified', 'not checked'].includes(c.status)) } };
    await poll();
    dispatch(new board.MigrationAction('adopt'));
    expect(posts()).toHaveLength(blockedPosts);
    expect(confirmations.at(-1)).toContain('Behaviour: No smoke commands');
    expect(confirmations.at(-1)).toContain(original + '.pre-solace');
    expect(confirmations.at(-1)).toContain('copy takes its path');
    accept = true;
    postReply = { code: 'E7022', message: 'Original changed:\nsrc/app.gleam\nnotes.txt' };
    dispatch(new board.MigrationAction('adopt'));
    await until(() => !model.migration_busy);
    expect(html()).toContain('E7022: Original changed:\nsrc/app.gleam\nnotes.txt');
    postReply = { state: 'running' };
    dispatch(new board.MigrationAction('recheck'));
    await until(() => !model.migration_busy);
    await poll(); // Recheck finishes without changing top.
    expect(reloads).toBe(2);
    dispatch(new board.MigrationAction('adopt'));
    await until(() => !model.migration_busy);
    migration = state('adopted', { original, undo: true });
    await poll();
    expect(reloads).toBe(3);
    expect(html()).toContain(original + '.pre-solace');
    expect(html()).toContain('Undo adoption');
    const adopted = model;
    dispatch(new board.DismissMigration());
    expect(html()).not.toContain('Undo adoption');
    model = adopted;
    accept = false;
    const beforeUndo = posts().length;
    dispatch(new board.MigrationAction('undo'));
    expect(posts()).toHaveLength(beforeUndo);
    expect(confirmations.at(-1)).toContain('Work done after adoption stays');
    accept = true;
    dispatch(new board.MigrationAction('undo'));
    await until(() => !model.migration_busy);
    migration = state('none'); await poll();
    expect(reloads).toBe(4);
    expect(html()).not.toContain('Migration in progress');
    migration = state('in_progress', { top: copy, original, copy }); await poll();
    expect(html()).toContain('No report yet');
    accept = false;
    const beforeAbandon = posts().length;
    dispatch(new board.MigrationAction('abandon'));
    expect(posts()).toHaveLength(beforeAbandon);
    expect(confirmations.at(-1)).toContain('deletes the copy at ' + copy);
    expect(confirmations.at(-1)).toContain('original at ' + original + ' stays untouched');
    accept = true;
    dispatch(new board.MigrationAction('abandon'));
    await until(() => !model.migration_busy);
    migration = state('running', { top: original, original, copy }); await poll();
    expect(reloads).toBe(5); // A changed top alone is sufficient to reload.
    migration = state('available', { error: 'E7022 Original changed: notes.txt' }); await poll();
    expect(html()).toContain('E7022 Original changed: notes.txt');
    expect(posts().map(r => r.path)).toContain('/migration/abandon');
    expect(posts().map(r => r.path)).toContain('/migration/undo');
    expect(requests.filter(r => r.path.includes('migration')).every(r => r.path.startsWith('/migration'))).toBe(true);
  } finally {
    for (const [key, descriptor] of Object.entries(saved)) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor); else delete globalThis[key];
    }
  }
});

test('migration transport ignores a project contract-view prefix', async () => {
  const oldLocation = Object.getOwnPropertyDescriptor(globalThis, 'location');
  const oldFetch = globalThis.fetch;
  const requests = [];
  try {
    Object.defineProperty(globalThis, 'location', { configurable: true, value: { pathname: '/p/web/view' } });
    globalThis.fetch = async path => { requests.push(path); return Response.json({}); };
    const { request } = await import('../src/api.mjs?migration-root-test');
    for (const [path, method] of [['/migration', 'GET'], ['/migration/start', 'POST'], ['/graph', 'GET']]) {
      await new Promise(resolve => request(path, method, '{}', resolve));
    }
    expect(requests).toEqual(['/migration', '/migration/start', '/p/web/graph']);
  } finally {
    globalThis.fetch = oldFetch;
    if (oldLocation) Object.defineProperty(globalThis, 'location', oldLocation); else delete globalThis.location;
  }
});

test('migration decodes load-time states and timestamps without a report', () => {
  for (const [value, visible] of [
    [state('none'), ''],
    [state('available'), 'Migrate this project'],
    [state('running', { step: 'Copying files' }), 'Copying files'],
    [state('in_progress', { original, copy }), 'No report yet'],
    [state('adopted', { original, undo: false }), ''],
  ]) {
    let [model] = board.init();
    [model] = board.update(model, new board.MigrationLoaded(0, JSON.stringify(value)));
    expect(model.migration[0].state).toBe(value.state);
    const html = to_string(board.view(model));
    if (visible) expect(html).toContain(visible);
    else expect(html).not.toContain('Migration in progress');
  }
  for (const checked_at of ['1728000000', 1728000000]) {
    let [model] = board.init();
    [model] = board.update(model, new board.MigrationLoaded(0, JSON.stringify(state('in_progress', {
      original, copy, report: { ...report, checked_at },
    }))));
    expect(to_string(board.view(model))).toContain('Checked: ' + new Date(1728000000000).toLocaleString());
  }
});
