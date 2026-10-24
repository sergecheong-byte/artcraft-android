#!/usr/bin/env python3
"""Verify official VectorCraft tool-selection shortcuts survive Android builds.

Runs against the exact upstream source checkout, before compiling the test APK.
This is structural protection, NOT proof that a hardware keyboard was tested.
"""
import argparse
from pathlib import Path


DEFAULTS = {
    "selection": ("V", "Selection Tool"),
    "directSelection": ("A", "Direct Selection Tool"),
    "type": ("T", "Type Tool"),
    "pen": ("P", "Pen Tool"),
    "rectangle": ("M", "Rectangle Tool"),
    "ellipse": ("L", "Ellipse Tool"),
    "paintbrush": ("B", "Paintbrush Tool"),
    "pencil": ("N", "Pencil Tool"),
    "hand": ("H", "Hand Tool"),
    "zoom": ("Z", "Zoom Tool"),
    "touchType": ("Shift+T", "Touch Type Tool"),
    "blobBrush": ("Shift+B", "Blob Brush Tool"),
}


def verify(root: Path) -> None:
    catalog = (root / "crates/tools/src/catalog.rs").read_text(encoding="utf-8")
    shortcuts = (root / "crates/ui-egui/src/shortcuts.rs").read_text(encoding="utf-8")
    editor = (root / "crates/ui-egui/src/shortcut_editor.rs").read_text(encoding="utf-8")

    for tool, (shortcut, label) in DEFAULTS.items():
        expected = f't("{tool}", "{label}", Some("{shortcut}"),'
        if expected not in catalog:
            raise ValueError(f"official {tool} default {shortcut} missing or changed")

    for feature, marker, source in [
        ("user-customizable tool selection", "pub fn tool_for_key(key: &str)", editor),
        ("user-customized key lookup", "crate::shortcut_editor::tool_for_key", shortcuts),
        ("activation of selected tool", "app.select_tool(t);", shortcuts),
        ("text-field focus gate", "let typing = ctx.egui_wants_keyboard_input();", shortcuts),
        ("active text editor gate", "if app.session.tool_wants_text() {", shortcuts),
        ("IME composition gate", "if composing || app.session.tool_composing() {", shortcuts),
        ("plain-key handling", "Single-key tool shortcuts (no Cmd/Ctrl/Alt)", shortcuts),
        ("modifier chord protection", "if m.command || m.alt || m.ctrl {", shortcuts),
    ]:
        if marker not in source:
            raise ValueError(f"upstream {feature} changed; review before candidate release")

    print("PASS: VectorCraft V/T/A/P/M/L/B/N/H/Z and Shift tool keys")
    print("PASS: editable mappings and text-field/IME protections present")
    print("NOTE: Hardware-key events and text generation still require tablet validation")


if __name__ == "__main__":
    cli = argparse.ArgumentParser()
    cli.add_argument("upstream_checkout", type=Path)
    args = cli.parse_args()
    try:
        verify(args.upstream_checkout)
    except (ValueError, OSError) as error:
        raise SystemExit(f"STOP: VectorCraft tool shortcuts preservation gate: {error}")
