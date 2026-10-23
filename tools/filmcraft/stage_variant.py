#!/usr/bin/env python3
"""Safely stage FilmCraft-only display version and provisional versionCode.

Run only against a disposable checkout of the reviewed builder, never against
KosmosSG's working default branch or other apps' signing/build logic.
"""
from __future__ import annotations

import sys
from pathlib import Path

MARKER = "    code = version_code(version)\n"
REPLACEMENT = """    code = version_code(version)
    # Temporary FilmCraft variant only. All other ArtCraft apps retain the
    # reviewed builder's original versioning behavior unchanged.
    if app == "filmcraft":
        import os
        if version != "0.10.0":
            raise SystemExit("STOP: FilmCraft upstream version no longer matches v0.10.0")
        version = os.environ.get("FILMCRAFT_ANDROID_VERSION_NAME", "")
        if version != "0.10.0-android.1":
            raise SystemExit("STOP: invalid FilmCraft Android candidate versionName")
        value = os.environ.get("FILMCRAFT_EPHEMERAL_VERSION_CODE", "")
        if not value.isascii() or not value.isdecimal():
            raise SystemExit("STOP: numeric ephemeral versionCode is required")
        code = int(value)
        if not 1 <= code < 2100000000:
            raise SystemExit("STOP: invalid Android versionCode")
"""


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("Usage: stage_variant.py /path/to/disposable-builder/lib/android_project.py")
    target = Path(sys.argv[1]).resolve()
    if target.name != "android_project.py" or target.parent.name != "lib":
        raise SystemExit("STOP: unexpected builder file path")
    if "filmcraft-builder" not in target.parts:
        raise SystemExit("STOP: only disposable filmcraft-builder checkout may be patched")
    raw = target.read_text()
    if raw.count(MARKER) != 1 or REPLACEMENT in raw:
        raise SystemExit("STOP: reviewed builder version-code hook changed; review before updating")
    patched = raw.replace(MARKER, REPLACEMENT, 1)
    compile(patched, str(target), "exec")
    target.write_text(patched)
    print("Scoped FilmCraft version-name/code override staged in disposable checkout")


if __name__ == "__main__":
    main()
