import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { requiresRecord } from './check-records.mjs';

const repositoryRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const checkerScript = path.join(repositoryRoot, 'scripts', 'check-records.mjs');

function git(repository, args) {
  return execFileSync('git', args, { cwd: repository, encoding: 'utf8' }).trim();
}

function createRepository() {
  const repository = mkdtempSync(path.join(os.tmpdir(), 'smartcat-check-'));
  git(repository, ['init']);
  git(repository, ['config', 'user.email', 'test@example.com']);
  git(repository, ['config', 'user.name', 'Record Test']);
  writeFileSync(path.join(repository, 'PROJECT_LOG.txt'), '기존 기록\n', 'utf8');
  git(repository, ['add', 'PROJECT_LOG.txt']);
  git(repository, ['commit', '-m', 'initial record']);
  return repository;
}

function runChecker(repository, args = []) {
  return execFileSync(process.execPath, [checkerScript, ...args], { cwd: repository, stdio: 'pipe' });
}

test('source changes require a record file', () => {
  assert.equal(requiresRecord(['src/app/App.tsx']), true);
});

test('a source change accompanied by a project record passes', () => {
  assert.equal(requiresRecord(['src/app/App.tsx', 'PROJECT_LOG.txt']), false);
});

test('documentation-only changes do not require another record', () => {
  assert.equal(requiresRecord(['docs/superpowers/plans/a.md']), false);
});

test('staged product changes fail the command-level enforcement check without a record', () => {
  const repository = createRepository();
  try {
    mkdirSync(path.join(repository, 'src'));
    writeFileSync(path.join(repository, 'src', 'translation.ts'), 'export const enabled = true;\n', 'utf8');
    git(repository, ['add', 'src/translation.ts']);

    assert.throws(() => runChecker(repository), (error) => error.status === 1);
  } finally {
    rmSync(repository, { force: true, recursive: true });
  }
});

test('staged product changes pass the command-level check with a record', () => {
  const repository = createRepository();
  try {
    mkdirSync(path.join(repository, 'src'));
    writeFileSync(path.join(repository, 'src', 'translation.ts'), 'export const enabled = true;\n', 'utf8');
    writeFileSync(path.join(repository, 'PROJECT_LOG.txt'), '변경 기록\n', 'utf8');
    git(repository, ['add', 'src/translation.ts', 'PROJECT_LOG.txt']);

    assert.doesNotThrow(() => runChecker(repository));
  } finally {
    rmSync(repository, { force: true, recursive: true });
  }
});

test('a product-only commit range fails CI enforcement', () => {
  const repository = createRepository();
  try {
    const base = git(repository, ['rev-parse', 'HEAD']);
    mkdirSync(path.join(repository, 'src'));
    writeFileSync(path.join(repository, 'src', 'translation.ts'), 'export const enabled = true;\n', 'utf8');
    git(repository, ['add', 'src/translation.ts']);
    git(repository, ['commit', '-m', 'product change']);
    const head = git(repository, ['rev-parse', 'HEAD']);

    assert.throws(() => runChecker(repository, ['--base', base, '--head', head]), (error) => error.status === 1);
  } finally {
    rmSync(repository, { force: true, recursive: true });
  }
});

test('a product and record commit range passes CI enforcement', () => {
  const repository = createRepository();
  try {
    const base = git(repository, ['rev-parse', 'HEAD']);
    mkdirSync(path.join(repository, 'src'));
    writeFileSync(path.join(repository, 'src', 'translation.ts'), 'export const enabled = true;\n', 'utf8');
    writeFileSync(path.join(repository, 'PROJECT_LOG.txt'), '변경 기록\n', 'utf8');
    git(repository, ['add', 'src/translation.ts', 'PROJECT_LOG.txt']);
    git(repository, ['commit', '-m', 'product change with record']);
    const head = git(repository, ['rev-parse', 'HEAD']);

    assert.doesNotThrow(() => runChecker(repository, ['--', '--base', base, '--head', head]));
  } finally {
    rmSync(repository, { force: true, recursive: true });
  }
});

test('an initial-push range falls back to the empty tree', () => {
  const repository = mkdtempSync(path.join(os.tmpdir(), 'smartcat-check-initial-'));
  try {
    git(repository, ['init']);
    git(repository, ['config', 'user.email', 'test@example.com']);
    git(repository, ['config', 'user.name', 'Record Test']);
    mkdirSync(path.join(repository, 'src'));
    writeFileSync(path.join(repository, 'src', 'translation.ts'), 'export const enabled = true;\n', 'utf8');
    git(repository, ['add', 'src/translation.ts']);
    git(repository, ['commit', '-m', 'initial product change']);
    const head = git(repository, ['rev-parse', 'HEAD']);

    assert.throws(
      () => runChecker(repository, ['--base', '0'.repeat(40), '--head', head]),
      (error) => error.status === 1,
    );
  } finally {
    rmSync(repository, { force: true, recursive: true });
  }
});
