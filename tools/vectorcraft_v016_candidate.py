#!/usr/bin/env python3
"""Stage VectorCraft v0.16 as a separate Android Test app in a throwaway builder.

No production certificate or package is used, and the checked-in adapter remains
unchanged. The caller copies adapters/vectorcraft into BUILDER/adapters first.
"""
import sys
import tomllib
from pathlib import Path

PROD = "com.artcraft.vectorcraft"
TEST = "com.artcraft.vectorcraft.candidate"
JAVA = ("VectorCraftActivity.java", "VectorCraftExtras.java")


def stage(builder: Path) -> None:
    adapter = builder / "adapters/vectorcraft"
    manifest = adapter / "Cargo.toml"
    original = manifest.read_text(encoding="utf-8")
    old = f'package = "{PROD}"'
    if original.count(old) != 1 or original.count('label = "VectorCraft"') != 2:
        raise ValueError("original Android package or label was modified")
    changed = original.replace(old, f'package = "{TEST}"', 1).replace(
        'label = "VectorCraft"', 'label = "VectorCraft Test"')
    a = tomllib.loads(changed)["package"]["metadata"]["android"]
    if a["package"] != TEST or a["build_targets"] != ["aarch64-linux-android"]:
        raise ValueError("incorrect test package or ABI")
    activity = a["application"]["activity"][0]
    if activity["name"] != ".VectorCraftActivity":
        raise ValueError("unexpected launcher Activity")
    changes = []
    for name in JAVA:
        path = adapter / "java/com/artcraft/vectorcraft" / name
        src = path.read_text(encoding="utf-8")
        declaration = f"package {PROD};"
        if not src.startswith(declaration + "\n") or src.count(declaration) != 1:
            raise ValueError("unexpected Java namespace: " + name)
        changes.append((path, src.replace(declaration, f"package {TEST};", 1)))
    manifest.write_text(changed, encoding="utf-8")
    for path, content in changes:
        path.write_text(content, encoding="utf-8")


def verify(builder: Path) -> None:
    adapter = builder / "adapters/vectorcraft"
    a = tomllib.loads((adapter / "Cargo.toml").read_text())["package"]["metadata"]["android"]
    if a["package"] != TEST or a["application"]["label"] != "VectorCraft Test":
        raise ValueError("not an isolated VectorCraft Test app")
    for name in JAVA:
        text = (adapter / "java/com/artcraft/vectorcraft" / name).read_text()
        if not text.startswith(f"package {TEST};\n"):
            raise ValueError("Java package mismatch: " + name)


if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] not in ("stage", "verify"):
        raise SystemExit("usage: vectorcraft_v016_candidate.py stage|verify BUILDER")
    try:
        (stage if sys.argv[1] == "stage" else verify)(Path(sys.argv[2]))
    except (OSError, KeyError, ValueError) as error:
        raise SystemExit(f"STOP: VectorCraft test package isolation failed: {error}")
    print("PASS: disposable VectorCraft v0.16 package identity is isolated")
