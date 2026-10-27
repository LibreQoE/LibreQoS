#!/usr/bin/env python3
"""Build-script smoke.

Every crate referenced by the packaging/build scripts (`build_rust.sh`,
`build_dpkg.sh`) must be a current workspace member. K3-A deleted
`lqos_map_perf` and `xdp_iphash_to_cpu_cmdline`, but the scripts kept
referencing them, so the `.deb` silently stopped building. CI runs this before
the deb build so the next stale-crate reference fails fast and by name.
"""
import pathlib
import re
import sys

HERE = pathlib.Path(__file__).resolve().parent
SCRIPTS = ("build_rust.sh", "build_dpkg.sh")


def workspace_members() -> set[str]:
    text = (HERE / "rust" / "Cargo.toml").read_text()
    block = re.search(r"\[workspace\](.*?)(?:\n\[|\Z)", text, re.S)
    if block is None:
        raise SystemExit("build-script smoke: no [workspace] table found")
    members = re.search(r"members\s*=\s*\[(.*?)\]", block.group(1), re.S)
    if members is None:
        raise SystemExit("build-script smoke: no workspace members list found")
    out: set[str] = set()
    for line in members.group(1).splitlines():
        line = line.split("#", 1)[0].strip().rstrip(",").strip().strip('"')
        if line:
            out.add(line)
    return out


def bin_targets() -> set[str]:
    """Binary target names across the workspace (`[[bin]] name = "..."`).

    `RUSTPROGS` in `build_dpkg.sh` lists BINARY names, not crates: it is
    consumed as `for prog in "${RUSTPROGS[@]}"` to copy `bin/<prog>`. A bin
    whose name differs from its crate (`libreqos-power-profile` in
    `lqos_power`) is therefore a valid reference even though it is not a
    workspace member.
    """
    out: set[str] = set()
    for cargo in sorted((HERE / "rust").glob("*/Cargo.toml")):
        text = cargo.read_text()
        for block in re.findall(r"\[\[bin\]\](.*?)(?=\n\[|\Z)", text, re.S):
            match = re.search(r'name\s*=\s*"([^"]+)"', block)
            if match:
                out.add(match.group(1))
    return out


def referenced_crates() -> dict[str, set[str]]:
    refs: dict[str, set[str]] = {}
    for name in SCRIPTS:
        text = (HERE / name).read_text()
        # Literal `-p <crate>` continuation lines (cargo build). Line-anchored
        # so `mkdir -p bin/static` and `"-p" "$pkg"` are not mistaken for
        # crate references.
        found = set(re.findall(r"^\s*-p\s+([A-Za-z0-9_]+)\s*\\?\s*$", text, re.M))
        for array in re.findall(
            r"(?:PROGS|BUILD_PACKAGES|RUSTPROGS)=\((.*?)\)", text, re.S
        ):
            for line in array.splitlines():
                line = line.split("#", 1)[0].strip()
                if line:
                    found.add(line)
        refs[name] = found
    return refs


def main() -> int:
    members = workspace_members()
    known = members | bin_targets()
    missing: list[str] = []
    referenced: set[str] = set()
    for name, refs in referenced_crates().items():
        referenced |= refs
        for crate in sorted(refs - known):
            missing.append(f"{name}: {crate}")
    if missing:
        for item in missing:
            print(
                f"build-script smoke: {item} is referenced but is not a "
                "workspace member or binary target",
                file=sys.stderr,
            )
        return 1
    print(
        "build-script smoke: all referenced crates are workspace members or "
        f"binary targets ({len(referenced)} checked)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
