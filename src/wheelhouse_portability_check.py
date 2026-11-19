#!/usr/bin/env python3
"""Fail if an offline Python wheelhouse contains a CPython-ABI-specific wheel.

The release wheelhouse is built once on Ubuntu 24.04 (CPython 3.12) but the
package supports 24.04+ including 26.04 (CPython 3.14). pip silently skips a
wheel whose ABI does not match the running interpreter, so an all-cp312
wheelhouse installs on 3.12 and fails on 3.14 with "No matching distribution
found" (the beta20.7 field failure: chardet, then cachebox).

Portable wheels only:
  * python tag `py3` or `py2.py3`, ABI `none`, platform `any`; or
  * ABI `abi3` (the stable ABI; forward-compatible across 3.x, e.g.
    psutil's `cp36-abi3-manylinux...`).

The python tag is checked explicitly: `foo-1.0-cp312-none-any.whl` has ABI
`none` and platform `any` but is still CPython-3.12-specific and is REJECTED.

Usage: wheelhouse_portability_check.py <wheelhouse-dir>
Exit 0 = portable, 1 = at least one non-portable wheel (listed on stderr).
"""

from __future__ import annotations

import pathlib
import sys


def wheel_tags(name: str) -> tuple[str, str] | None:
    """Return (python_tag, abi_tag) for a wheel filename, or None."""
    if not name.endswith(".whl"):
        return None
    stem = name[: -len(".whl")]
    # {distribution}-{version}(-{build})?-{python}-{abi}-{platform}
    parts = stem.rsplit("-", 3)
    if len(parts) != 4:
        return None
    _, python_tag, abi_tag, _platform = parts
    return python_tag, abi_tag


def is_portable(name: str) -> bool:
    tags = wheel_tags(name)
    if tags is None:
        return False
    python_tag, abi_tag = tags
    if abi_tag == "abi3":
        return True
    return abi_tag == "none" and python_tag in ("py3", "py2.py3")


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(f"usage: {argv[0]} <wheelhouse-dir>", file=sys.stderr)
        return 2
    wheelhouse = pathlib.Path(argv[1])
    if not wheelhouse.is_dir():
        print(f"ERROR: not a wheelhouse directory: {wheelhouse}", file=sys.stderr)
        return 2
    bad = [w.name for w in sorted(wheelhouse.glob("*.whl")) if not is_portable(w.name)]
    if bad:
        print(
            "ERROR: offline Python wheelhouse has CPython-ABI-specific wheels; "
            "these are ignored on other supported Python versions (for example "
            "Ubuntu 26.04 / CPython 3.14):",
            file=sys.stderr,
        )
        for name in bad:
            print(f"  {name}", file=sys.stderr)
        print(
            "Pin the dependency to a portable distribution or build the "
            "wheelhouse for the target ABI(s).",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
