# Sanitized v1 migration fixtures

These fixtures model SpeakEasy v1 configuration and preset inputs without containing user data, credentials, transcripts, audio, model files, reachable network endpoints, or host-specific paths.

## Rules

- Every path uses a reserved synthetic fixture name such as `C:\FixtureData`, `R:\FixtureRemovable`, or `\\fixture-server.invalid\share`.
- Remote URLs use `.invalid` or loopback.
- Credential presence is injected from `credential-cases.json`; no secret value is serialized.
- Corrupt files are intentional and listed in `manifest.json`.
- Baseline provenance identifies immutable upstream commits but fixture contents are synthetic test data, not copies of a user's settings.
- `SHA256SUMS` is regenerated only through a reviewed fixture change.

## Layout

- `baselines/` contains representative explicit test roots for v0.14.5, v0.15.0, and the pinned post-0.15 hotfix.
- `edge/` contains parser/validation hazards.
- `presets/` contains reusable preset collision/corruption inputs.
- `credential-cases.json` defines fake credential-store states.
- `expected-cases.json` defines planner and transaction expectations that the v1-import tests enforce.

These fixtures do not authorize a production path resolver or production migration writer.
