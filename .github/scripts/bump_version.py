#!/usr/bin/env python3
"""Bump the package version in Cargo.toml based on a Conventional Commit PR title."""

import os
import re
import sys
from pathlib import Path

CONVENTIONAL_RE = re.compile(
    r"^(?P<type>[a-zA-Z]+)(?:\([^)]+\))?(?P<breaking>!)?: .+"
)
PACKAGE_SECTION_RE = re.compile(r"^\[package\]\s*$")
SECTION_RE = re.compile(r"^\[.+\]\s*$")
VERSION_LINE_RE = re.compile(r'^(version\s*=\s*")(\d+)\.(\d+)\.(\d+)(".*)$')


def determine_bump(pr_title: str, pr_body: str = "") -> str:
    match = CONVENTIONAL_RE.match(pr_title.strip())
    if not match:
        return "patch"

    if match.group("breaking") == "!" or re.search(
        r"^BREAKING[ -]CHANGE:", pr_body, re.MULTILINE
    ):
        return "major"

    if match.group("type").lower() == "feat":
        return "minor"

    return "patch"


def bump_semver(major: int, minor: int, patch: int, bump: str) -> tuple[int, int, int]:
    if bump == "major":
        return major + 1, 0, 0
    if bump == "minor":
        return major, minor + 1, 0
    return major, minor, patch + 1


def update_cargo_toml(cargo_path: Path, bump: str) -> tuple[str, str]:
    lines = cargo_path.read_text(encoding="utf-8").splitlines(keepends=True)
    in_package = False
    old_version = None
    new_version = None

    for idx, line in enumerate(lines):
        stripped = line.strip()
        if PACKAGE_SECTION_RE.match(stripped):
            in_package = True
            continue
        if in_package and SECTION_RE.match(stripped):
            in_package = False
            continue

        if in_package:
            m = VERSION_LINE_RE.match(line.rstrip("\r\n"))
            if m:
                prefix, maj_s, min_s, pat_s, suffix = m.groups()
                old_version = f"{maj_s}.{min_s}.{pat_s}"
                new_maj, new_min, new_pat = bump_semver(
                    int(maj_s), int(min_s), int(pat_s), bump
                )
                new_version = f"{new_maj}.{new_min}.{new_pat}"
                ending = "\r\n" if line.endswith("\r\n") else ("\n" if line.endswith("\n") else "")
                lines[idx] = f"{prefix}{new_version}{suffix}{ending}"
                break

    if old_version is None or new_version is None:
        raise RuntimeError(f"Could not find [package] version in {cargo_path}")

    cargo_path.write_text("".join(lines), encoding="utf-8")
    return old_version, new_version


def main() -> int:
    pr_title = os.environ.get("PR_TITLE", "")
    pr_body = os.environ.get("PR_BODY", "")
    cargo_path = Path(os.environ.get("CARGO_TOML_PATH", "Cargo.toml"))

    if not pr_title:
        print("PR_TITLE environment variable is required", file=sys.stderr)
        return 1

    bump = determine_bump(pr_title, pr_body)
    old_version, new_version = update_cargo_toml(cargo_path, bump)
    print(f"Bump type: {bump} ({old_version} -> {new_version})")

    github_output = os.environ.get("GITHUB_OUTPUT")
    if github_output:
        with open(github_output, "a", encoding="utf-8") as fh:
            fh.write(f"bump={bump}\n")
            fh.write(f"old_version={old_version}\n")
            fh.write(f"new_version={new_version}\n")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
