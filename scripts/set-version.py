#!/usr/bin/env python3
"""Set the version everywhere: the app compares tauri.conf.json's with latest.json's.
Used by release.sh and by the x86_64 Linux build on GitHub (which builds the release's version
before release.sh commits it)."""
import re
import sys

ver = sys.argv[1]
for path, pattern in (
    ("Cargo.toml", r'^(version = ")[^"]*(")'),
    ("src-tauri/tauri.conf.json", r'("version": ")[^"]*(")'),
    ("package.json", r'("version": ")[^"]*(")'),
):
    s = open(path).read()
    s = re.sub(pattern, lambda m: m.group(1) + ver + m.group(2), s, count=1, flags=re.M)
    open(path, "w").write(s)
