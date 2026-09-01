import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const skillPath = new URL('../.codex/skills/omx-glm-duel/SKILL.md', import.meta.url);

test('OMX duel keeps implementation and final synthesis inside supervised CLI lanes', async () => {
  const skill = await readFile(skillPath, 'utf8');

  for (const invariant of [
    'all production edits, verification, and commits—including final synthesis—must occur in supervised `omx exec` terminal lanes',
    'App/Paseo Codex may only orchestrate and perform read-only review',
    'Never fall back to app-side coding',
    'Run selection, corrections, fresh verification, and the consolidated implementation commit in a new supervised `omx exec` finalization terminal lane',
  ]) {
    assert.ok(skill.includes(invariant), `missing CLI-only invariant: ${invariant}`);
  }
});

test('OMX duel classifies inactive builds and failed interrupts as errors', async () => {
  const skill = await readFile(skillPath, 'utf8');

  assert.match(skill, /no matching operating-system child process[\s\S]*is failed, not running/);
  assert.match(skill, /interrupt request fails[\s\S]*surface an error/);
});
