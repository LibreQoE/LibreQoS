---
name: libreqos-anti-slop
description: Anti-slop workflow for LibreQoS. Use when changing code, docs, UI, or user-facing text. Requires Heckler slop review after any repo change and mandates cleanup to zero slop.
---

# LibreQoS Anti-Slop Workflow

Use this skill when writing or reviewing code, docs, UI, or any user-facing text in this repo.

The goal is **zero slop**.

## Required Workflow

1. Make your change.
2. Before returning to the user (whenever the repo has changed), invoke the `heckler` subagent on the current diff.
3. If `heckler` reports a non-zero slop score, you **MUST**:
   - print exactly: `SLOP! SLOP! SLOP ALERT!`
   - fix the slop (delete dead code, remove placeholders, rename generic identifiers, remove filler text, simplify algorithms, etc.)
   - re-run `heckler` until the slop score is **0/10**, or explicitly explain why a residual item cannot be fixed in this scope.

This repo also uses `reaper` to enforce DRY/duplication and dead-code avoidance for source-code batches; see `$libreqos-review-subagents-workflow`.

## Code Slop (Rust-first)

### High-confidence slop in Rust

- `todo!()`, `unimplemented!()`, commented-out code, placeholder stubs
- unused code or `#[allow(dead_code)]` without a strong reason
- duplicated helpers or copy/paste logic that should reuse an existing helper/module
- generic names that hide meaning (`data`, `result`, `temp`, `value`, `thing`, `info`)
- \"design pattern\" scaffolding that adds layers without need
- obvious lazy algorithms (quadratic loops, repeated allocation/parsing, avoidable scans)
- magic numbers without an explanatory constant

### Rust cleanup actions

- Delete dead code and unused imports; do not silence warnings to \"make it compile\".
- Prefer a small shared helper over copy/paste variants.
- Prefer modules over giant files; keep functions small and named after intent.
- Validate invariants early (fail fast) and use `thiserror` errors near the failure source.

### Python slop patterns (when touching `src/`)

- catching broad exceptions (especially `except Exception:`) without a specific recovery path
- swallowing errors or logging-only handling that hides real failures
- placeholder \"TODO\" blocks with no owner/action
- copy/paste helper functions that should share common logic

## Text Slop

Avoid AI-ish filler and meta-commentary in docs, comments, and user-facing strings:

- No \"delve into\", \"dive deep\", \"navigate the complexities\", \"it's important to note that\", \"in today's fast-paced world\", etc.
- No stage directions (\"In this section we will...\", \"Let's take a closer look...\").
- Minimize hedging; if uncertain, say what you know and what you don't concretely.
- Prefer active voice and direct statements.

## UI / Design Slop

LibreQoS UI is operator-facing. Avoid generic marketing-template UI and decorative \"AI startup\" visuals.

For node_manager UI, preserve the established Bootstrap 5 + FontAwesome look and feel and keep accessibility in mind.

## Notes

- Required workflow phrases or project quirks may look odd in isolation (e.g. the mandated clippy-suppression apology line). Treat these as intentional and do not \"clean them up\".
- This skill is enforced by the `heckler` subagent defined in `.codex/agents/heckler.toml`.
