---
name: libreqos-review-subagents-workflow
description: Project workflow for invoking the local review sub-agents Thomas, Helen, Beck, and Jonas during LibreQoS coding sessions. Use when code changes need recent-diff review for Rust quality, accessibility, test coverage, scope control, and algorithmic sanity.
---

# LibreQoS Review Sub-Agents Workflow

Use this skill when code changes in this repo need the project-standard reviewer pass.

## Agents

- `thomas`: adversarial review of recent code changes for Rust best practices, safety, idiomatic style, and warning or Clippy suppression
- `beck`: review of recent code changes for meaningful unit-test coverage and weak or pointless tests
- `jonas`: review of recent code changes for scope drift, unrelated cleanup, and algorithmic or performance mistakes
- `helen`: review of recent user-interface changes for accessibility, ADA issues, missing alt text, missing labels, missing ARIA semantics, and related barriers

## Required Invocation Rules

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

1. After a substantial implementation batch, spawn `thomas`, `beck`, and `jonas` in parallel against the current recent changes.
2. If UI changed, spawn `helen` at the same checkpoint.
3. Use the current worktree diff as the default scope. If the worktree is clean but the session included recent commits, pass an explicit commit or diff range.
4. Review the findings, fix valid issues, or record why a concern is intentionally accepted.
5. Before final completion, rerun any reviewer whose scope changed since the prior pass.

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

Example prompt for `helen`:

```text
Review the recent user-interface changes for this session. Use the current worktree diff as the review scope unless I specify a narrower range. Focus on accessibility and other user barriers.
```

## Notes

- These agents are read-only review surfaces, not replacements for targeted tests, `cargo check`, `cargo clippy`, or visual verification.
- For tiny trivial edits, an intermediate review pass may be unnecessary, but the final required review still applies whenever code changed.
- In this repo, the subagent definitions live under `.codex/agents/*.toml`.
