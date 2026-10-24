#!/usr/bin/env python3
"""Read-only source/feature guard for the isolated official VectorCraft v0.16.0 port.

No upstream synchronization, source checkout, production key, signing or publication.
The GitHub Action checks out immutable source independently and invokes this checker.
"""
import json
import os
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONFIG = ROOT / "configs/vectorcraft-v0.16.0-android.1.json"
SHA = "10190753ef356ce8de7592626bcb99fd69db467c"
TAG = "v0.16.0"
REPO = "https://github.com/storytold/vectorcraft"
MENU_ACTIONS = (
    "android.vectorcraft.fontImport",
    "android.vectorcraft.fontFolder",
    "android.vectorcraft.fontRefresh",
    "android.vectorcraft.updates",
    "android.vectorcraft.startupUpdates",
    "android.vectorcraft.touchPad",
    "android.vectorcraft.diagnostics",
)
BRIDGE_MARKERS = (
    "vcRequestOpen", "vcRequestSave", "apply_android_modifiers",
    "text.rescanFonts", "vcCheckForUpdates", "vcPollUpdateResult",
    "touch_dock", "wgpu_render_state",
)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def preflight():
    cfg = json.loads(CONFIG.read_text(encoding="utf-8"))
    expected = {
        "app": "vectorcraft", "upstream_repository": REPO,
        "upstream_tag": TAG, "upstream_commit": SHA,
        "upstream_workspace_version": "0.16.0",
        "upstream_rust_toolchain": "1.95.0",
        "android_version_name": "0.16.0-android.1",
        "android_package": "com.artcraft.vectorcraft",
        "android_target": "aarch64-linux-android",
        "android_min_sdk": 28, "android_target_sdk": 35,
        "existing_signed_version_code_floor_exclusive": 16780805,
        "production_signing_allowed": False,
        "production_apk_build_allowed": False,
        "test_apk_build_allowed": True, "publishing_allowed": False,
        "automatic_upstream_sync_allowed": False,
    }
    for key, value in expected.items():
        require(cfg.get(key) == value, f"unsafe or changed v0.16 config: {key}")
    # Temporary APK build permission is exclusive to the named, reviewed
    # test branch. It cannot be granted on main or production release branches.
    require(
        os.environ.get("GITHUB_REPOSITORY") == "KosmosSG/artcraft-android"
        and os.environ.get("GITHUB_REF")
            == "refs/heads/development/vectorcraft-v016-popup-test-20261024",
        "isolated v0.16 popup test branch identity was not proved",
    )
    adapter = (ROOT / "adapters/vectorcraft/src/lib.rs").read_text()
    require(all(m in adapter for m in BRIDGE_MARKERS), "Android adapter feature marker missing")
    manifest = tomllib.loads((ROOT / "adapters/vectorcraft/Cargo.toml").read_text())
    android = manifest["package"]["metadata"]["android"]
    require(android["package"] == "com.artcraft.vectorcraft", "Android package changed")
    require(android["build_targets"] == ["aarch64-linux-android"], "Android ABI changed")
    require(android["sdk"]["min_sdk_version"] == 28, "Minimum SDK changed")
    require(android["sdk"]["target_sdk_version"] == 35, "Target SDK changed")
    print("PASS: VectorCraft v0.16 pin, read-only CI policy and Android features")


def git(source, *args):
    return subprocess.check_output(
        ["git", "-C", str(source), *args],
        text=True, stderr=subprocess.PIPE).strip()


def verify_source(source):
    source = Path(source)
    require((source / ".git").exists(), "expected a Git checkout")
    origin = git(source, "remote", "get-url", "origin").removesuffix(".git")
    require(origin == REPO, "wrong upstream origin")
    require(git(source, "rev-parse", "HEAD") == SHA, "wrong official upstream source SHA")
    manifest = tomllib.loads((source / "Cargo.toml").read_text(encoding="utf-8"))
    ws = manifest["workspace"]["package"]
    require(ws["version"] == "0.16.0" and ws["rust-version"] == "1.95",
            "official workspace version or rust-version changed")
    return source


def verify_port(source):
    source = verify_source(source)
    text = (source / "crates/ui-egui/src/menus.rs").read_text(encoding="utf-8")
    for action in MENU_ACTIONS:
        require(action in text, "missing Android menu action: " + action)
    require("take_android_menu_bits" in text and "set_android_preferences" in text,
            "missing Android menu command bridge")
    launcher = source / "apps/vectorcraft-android/src/lib.rs"
    require(launcher.is_file(), "missing isolated Android launcher crate")
    print("PASS: exact VectorCraft v0.16 source, Android menu port and adapter")


def main():
    require(len(sys.argv) in (2, 3), "usage: script preflight|source|port [source_checkout]")
    mode = sys.argv[1]
    if mode == "preflight":
        require(len(sys.argv) == 2, "preflight takes no checkout")
        preflight()
    else:
        require(len(sys.argv) == 3, "source checkout path required")
        preflight()
        if mode == "source":
            verify_source(sys.argv[2])
            print("PASS: pinned official VectorCraft v0.16.0 checkout")
        elif mode == "port":
            verify_port(sys.argv[2])
        else:
            raise ValueError("unknown command")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as exc:
        raise SystemExit(f"STOP: VectorCraft 0.16 isolated compatibility guard failed: {exc}")
