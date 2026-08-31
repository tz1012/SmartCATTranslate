---
name: omx-glm-duel
description: Use when a Paseo task benefits from independent OMX and GLM implementations with Codex selection or synthesis.
---

# OMX GLM Duel

## Overview

Run two candidates from one Git commit in isolated Paseo worktrees. Have Codex judge, adopt, test, and improve the result on a separate final branch. Never merge, push, deploy, or modify the caller's branch automatically.

**REQUIRED BACKGROUND:** Use `paseo` for provider discovery, agent creation, notifications, and worktree lifecycle.

## Input Contract

Treat `$ARGUMENTS` as the task. Derive explicit acceptance criteria and relevant verification commands. Record `git rev-parse HEAD` as the base. Report that uncommitted caller changes are excluded; never copy or reset them.

## Workflow

1. Resolve Paseo CLI from `PATH`, falling back on Windows to `C:\Program Files\Paseo\resources\bin\paseo.cmd`.
2. Run `provider ls --json`, `provider models <provider> --json`, and `provider diagnostic <provider> --json`. Require available `codex` and `glm-acp-agent` providers. Prefer `codex/gpt-5.6-sol` and `glm-acp-agent/glm-5.2`; select only model and mode IDs actually returned.
3. Generate one collision-resistant slug and three branches from the recorded base:
   - `duel/omx-<slug>`
   - `duel/glm-<slug>`
   - `duel/final-<slug>`
4. Launch both candidates with `--background --new-workspace worktree --worktree-mode branch-off --new-branch <candidate-branch> --base <sha>`:
   - OMX lane: Codex provider with the discovered full-write mode. Require it to follow repository `AGENTS.md`, use applicable OMX skills/native role routing, implement only the task, test, and commit. Do not claim tmux-only OMX modes when unavailable.
   - GLM lane: `glm-acp-agent` with the discovered edit-approval mode. Give identical acceptance criteria, implementation scope, tests, and commit requirement.
5. Do not poll. Trust Paseo finish notifications; if callbacks are unavailable, use one bounded `paseo wait` per agent. Retrieve each final report and verify each candidate branch contains a commit after the base. A missing commit or failing required test is a failed candidate, not an automatic winner.
6. Launch the Codex judge with `--background --new-workspace worktree --worktree-mode branch-off --new-branch duel/final-<slug> --base <sha>`. Give it both branches, reports, acceptance criteria, and tests. Require it to:
   - inspect `git diff <base>..<candidate>` and commit history;
   - score requirements 30%, verification 25%, correctness 20%, security/policy 15%, maintainability/scope 10%;
   - reject a candidate with required-test failure or material requirement gaps;
   - cherry-pick the stronger candidate, combine sound parts, or implement a corrected solution if neither passes;
   - run fresh tests and commit the final result;
   - avoid merging, pushing, deploying, or editing either candidate branch.
7. Verify the final branch has a commit after the base and fresh passing evidence. Report candidate scores, selected/synthesized elements, final branch, final commit, tests, and residual risks.

## Stop Conditions

- Stop before dispatch if the repository has no Git base, acceptance criteria are materially ambiguous, or either provider/model/mode is unavailable.
- Stop without adoption if both candidates and the judge cannot produce a verified result.
- Leave all worktrees and branches available for inspection. Archive them only when the user explicitly requests cleanup.

## Example

`$omx-glm-duel 사용자 사전에서 용어를 가져와 번역 프롬프트에 안전하게 반영하고 관련 테스트를 추가해줘`

The deliverable is a verified `duel/final-*` branch, not a mutation of the user's current branch.
