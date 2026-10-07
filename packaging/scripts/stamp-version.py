#!/usr/bin/env python3
# Project:   git-scrub
# File:      packaging/scripts/stamp-version.py
# Purpose:   Stamp a release version into VERSION, Cargo.toml and Cargo.lock
# Language:  Python
#
# License:   Apache-2.0
# Copyright: (c) 2026 HYPERI PTY LIMITED

"""Stamp a release version into VERSION, Cargo.toml and Cargo.lock.

Usage: stamp-version.py <version>

Runs before the release build, so the binaries report the version being
released, and again in semantic-release's prepare step, so the commit it
pushes carries the same version in all three files.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SEMVER = re.compile(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?")


def substitute_once(path: Path, pattern: str, replacement: str) -> None:
    """Replace the first match of ``pattern`` in ``path``, failing if there is none."""
    text = path.read_text(encoding="utf-8")
    stamped, count = re.subn(pattern, replacement, text, count=1, flags=re.MULTILINE)
    if count != 1:
        sys.exit(f"stamp-version: no version line found in {path}")
    path.write_text(stamped, encoding="utf-8", newline="\n")


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit("usage: stamp-version.py <version>")
    version = sys.argv[1].removeprefix("v")
    if not SEMVER.fullmatch(version):
        sys.exit(f"stamp-version: not a semver version: {version!r}")

    (ROOT / "VERSION").write_text(f"{version}\n", encoding="utf-8", newline="\n")
    substitute_once(ROOT / "Cargo.toml", r'^version\s*=\s*"[^"]*"', f'version = "{version}"')
    substitute_once(
        ROOT / "Cargo.lock",
        r'^(name = "git-scrub"\nversion = )"[^"]*"',
        rf'\g<1>"{version}"',
    )
    print(f"stamped {version} into VERSION, Cargo.toml and Cargo.lock")


if __name__ == "__main__":
    main()
