import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { buildRecordEntry } from './record-change.mjs';

const repositoryRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const recordScript = path.join(repositoryRoot, 'scripts', 'record-change.mjs');

test('builds a stable record with sorted repository-relative paths', () => {
  const entry = buildRecordEntry({
    summary: 'chore: records-automation',
    tests: 'pnpm records:test && 8/8 passed',
    files: [
      path.join(repositoryRoot, 'src', 'app', 'App.tsx'),
      path.join(repositoryRoot, 'package.json'),
    ],
    repositoryRoot,
    now: new Date('2026-08-28T03:45:00.000Z'),
  });

  assert.equal(
    entry,
    '\n[2026-08-28 12:45 Asia/Seoul] 변경 기록\n- 요약: chore: records-automation\n- 변경 파일: package.json, src/app/App.tsx\n- 검증: pnpm records:test && 8/8 passed\n',
  );
  assert.equal(entry.endsWith('\n\n'), false);
});

test('rejects summaries containing bearer tokens', () => {
  assert.throws(
    () => buildRecordEntry({ summary: 'Bearer eyJhbGciOiJIUzI1NiJ9.payload.signature', tests: 'pnpm test', files: [] }),
    /secret/i,
  );
});

test('rejects arbitrary prose, line breaks, and oversized values in record fields', () => {
  assert.throws(
    () => buildRecordEntry({ summary: 'translated source sentence', tests: 'pnpm test', files: [] }),
    /summary/i,
  );
  assert.throws(
    () => buildRecordEntry({ summary: 'chore: records', tests: 'translation prose is not diagnostic', files: [] }),
    /tests/i,
  );
  assert.throws(
    () => buildRecordEntry({ summary: 'chore: records\nsource text', tests: 'pnpm test', files: [] }),
    /control/i,
  );
  assert.throws(
    () => buildRecordEntry({ summary: 'chore: records', tests: 'pnpm test\n8/8 passed', files: [] }),
    /control/i,
  );
  assert.throws(
    () => buildRecordEntry({ summary: 'chore: records', tests: `pnpm test ${'x'.repeat(200)}`, files: [] }),
    /too long/i,
  );
});

test('rejects bearer credentials and absolute user paths in test diagnostics', () => {
  assert.throws(
    () => buildRecordEntry({ summary: 'chore: records', tests: 'pnpm test && Bearer abc.def.ghi', files: [] }),
    /secret/i,
  );
  assert.throws(
    () => buildRecordEntry({ summary: 'chore: records', tests: 'pnpm test --path /Users/name/file', files: [] }),
    /absolute path/i,
  );
  assert.throws(
    () => buildRecordEntry({ summary: 'chore: records', tests: 'pnpm test --api-key abc', files: [] }),
    /secret/i,
  );
});

test('rejects summaries containing absolute user paths', () => {
  assert.throws(
    () => buildRecordEntry({ summary: 'C:\\Users\\name\\secret.txt', tests: 'pnpm test', files: [] }),
    /absolute path/i,
  );
});

test('exits nonzero when required command arguments are absent', () => {
  assert.throws(
    () => execFileSync(process.execPath, [recordScript], { stdio: 'pipe' }),
    (error) => error.status !== 0,
  );
});

test('accepts the pnpm argument separator and appends a record', () => {
  const temporaryRepository = mkdtempSync(path.join(os.tmpdir(), 'smartcat-record-'));
  try {
    writeFileSync(path.join(temporaryRepository, 'PROJECT_LOG.txt'), '기존 기록\n', 'utf8');
    execFileSync('git', ['init'], { cwd: temporaryRepository });
    execFileSync('git', ['config', 'user.email', 'test@example.com'], { cwd: temporaryRepository });
    execFileSync('git', ['config', 'user.name', 'Record Test'], { cwd: temporaryRepository });
    execFileSync('git', ['add', 'PROJECT_LOG.txt'], { cwd: temporaryRepository });
    execFileSync('git', ['commit', '-m', 'initial'], { cwd: temporaryRepository });

    execFileSync(
      process.execPath,
      [recordScript, '--', '--summary', 'chore: automatic-record', '--tests', 'node --test', '--files-from-git'],
      { cwd: temporaryRepository },
    );

    assert.match(readFileSync(path.join(temporaryRepository, 'PROJECT_LOG.txt'), 'utf8'), /- 요약: chore: automatic-record/);
  } finally {
    rmSync(temporaryRepository, { force: true, recursive: true });
  }
});

test('records tracked and untracked product files while excluding sources and sensitive paths', () => {
  const temporaryRepository = mkdtempSync(path.join(os.tmpdir(), 'smartcat-record-files-'));
  try {
    writeFileSync(path.join(temporaryRepository, 'PROJECT_LOG.txt'), '기존 기록\n', 'utf8');
    mkdirSync(path.join(temporaryRepository, 'src'));
    writeFileSync(path.join(temporaryRepository, 'src', 'tracked.ts'), 'export const value = 1;\n', 'utf8');
    execFileSync('git', ['init'], { cwd: temporaryRepository });
    execFileSync('git', ['config', 'user.email', 'test@example.com'], { cwd: temporaryRepository });
    execFileSync('git', ['config', 'user.name', 'Record Test'], { cwd: temporaryRepository });
    execFileSync('git', ['add', 'PROJECT_LOG.txt', 'src/tracked.ts'], { cwd: temporaryRepository });
    execFileSync('git', ['commit', '-m', 'initial'], { cwd: temporaryRepository });

    writeFileSync(path.join(temporaryRepository, 'src', 'tracked.ts'), 'export const value = 2;\n', 'utf8');
    writeFileSync(path.join(temporaryRepository, 'src', 'untracked.ts'), 'export const value = 3;\n', 'utf8');
    mkdirSync(path.join(temporaryRepository, 'sources'));
    writeFileSync(path.join(temporaryRepository, 'sources', 'reference.txt'), 'source material', 'utf8');
    writeFileSync(path.join(temporaryRepository, 'src', 'api-token.txt'), 'sensitive name', 'utf8');

    execFileSync(
      process.execPath,
      [recordScript, '--summary', 'chore: file-collection', '--tests', 'pnpm records:test && 8/8 passed', '--files-from-git'],
      { cwd: temporaryRepository },
    );

    const record = readFileSync(path.join(temporaryRepository, 'PROJECT_LOG.txt'), 'utf8');
    assert.match(record, /src\/tracked\.ts/);
    assert.match(record, /src\/untracked\.ts/);
    assert.doesNotMatch(record, /sources\/reference\.txt/);
    assert.doesNotMatch(record, /src\/api-token\.txt/);
  } finally {
    rmSync(temporaryRepository, { force: true, recursive: true });
  }
});
