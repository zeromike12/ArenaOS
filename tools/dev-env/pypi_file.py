#!/usr/bin/env python3
"""Resolve a PyPI release file URL by its exact SHA-256.

Some projects (ziglang among them) do not publish a wheel that `pip
download --platform` will select for this host, even though the release file
exists and is the one the project pinned. This helper asks the PyPI JSON API
for the release's file list and returns the URL of the file whose digest
matches. Selecting by digest rather than by filename means a renamed or
re-uploaded artifact cannot silently substitute itself.

Usage: pypi_file.py <project> <version> <sha256>
Exit status 1 (with a message) when no file in that release matches.
"""

import json
import sys
import urllib.request


def main(argv: list[str]) -> int:
    if len(argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    project, version, want = argv[1], argv[2], argv[3]
    url = f"https://pypi.org/pypi/{project}/{version}/json"
    with urllib.request.urlopen(url, timeout=120) as response:
        data = json.load(response)
    for entry in data.get("urls", []):
        if entry.get("digests", {}).get("sha256") == want:
            print(entry["url"])
            return 0
    print(f"{project} {version}: no release file matches {want}",
          file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
