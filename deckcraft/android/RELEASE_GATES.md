# DeckCraft Android signed review: release verification checklist

Status: **NOT APPROVED FOR PRODUCTION**. This checklist does not claim device testing.

## Verified in GitHub CI
- Signed review workflow [run 39829744384](https://github.com/KosmosSG/artcraft-android/actions/runs/39829744384) succeeded.
- GitHub Actions steps confirm pinned DeckCraft v0.9.0 source, isolated Android builder, app-specific signing identity matched to original backup fingerprint, ARM64 APK compilation, package/version/resources integrity, and signed CI artifact upload.
- Artifact: `DeckCraft-v0.9.0-arm64-signed-review`. The ZIP contains the review APK and checksum evidence; review artifact is not a GitHub Release.
- Backup bootstrap [run 39822965531](https://github.com/KosmosSG/artcraft-android/actions/runs/39822965531) succeeded. User must download and preserve the encrypted backup and its separately stored password off GitHub before publication.

## Release-blocking features / validation not yet proven
- [ ] Android-private persistent imported TTF/OTF storage after restart and update
- [ ] Android SAF shared ArtCraft font folder selection, reload, and revoked-permission handling
- [ ] DeckCraft-specific Help > Check for Updates, safe signed-release filtering, and opt-in startup preference
- [ ] Tablet physical keyboard shortcuts, optional remapping and docked shortcut pad
- [ ] Honest graphics diagnostics and CPU fallback behavior on target device
- [ ] File open, Save/Save As, export (PPTX, PDF, PNG) and slide content round-trip on HONOR MagicPad 4
- [ ] Android lifecycle, rotation, touchscreen, external mouse and keyboard tested
- [ ] Approved permanent signing backup downloaded/offline and restored in independent dry run
- [ ] Package ID, release versionCode monotonicity and compatibility with any installed historical DeckCraft APK
- [ ] Public-distribution branding and licensing review
- [ ] Explicit approved release notes and artifact SHA-256 matched to tablet-tested APK

**Do not merge or publish simply because CI is green.** CI has not established the above device-level behaviors. Keep other ArtCraft applications untouched.
