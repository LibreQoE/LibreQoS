---
name: libreqos-review-subagents-workflow
description: Project workflow for invoking the local review sub-agents Thomas, Helen, Beck, Jonas, The Reaper, and Heckler during LibreQoS coding sessions. Use when code changes need recent-diff review for DRY/duplication, slop detection, Rust quality, accessibility, test coverage, scope control, and algorithmic sanity.
---

# LibreQoS Review Sub-Agents Workflow

Use this skill when code changes in this repo need the project-standard reviewer pass.

## Agents

- `thomas`: adversarial review of recent code changes for Rust best practices, safety, idiomatic style, and warning or Clippy suppression
- `beck`: review of recent code changes for meaningful unit-test coverage and weak or pointless tests
- `jonas`: review of recent code changes for scope drift, unrelated cleanup, and algorithmic or performance mistakes
- `helen`: review of recent user-interface changes for accessibility, ADA issues, missing alt text, missing labels, missing ARIA semantics, and related barriers
- `reaper`: review of recent changes for duplicated functionality (DRY violations), dead code, placeholders, and lazy algorithm choices
- `heckler`: hyper-skeptical slop detector for AI slop, dead code, placeholders, and lazy algorithm choices; returns a slop score (goal: 0/10)

## Required Invocation Rules

- After any changes to the repo, invoke `heckler` before returning to the user.
- If `heckler` reports a non-zero slop score, print `SLOP! SLOP! SLOP ALERT!`, fix the slop, and re-run until the score is `0/10` (see `$libreqos-anti-slop`).
- After each batch of source-code edits (Rust/Python/JS/etc.), invoke `reaper` before returning to the user.
- During large sessions, invoke `thomas`, `beck`, and `jonas` together after substantial implementation batches.
- Always invoke `thomas`, `beck`, and `jonas` before the final user-facing completion message when code changed in the session.
- After any user-interface change, invoke `helen`.
- If the session includes both UI and code changes, invoke `helen` alongside the others at the same review checkpoint.

## What Counts As A Large Session

Treat a session as large when any of these are true:

- the diff is more than a tiny one-file tweak
- multiple files or crates changed
- you finished a meaningful implementation batch and are about to continue layering on more work
- the blast radius is not obvious from a quick glance

## Invocation Pattern

1. After any change to the repo, spawn `heckler` against the current recent changes.
2. After each source-code implementation batch, also spawn `reaper` against the same scope.
3. After a substantial implementation batch in a large session, spawn `thomas`, `beck`, and `jonas` in parallel against the current recent changes.
4. If UI changed, spawn `helen` at the same checkpoint.
5. Use the current worktree diff as the default scope. If the worktree is clean but the session included recent commits, pass an explicit commit or diff range.
6. Review the findings, fix valid issues, or record why a concern is intentionally accepted.
7. Before final completion, rerun any reviewer whose scope changed since the prior pass.

## Required Prompt Shape

When spawning a reviewer, include:

- the review scope:
  - current worktree diff, or
  - explicit commit or diff range
- the intent of the change
- any area-specific context that helps the reviewer load the right project skills

Example prompt for `thomas`, `beck`, and `jonas`:

```text
Review the recent changes for this session. Use the current worktree diff as the review scope unless I specify a narrower range. The goal of the change is: <brief intent>.
```

Example prompt for `reaper`:

```text
Review the recent changes for duplicated functionality (DRY violations), dead code, placeholders, and lazy algorithm choices. Use the current worktree diff as the review scope unless I specify a narrower range. The goal of the change is: <brief intent>.
```

Example prompt for `heckler`:

```text
Review the recent changes for AI slop, dead code, placeholders, and lazy algorithm choices. Return a slop score (goal: 0/10). Use the current worktree diff as the review scope unless I specify a narrower range. The goal of the change is: <brief intent>.
```

Example prompt for `helen`:

```text
Review the recent user-interface changes for this session. Use the current worktree diff as the review scope unless I specify a narrower range. Focus on accessibility and other user barriers.
```

## Notes

- These agents are read-only review surfaces, not replacements for targeted tests, `cargo check`, `cargo clippy`, or visual verification.
- For tiny trivial edits, an intermediate review pass may be unnecessary, but the final required review still applies whenever code changed.
- In this repo, the subagent definitions live under `.codex/agents/*.toml` (including `.codex/agents/reaper.toml` and `.codex/agents/heckler.toml`).
- `heckler` is the enforcement surface for `$libreqos-anti-slop`.
