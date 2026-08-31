---
name: omx-glm-duel
description: Use when a Paseo coding task needs independent CLI OMX and GLM implementations followed by Codex App review or synthesis.
---

# OMX GLM Duel

## Overview

Run Codex CLI with OMX and GLM in separate Paseo worktree workspaces. Use a third, ordinary Paseo Codex agent to inspect, select, improve, and verify the final result. Never treat OMX as a Paseo agent provider.

**REQUIRED BACKGROUND:** Use `paseo` for workspace, terminal, agent, profile, and provider operations.

## Contract

Treat `$ARGUMENTS` as the task. Derive explicit acceptance criteria and focused verification commands. Record `git rev-parse HEAD` as the immutable base. Exclude uncommitted caller changes; never copy, reset, merge, push, or deploy them.

## Workflow

1. Resolve Paseo CLI from `PATH`, falling back on Windows to `C:\Program Files\Paseo\resources\bin\paseo.cmd`. Require working `omx exec`, available `glm-acp-agent`, and available `codex`. Discover actual model and mode IDs; never guess them.
2. Create one unique slug and three worktree-isolated Paseo workspaces from the base:
   - `duel/omx-<slug>` for CLI OMX
   - `duel/glm-<slug>` for GLM
   - `duel/final-<slug>` for Codex App review
3. Write the same task contract into each candidate briefing: requirements, constraints, tests, no external production actions, one final commit, and a structured report.
4. In the OMX workspace, create a supervised Paseo terminal. Run `omx exec` non-interactively from that worktree with workspace-write sandboxing and an output-last-message artifact. This is the CLI lane. Do not launch it through `create_agent`, a Codex provider profile, or tmux-only `$team` unless the user explicitly requested and the runtime preflight passes.
5. In the GLM workspace, create a `glm-acp-agent` agent using the configured GLM implementation profile when present; otherwise materialize discovered `glm-5.2`, edit-approval mode, and maximum thinking. Require the same task contract, tests, commit, and report.
6. Start OMX and GLM concurrently. Trust GLM finish notification. For the terminal lane, use bounded terminal status/capture checks; never busy-poll. Verify both branches contain commits after the base and record fresh test evidence. A missing commit or required-test failure makes that candidate ineligible.
7. In the final workspace, launch an ordinary `codex` agent using the configured Codex reviewer profile when present; otherwise use discovered `gpt-5.6-sol`, full-write mode, and high reasoning. Give it the base, both branches, reports, acceptance criteria, and tests. Require it to:
   - inspect both diffs and histories;
   - score requirements 30%, tests 25%, correctness 20%, security 15%, maintainability 10%;
   - reject materially incomplete or failing candidates;
   - cherry-pick the stronger candidate, combine sound parts, or correct both;
   - run fresh tests and commit only on `duel/final-<slug>`.
8. Report scores, adopted and improved parts, final branch, commit, tests, and residual risks. Leave workspaces available for inspection. Archive only on explicit request.

## Stop Conditions

- Stop before dispatch when the Git base or acceptance criteria are unclear, `omx exec` is unhealthy, or a required provider/model/mode is unavailable.
- Stop without adoption when no candidate and reviewer can produce verified output.
- Never merge the final branch into the caller branch automatically.

## Example

`$omx-glm-duel 사용자 사전 용어를 번역 프롬프트에 안전하게 반영하고 테스트를 추가해줘`
