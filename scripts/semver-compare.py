#!/usr/bin/env python3
"""Compare two versions by SemVer precedence (semver.org 2.0.0, section 11).

    scripts/semver-compare.py A B

prints `higher`, `equal` or `lower`: where A stands relative to B. Build
metadata (`+...`) does not count. A malformed version is an error (exit 2),
never an answer.

The release workflow uses it so the kanal-vorab channel only moves forward.
`sort -V` cannot do that job: it puts 0.10.0 before 0.10.0-pre.1, while
SemVer ranks a pre-release below its release.
"""

import re
import sys

IDENTIFIER = r"(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
VERSION = re.compile(
    r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    rf"(?:-({IDENTIFIER}(?:\.{IDENTIFIER})*))?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
)


def precedence(version):
    """A key that sorts versions in SemVer precedence order."""
    match = VERSION.fullmatch(version)
    if match is None:
        raise ValueError(f"not a SemVer version: {version!r}")
    major, minor, patch, pre = match.groups()
    core = (int(major), int(minor), int(patch))
    if pre is None:
        # A release ranks above every pre-release of the same version.
        return (core, 1, ())
    # Numeric identifiers compare as numbers and rank below alphanumeric
    # ones, which compare in ASCII order. When one list is the start of the
    # other, the longer one ranks higher; tuples compare the same way.
    identifiers = tuple(
        (0, int(part), "") if part.isdigit() else (1, 0, part)
        for part in pre.split(".")
    )
    return (core, 0, identifiers)


def main(argv):
    if len(argv) != 3:
        print("usage: semver-compare.py A B", file=sys.stderr)
        return 2
    try:
        a, b = precedence(argv[1]), precedence(argv[2])
    except ValueError as err:
        print(err, file=sys.stderr)
        return 2
    print("higher" if a > b else "lower" if a < b else "equal")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
