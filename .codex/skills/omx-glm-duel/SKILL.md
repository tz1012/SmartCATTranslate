---
name: omx-glm-duel
description: Use when a Paseo coding task needs independent CLI OMX and GLM implementations followed by supervised CLI OMX finalization.
---

# OMX GLM Duel

## Overview

Run independent OMX and GLM candidates from one immutable Git base in isolated Paseo worktrees. Paseo and app-side Codex are orchestration and read-only review surfaces; they are not implementation fallbacks.

## CLI-only production contract

When the user requires Codex coding through CLI OMX, all production edits, verification, and commits—including final synthesis—must occur in supervised `omx exec` terminal lanes. App/Paseo Codex may only orchestrate and perform read-only review. It must not edit product files, run the product verification suite, or create an implementation commit.

If a required CLI lane is failed, unavailable, unhealthy, or lacks write authority, stop with a blocker. Never fall back to app-side coding or silently move final synthesis into a Paseo agent.

## Workflow

1. Record `git rev-parse HEAD` as the immutable base and derive explicit acceptance criteria and verification commands. Exclude uncommitted caller changes; never copy, reset, merge, push, or deploy them.
2. Discover actual Paseo providers, models, modes, and the working `omx exec` command. Never guess identifiers.
3. Create isolated OMX candidate, GLM candidate, and final worktrees from the recorded base.
4. Run the OMX implementation in a supervised Paseo terminal using non-interactive `omx exec`, workspace-write sandboxing, a bounded timeout, and an output-last-message artifact. Do not launch OMX implementation or finalization through `create_agent` or an app-side Codex profile.
5. Run the GLM candidate independently with the same task contract, test requirements, and commit requirement.
6. Collect bounded status and terminal-capture evidence. Do not busy-poll or wait indefinitely.
7. Run selection, corrections, fresh verification, and the consolidated implementation commit in a new supervised `omx exec` finalization terminal lane. App/Paseo Codex may provide read-only candidate scores and orchestration inputs only.
8. Report candidate scores, adopted parts, final branch and commit, exact test evidence, and residual risks. Leave worktrees for inspection unless cleanup is explicitly requested.

## Bounded stale-run rule

A claimed build or verification run with no matching operating-system child process and no new terminal activity during the bounded observation window is failed, not running. Record the last observed command and evidence, stop that lane, and surface an error. If an interrupt request fails or cannot be confirmed, surface an error; never leave the lane classified as `running`.

## Stop conditions

- Stop before dispatch if the Git base or acceptance criteria are unclear, required providers are unavailable, or `omx exec` is unhealthy.
- Stop with a blocker if any required CLI implementation/finalization lane cannot be supervised or cannot write, verify, and commit in its assigned worktree.
- Stop without adoption when no eligible candidate can be corrected and verified in the CLI finalization lane.
- Never merge or push the final branch automatically.
