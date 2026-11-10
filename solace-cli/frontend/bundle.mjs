#!/usr/bin/env bun
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';

const frontend = import.meta.dir;
const root = resolve(frontend, '../..');
const parts = ['src', 'gleam.toml', 'package.json', 'bun.lock', 'manifest.toml'];

function run(command, args) {
  const result = spawnSync(command, args, { cwd: frontend, stdio: 'inherit' });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

export function sourceOwners(frontendRoot = frontend) {
  const owners = [frontendRoot];
  let manifest;
  try {
    manifest = Bun.TOML.parse(readFileSync(join(frontendRoot, 'gleam.toml'), 'utf8'));
  } catch {
    return owners;
  }
  const dependencies = manifest.dependencies;
  if (dependencies && typeof dependencies === 'object') {
    for (const dependency of Object.values(dependencies)) {
      if (typeof dependency?.path === 'string') {
        owners.push(resolve(frontendRoot, dependency.path));
      }
    }
  }
  return owners;
}

export function partHash(path) {
  const files = [];
  const walk = (absolute, name) => {
    let entries;
    try {
      entries = readdirSync(absolute, { withFileTypes: true });
    } catch {
      try {
        files.push([name, readFileSync(absolute)]);
      } catch {}
      return;
    }
    for (const entry of entries) {
      const child = join(absolute, entry.name);
      const childName = join(name, entry.name);
      if (entry.isSymbolicLink()) {
        try {
          files.push([childName, readFileSync(child)]);
        } catch {}
      } else if (entry.isDirectory()) {
        walk(child, childName);
      } else {
        try {
          files.push([childName, readFileSync(child)]);
        } catch {}
      }
    }
  };
  walk(path, '');
  files.sort(([a, ab], [b, bb]) => Buffer.compare(Buffer.from(a), Buffer.from(b)) || Buffer.compare(ab, bb));
  const chunks = [];
  for (const [name, bytes] of files) {
    chunks.push(Buffer.from(name), Buffer.from([0]), bytes, Buffer.from([0]));
  }
  return sha256(Buffer.concat(chunks));
}

if (import.meta.main) {
  const sources = {};
  for (const owner of sourceOwners()) {
    for (const part of parts) {
      const path = join(owner, part);
      sources[relative(root, path)] = partHash(path);
    }
  }

  run('bun', ['install', '--frozen-lockfile']);
  run('gleam', ['build']);
  run('bun', ['build', 'src/main.mjs', '--outfile', 'priv/app.js', '--minify', '--target', 'browser']);

  const output = 'priv/app.js';
  const fingerprint = { output: sha256(readFileSync(join(frontend, output))), sources };
  writeFileSync(join(frontend, `${output}.fingerprint`), `${JSON.stringify(fingerprint, null, 2)}\n`);
}
