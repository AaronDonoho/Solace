import { test, expect } from 'bun:test';
import { cpSync, mkdtempSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { partHash, sourceOwners } from '../bundle.mjs';

const fixture = join(import.meta.dir, 'fixtures/part');
const expectedPartHash = 'ae8589fc75d5d3f357cd2e90b5077723d43c84a1012965f0ab63408f978bee1b';

function copiedFixture() {
  const dir = mkdtempSync(join(tmpdir(), 'solace-part-'));
  cpSync(fixture, dir, { recursive: true });
  symlinkSync('file.txt', join(dir, 'link.txt'));
  return dir;
}

test('the shared fixture hashes the same in the bundle and the build plan', () => {
  const dir = copiedFixture();
  try {
    expect(partHash(dir)).toBe(expectedPartHash);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('table-style dependencies are bundle sources', () => {
  expect(sourceOwners(fixture).sort()).toEqual(
    [fixture, resolve(fixture, 'inline'), resolve(fixture, 'table')].sort(),
  );
});
