#!/usr/bin/env python3

"""Check repository-local links in Markdown without network access."""

import os
import re
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit


EXCLUDED_DIRECTORIES = {".git", "target"}
INLINE_LINK = re.compile(r"!?\[[^\]]*\]\(\s*(<[^>]+>|[^\s)]+)")
REFERENCE_LINK = re.compile(r"^\s*\[[^\]]+\]:\s*(<[^>]+>|\S+)", re.MULTILINE)
FENCE = re.compile(r"^\s*(```|~~~)")


def markdown_files(root: Path):
    for current, directories, files in os.walk(root):
        directories[:] = sorted(
            directory
            for directory in directories
            if directory not in EXCLUDED_DIRECTORIES
        )
        for filename in sorted(files):
            if filename.lower().endswith(".md"):
                yield Path(current, filename)


def without_fenced_code(contents: str) -> str:
    retained = []
    in_fence = False
    for line in contents.splitlines(keepends=True):
        if FENCE.match(line):
            in_fence = not in_fence
            retained.append("\n")
        elif in_fence:
            retained.append("\n")
        else:
            retained.append(line)
    return "".join(retained)


def local_target(raw_target: str):
    target = raw_target[1:-1] if raw_target.startswith("<") else raw_target
    split = urlsplit(target)
    if split.scheme or split.netloc or not split.path:
        return None
    return unquote(split.path)


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    failures = []

    for markdown in markdown_files(root):
        contents = without_fenced_code(markdown.read_text(encoding="utf-8"))
        matches = list(INLINE_LINK.finditer(contents))
        matches.extend(REFERENCE_LINK.finditer(contents))
        for match in matches:
            target = local_target(match.group(1))
            if target is None:
                continue
            destination = root / target.lstrip("/") if target.startswith("/") else markdown.parent / target
            if not destination.resolve().exists():
                line = contents.count("\n", 0, match.start()) + 1
                failures.append(
                    f"{markdown.relative_to(root)}:{line}: missing local link target {target!r}"
                )

    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1

    print("all repository-local Markdown links resolve")
    return 0


if __name__ == "__main__":
    sys.exit(main())
