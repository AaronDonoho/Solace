import { test, expect } from 'bun:test';
import { perform } from '../build/dev/javascript/lustre/lustre/effect.mjs';
import { to_string } from '../build/dev/javascript/lustre/lustre/element.mjs';
import * as delivery from '../build/dev/javascript/delivery/delivery.mjs';
import { desktop, open_folder } from '../src/api.mjs';

const noop = () => {};

test('a build starts, cancels, opens a step log and follows live updates', async () => {
  const requests = [];
  let push = () => {};
  let build = {
    id: 'b1', state: 'running', startedAt: 1, endedAt: null,
    steps: [{ project: 'web', name: 'build', command: ['gleam', 'build'], timeout: 60,
      state: 'waiting', exitCode: null, duration: 0 }],
  };
  const buildJson = () => JSON.parse(JSON.stringify(build));
  const server = Bun.serve({ port: 0, fetch(req) {
    const url = new URL(req.url);
    requests.push([req.method, url.pathname]);
    if (url.pathname === '/build/plan') {
      return Response.json([{ project: 'web', source: 'declared', after: [], outputs: [],
        stale: null, steps: [{ name: 'build', run: ['gleam', 'build'], timeout: 60 }] }]);
    }
    if (url.pathname === '/builds' && req.method === 'POST') return Response.json(buildJson());
    if (url.pathname === '/builds/b1/cancel') {
      build = { ...build, state: 'canceled', endedAt: 2 };
      return Response.json(buildJson());
    }
    if (url.pathname === '/builds/b1/steps/0/log') return new Response('compiling web\n');
    return Response.json({ code: 'E7023', message: 'not found' });
  }});
  const fetchOriginal = globalThis.fetch;
  globalThis.fetch = (path, options) => fetchOriginal(new URL(path, server.url), options);
  globalThis.EventSource = class {
    listeners = {};
    addEventListener(name, f) { this.listeners[name] = f; }
    constructor() {
      push = data => this.onmessage?.({ data: JSON.stringify(data) });
      setTimeout(() => this.listeners.ready?.({ data: '{}' }), 0);
    }
    close() {}
  };
  let model = delivery.initial();
  const emitted = [];
  const dispatch = msg => {
    let effect;
    [model, effect] = delivery.update(model, msg);
    perform(effect, dispatch, name => emitted.push(name), noop, noop, noop, noop, noop);
  };
  const page = () => to_string(delivery.view(model));
  const until = async predicate => {
    for (let i = 0; i < 300; i++) {
      if (predicate()) return;
      await Bun.sleep(10);
    }
    throw new Error('UI did not reach expected state: ' + page());
  };
  try {
    dispatch(new delivery.Connected());
    await until(() => model.caught_up && page().includes('Waiting'));
    expect(requests).toContainEqual(['GET', '/build/plan']);

    dispatch(new delivery.StartBuild());
    await until(() => page().includes('Last build: Running'));
    expect(requests).toContainEqual(['POST', '/builds']);

    dispatch(new delivery.CancelBuild());
    await until(() => page().includes('Last build: Canceled'));
    expect(requests).toContainEqual(['POST', '/builds/b1/cancel']);

    build = { ...build, state: 'running', endedAt: null };
    push({ build: buildJson() });
    await until(() => page().includes('Last build: Running'));

    dispatch(new delivery.OpenStep(0));
    await until(() => page().includes('compiling web'));
    expect(requests).toContainEqual(['GET', '/builds/b1/steps/0/log']);

    push({ buildLog: { id: 'b1', step: 0, text: 'done\n' } });
    await until(() => page().includes('done'));

    build = { ...build, state: 'passed', endedAt: 3,
      steps: [{ ...build.steps[0], state: 'passed', duration: 1200 }] };
    push({ build: buildJson() });
    await until(() => page().includes('Last build: Passed'));
    expect(emitted).toContain('attention');
  } finally {
    server.stop(true);
    globalThis.fetch = fetchOriginal;
    delete globalThis.EventSource;
  }
});

test('opening a folder asks the desktop app only when it is there', async () => {
  const saved = globalThis.window;
  try {
    delete globalThis.window;
    expect(desktop()).toBe(false);
    let reply = 'not called';
    open_folder('b1', text => { reply = text; });
    expect(reply).toBe('');

    const calls = [];
    globalThis.window = { __TAURI__: { core: { invoke: async (name, args) => {
      calls.push([name, args]);
    } } } };
    expect(desktop()).toBe(true);
    reply = 'pending';
    open_folder('b1', text => { reply = text; });
    await Bun.sleep(0);
    expect(calls).toEqual([['open_release_folder', { version: 'b1' }]]);
    expect(reply).toBe('');
  } finally {
    if (saved === undefined) delete globalThis.window;
    else globalThis.window = saved;
  }
});
