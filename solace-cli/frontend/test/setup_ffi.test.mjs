import { test, expect } from 'bun:test';
import { load_setup, recheck_setup } from '../src/setup_ffi.mjs';

const later = () => new Promise((resolve) => setTimeout(resolve, 0));

test('only the latest setup reply is delivered', async () => {
  const original = globalThis.fetch;
  const answers = [];
  globalThis.fetch = () => new Promise((resolve) => answers.push(resolve));
  const replies = [];
  try {
    load_setup((text) => replies.push(`first:${text}`));
    load_setup((text) => replies.push(`second:${text}`));
    answers[1]({ text: async () => 'new' });
    await later();
    answers[0]({ text: async () => 'old' });
    await later();
    expect(replies).toEqual(['second:new']);
  } finally {
    globalThis.fetch = original;
  }
});

test('the checklist is checked again without a click', () => {
  const original = globalThis.setInterval;
  let scheduled;
  globalThis.setInterval = (fn, ms) => {
    scheduled = { fn, ms };
    return 0;
  };
  let checks = 0;
  try {
    recheck_setup(() => { checks += 1; });
    expect(scheduled.ms).toBe(30000);
    scheduled.fn();
    expect(checks).toBe(1);
  } finally {
    globalThis.setInterval = original;
  }
});
