import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { buildRecordEntry } from './record-change.mjs';

const repositoryRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const recordScript = path.join(repositoryRoot, 'scripts', 'record-change.mjs');

test('builds a stable record with sorted repository-relative paths', () => {
  const entry = buildRecordEntry({
    summary: '기록 자동화를 추가함',
    tests: 'pnpm records:test',
    files: [
      path.join(repositoryRoot, 'src', 'app', 'App.tsx'),
      path.join(repositoryRoot, 'package.json'),
    ],
    repositoryRoot,
    now: new Date('2026-08-28T03:45:00.000Z'),
  });

  assert.equal(
    entry,
    '\n[2026-08-28 12:45 Asia/Seoul] 변경 기록\n- 요약: 기록 자동화를 추가함\n- 변경 파일: package.json, src/app/App.tsx\n- 검증: pnpm records:test\n',
  );
  assert.equal(entry.endsWith('\n\n'), false);
});

test('rejects summaries containing bearer tokens', () => {
  assert.throws(
    () => buildRecordEntry({ summary: 'Bearer eyJhbGciOiJIUzI1NiJ9.payload.signature', tests: 'pnpm test', files: [] }),
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
      [recordScript, '--', '--summary', '자동 기록', '--tests', 'node --test', '--files-from-git'],
      { cwd: temporaryRepository },
    );

    assert.match(readFileSync(path.join(temporaryRepository, 'PROJECT_LOG.txt'), 'utf8'), /- 요약: 자동 기록/);
  } finally {
    rmSync(temporaryRepository, { force: true, recursive: true });
  }
});
