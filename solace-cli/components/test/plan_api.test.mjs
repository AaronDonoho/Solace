import { test, expect } from 'bun:test';
import { select_conversation } from '../build/dev/javascript/components/plan_api.mjs';

// A keydown in the conversation, and what the selection was given.
function press(key, keys = {}) {
  const body = { name: 'conversation' };
  const selected = [];
  const saved = globalThis.getSelection;
  globalThis.getSelection = () => ({ selectAllChildren: node => selected.push(node) });
  let prevented = false;
  try {
    select_conversation({
      key, ...keys,
      target: { closest: selector => (selector === '.plan-body' ? body : null) },
      preventDefault: () => { prevented = true; },
    });
  } finally {
    globalThis.getSelection = saved;
  }
  return { prevented, selected, body };
}

test('Ctrl or Cmd+A selects only the conversation', () => {
  for (const keys of [{ ctrlKey: true }, { metaKey: true }]) {
    const { prevented, selected, body } = press('a', keys);
    expect(prevented).toBe(true);
    expect(selected).toEqual([body]);
  }
  expect(press('A', { ctrlKey: true }).prevented).toBe(true);
});

test('other keys keep their default', () => {
  for (const [key, keys] of [
    ['a', {}],
    ['c', { ctrlKey: true }],
    ['a', { ctrlKey: true, shiftKey: true }],
    ['a', { ctrlKey: true, altKey: true }],
    ['a', { metaKey: true, altKey: true }],
  ]) {
    const { prevented, selected } = press(key, keys);
    expect(prevented).toBe(false);
    expect(selected).toEqual([]);
  }
});

test('Ctrl+A outside a conversation keeps its default', () => {
  let prevented = false;
  select_conversation({
    key: 'a', ctrlKey: true,
    target: { closest: () => null },
    preventDefault: () => { prevented = true; },
  });
  expect(prevented).toBe(false);
});
