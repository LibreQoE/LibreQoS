---
name: libreqos-git-workflow
description: Shared LibreQoS git workflow. Use when committing, organizing commit history, preparing larger PRs, or deciding how to split changes for review.
---

# LibreQoS Git Workflow

Use this skill when creating commits or preparing a PR in this repo.

## Commit Strategy

- For small changes, a single focused commit is fine.
- For larger PRs, prefer multiple focused commits that tell a clear narrative for the reviewer.
- Each commit should represent one understandable step: setup, mechanical move, behavior change, tests, docs, or cleanup.
- Write commit messages that explain why the step exists, not just which files changed.
- Avoid mixing unrelated fixes, formatting churn, generated artifacts, and behavior changes in the same commit.
- Keep commits reviewable on their own whenever practical, but do not add compatibility scaffolding only to make an intermediate commit independently shippable.

## Before Committing

- Inspect `git status`, `git diff`, and `git log --oneline -10`.
- Stage only intended files. Leave unrelated user or generated changes untouched.
- Run the targeted validation required by the changed area before the commit when practical.
- If validation was skipped or could not run, say that explicitly in the final user-facing summary.

## Larger PR Narrative

- Order commits so the reviewer can follow the work without jumping between concepts.
- Put broad mechanical changes before behavior changes when that reduces review noise.
- Put tests close to the behavior they validate unless the repo convention prefers a separate test commit.
- Use docs commits where they help operators or reviewers understand the visible behavior change.

## Notes

- Do not amend, squash, rebase, force-push, or rewrite history unless the user explicitly asks.
- Do not commit secrets, local runtime artifacts, screenshots, logs, or unrelated untracked files.
