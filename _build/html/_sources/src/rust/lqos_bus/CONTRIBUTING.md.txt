# Contributing to `lqos_bus`

Thanks for your interest in improving the LibreQoS bus crate.

## License and Exceptions

- Contributions are accepted under: `AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception`.
- The exception is defined in `LICENSE` as AGPLv3 §7 “Additional permissions” for:
  - LibreQoS Linking Exception (GPL-2.0-only combination with LibreQoS), and
  - LibreQoE Internal-Use Exception (internal proprietary combination and deployment by LibreQoE LLC).
- By submitting a contribution (e.g., PR/patch), you affirm you have the right to do so and you agree to license your contribution under these terms, including the exceptions.

## SPDX Headers (required)

All new and modified source files must include SPDX headers at the top of the file. For Rust files:

```rust
// SPDX-FileCopyrightText: 2025 LibreQoE support@libreqos.io
// SPDX-License-Identifier: AGPL-3.0-or-later WITH LicenseRef-LibreQoS-Exception
```

Use appropriate comment styles for other languages. Keep years and names current.

## Development Guidelines

- Style: run `cargo fmt` for formatting and `cargo clippy -p lqos_bus --all-features` for linting.
- Build: ensure `cargo build -p lqos_bus --all-features` succeeds without warnings where practical.
- Dependencies: prefer well-maintained crates with compatible licenses. Avoid adding strong copyleft conflicts beyond what the AGPL already imposes.
- API stability: the bus is intentionally unstable; document protocol changes in PRs.

## Commit and PR Tips

- Keep changes focused; include rationale and any performance/compatibility notes.
- If your change affects external integrations, note migration steps.
- Include tests where feasible or a short plan for manual verification.

## Developer Certificate of Origin (DCO)

We use a lightweight DCO process. Please add a Signed-off-by line to your commits:

```
Signed-off-by: Your Name <you@example.com>
```

This certifies that you have the right to submit the work under the project license and exceptions.

## Security and Sensitive Info

- Do not include secrets or customer data in issues or PRs.
- Report security concerns privately to support@libreqos.io.

## Questions

Open a discussion/issue or email support@libreqos.io. Thank you for contributing!

