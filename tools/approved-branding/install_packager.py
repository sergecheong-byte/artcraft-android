#!/usr/bin/env python3
"""Install a checksum-pinned cargo-apk2 with explicit versionCode support.

Upstream 1.5.0 discards Android revision suffixes and rejects version_code.
Keep its default calculation but permit the EffectCraft manifest's explicit code.
Only the temporary CI runner's packager is changed.
"""
import hashlib
import io
from pathlib import Path
import subprocess
import tarfile
import tempfile
import urllib.request

URL = "https://static.crates.io/crates/cargo-apk2/cargo-apk2-1.5.0.crate"
SHA256 = "36e7a84b003ed4d118b67377e576e05ed313a18934713d3254c236ca42fbf02f"
BEFORE = '''        if manifest
            .android_manifest
            .version_code
            .replace(version_code)
            .is_some()
        {
            panic!("version_code should not be set in TOML");
        }'''
AFTER = '''        manifest.android_manifest.version_code.get_or_insert(version_code);'''

def patch(source):
    if source.count(BEFORE) != 1:
        raise ValueError("Unexpected cargo-apk2 version-code implementation")
    return source.replace(BEFORE, AFTER, 1)

def main():
    with urllib.request.urlopen(URL, timeout=60) as response:
        data = response.read(4 * 1024 * 1024)
    if hashlib.sha256(data).hexdigest() != SHA256:
        raise ValueError("cargo-apk2 archive checksum mismatch")
    with tempfile.TemporaryDirectory(prefix="effectcraft-packager-") as folder:
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
            archive.extractall(folder, filter="data")
        root = Path(folder) / "cargo-apk2-1.5.0"
        path = root / "src/apk.rs"
        path.write_text(patch(path.read_text()))
        subprocess.run(["cargo", "install", "--path", str(root), "--locked", "--force"], check=True)

if __name__ == "__main__":
    main()
