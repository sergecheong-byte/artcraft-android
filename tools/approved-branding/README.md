# LightCraft approved branding candidate

Approved source: owner-supplied ArtCraft_Android_Approved_Icon_Package_2026-10-24.zip.
Baseline commit: `1cc86e06bd0b56a55167768439bbf691a302cd7a`; existing release: `lightcraft-v0.11.0-android.6`.

The final generator hook installs the exact supplied legacy/adaptive resources,
changes only the Android application logo and related native branding, and preserves
app identity and existing feature integration. It runs after app-specific preparation.
UI textures are cached and compiled into the native binary; no new runtime file access.

The candidate workflow never publishes or accesses permanent signing keys.
Its temporary-signature APK is NOT an install-over update. Production signing must
use the app's existing permanent workflow, match the prior certificate, and pass
compiled-resource verification and exact-APK MagicPad installation/data/font/updater tests.
Device tests are pending; source validation does not substitute for them.

`assets/manifest.json` records approved master/resource hashes and proposed version.
`verify_apk.py` checks APK pixels, embedded UI bytes, adaptive XML, version and ARM64.
All previous releases and ongoing development branches remain untouched.
