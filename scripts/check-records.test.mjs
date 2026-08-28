import assert from 'node:assert/strict';
import test from 'node:test';
import { requiresRecord } from './check-records.mjs';

test('source changes require a record file', () => {
  assert.equal(requiresRecord(['src/app/App.tsx']), true);
});

test('a source change accompanied by a project record passes', () => {
  assert.equal(requiresRecord(['src/app/App.tsx', 'PROJECT_LOG.txt']), false);
});

test('documentation-only changes do not require another record', () => {
  assert.equal(requiresRecord(['docs/superpowers/plans/a.md']), false);
});
