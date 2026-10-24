#!/usr/bin/env python3
"""Stage a separately installable v0.14 keyboard test app in a disposable builder.

Only BUILDER/adapters/vectorcraft is modified, never tracked source or the
production application's package/signing identity. Fail closed on drift.
"""
from __future__ import annotations
import sys
import tomllib
from pathlib import Path

PRODUCTION = "com.artcraft.vectorcraft"
TEST_PACKAGE = "com.artcraft.vectorcraft.keyboardtest"
TEST_LABEL = "VectorCraft Keyboard Test"
JAVA_FILES = ("VectorCraftActivity.java", "VectorCraftExtras.java")


def stage(builder: Path) -> None:
    adapter = builder / "adapters" / "vectorcraft"
    manifest = adapter / "Cargo.toml"
    before = manifest.read_text(encoding="utf-8")
    package_line = f'package = "{PRODUCTION}"'
    if before.count(package_line) != 1 or before.count('label = "VectorCraft"') != 2:
        raise ValueError("unexpected production identity or labels; do not modify")
    modified = before.replace(package_line, f'package = "{TEST_PACKAGE}"', 1)
    modified = modified.replace('label = "VectorCraft"', f'label = "{TEST_LABEL}"')
    meta = tomllib.loads(modified)["package"]["metadata"]["android"]
    if meta["package"] != TEST_PACKAGE or meta["build_targets"] != ["aarch64-linux-android"]:
        raise ValueError("APK package or ARM64 targets not correctly isolated")
    if meta["application"]["activity"][0]["name"] != ".VectorCraftActivity":
        raise ValueError("unexpected launcher Activity")
    declarations = []
    for name in JAVA_FILES:
        path = adapter / "java" / "com" / "artcraft" / "vectorcraft" / name
        text = path.read_text(encoding="utf-8")
        original = f"package {PRODUCTION};"
        if not text.startswith(original + "\n") or text.count(original) != 1:
            raise ValueError(f"unexpected Android Java namespace: {name}")
        declarations.append((path, text.replace(original, f"package {TEST_PACKAGE};", 1)))
    manifest.write_text(modified, encoding="utf-8")
    for path, text in declarations:
        path.write_text(text, encoding="utf-8")
    verify(builder)


def verify(builder: Path) -> None:
    adapter = builder / "adapters" / "vectorcraft"
    meta = tomllib.loads((adapter / "Cargo.toml").read_text(encoding="utf-8"))["package"]["metadata"]["android"]
    if meta["package"] != TEST_PACKAGE or meta["application"]["label"] != TEST_LABEL:
        raise ValueError("test identity not found")
    if meta["application"]["activity"][0]["label"] != TEST_LABEL:
        raise ValueError("test launcher name not found")
    if meta["build_targets"] != ["aarch64-linux-android"]:
        raise ValueError("wrong target ABI")
    for name in JAVA_FILES:
        source = (adapter / "java" / "com" / "artcraft" / "vectorcraft" / name).read_text(encoding="utf-8")
        if not source.startswith(f"package {TEST_PACKAGE};\n"):
            raise ValueError(f"Java package still uses production: {name}")


if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] not in ("stage", "verify"):
        raise SystemExit("Usage: vectorcraft_keyboard_candidate.py stage|verify BUILDER")
    try:
        (stage if sys.argv[1] == "stage" else verify)(Path(sys.argv[2]))
    except (OSError, ValueError, KeyError, TypeError) as exc:
        raise SystemExit(f"STOP: test-only package guard failed: {exc}")
    print("PASS: isolated VectorCraft keyboard test app; production package untouched")
