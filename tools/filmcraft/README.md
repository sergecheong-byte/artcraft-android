# FilmCraft Android v0.10.0 candidate (review only)

- Official release: https://github.com/storytold/filmcraft/releases/tag/v0.10.0
- Official release source SHA: `cdf43fd366e378da266cb6d3c94549a45244be1f` (tag must still resolve to this exact SHA at build time).
- Reviewed Android builder: `MostafaXS1/artcraft-android@ac54d62d9d2d6f663295a4afd8972a7a3e79f9af` (separate, pinned reference checkout; do **not** overwrite this repository's builder).
- Target package (provisional, no prior FilmCraft signer was found): `com.artcraft.filmcraft`.
- Android versionName: `0.10.0-android.1`.
- Provisional ephemeral-only versionCode: `1000001`. This is **not** a permanent release reservation. Confirm any older signed/published and **tablet-installed** FilmCraft versionCodes before reserving a permanent number.
- Expected exploratory artifact name: `filmcraft-0.10.0-android.1.apk`.

## Existing source capabilities to preserve and verify

The reviewed builder includes FilmCraft-specific SAF hooks, staged media access, file-opening intents, IME input, physical modifier handling, system fonts, audio, preference persistence, and mouse/touch wiring. These are candidates for Android testing, **not confirmed tablet functionality**. Work remaining includes shared user TTF/OTF folder import for title rendering without a floating dialog, customizable keyboard/Shortcut Pad UI, truthful GPU/CPU diagnostics, and opt-in manual/startup update checks. Never swap in a generic adapter if doing so removes these capabilities.

FilmCraft upstream v0.10.0 uses Rust 1.95, egui 0.36, and MIT OR Apache-2.0. Preserve its license/attribution notices. The liquid-glass `Fc` icon is user-supplied; do not substitute the upstream owl asset. The supplied original may be kept separately from its Android-density derivatives.

## Release blockers and safeguards

1. No FilmCraft signing repository/key/certificate has been identified among connected KosmosSG repositories. Do not treat absence of a repository as proof that no historical key exists elsewhere. Do not reuse PhotoCraft, VectorCraft or DesignCraft signing credentials.
2. An exploratory CI builder uses only a disposable temporary signing key. It **must never create a Release, tag, schedule, permanent keystore, or overwrite an APK**. A successful CI build is not a signed release.
3. Before provisioning a permanent FilmCraft key, check package identity, all known prior signers and versionCodes (including installed tablet app if any); verify encrypted key backup and recovery. Compare certificate and hash with the chosen immutable release baseline.
4. Prior to any public signed release, validate ARM64 packaging, launcher content, file SAF bridge, font, keyboard, audio playback, import/export and GPU fallback, plus Android 10+ scoped-storage implications. Confirm tablet behavior only after actual user device testing.
5. Source checking for future upstream releases may produce only a **draft PR** until reviewed. No unattended publishing, auto-merge or schedule is configured by this candidate.

No existing PhotoCraft, VectorCraft or DesignCraft paths should be changed by this FilmCraft work.
