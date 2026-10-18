# Release Management

This document defines the release contract for TimemAi. A public release is an
immutable tag over a commit that already passed the same production gate both
locally and on GitHub.

## Version Sources

Keep the release version identical in the workspace Cargo manifests and lock
file, the Timem Web package metadata, `CHANGELOG.md`, and the matching
`docs/release-notes-vX.Y.Z.md`. `scripts/version_consistency_check.sh` is the
executable authority for this rule.

## Required Order

1. Update user documentation, architecture/module boundaries, the feature-test
   ledger, changelog, and release notes in the same branch as the behavior.
2. Regenerate the embedded Timem Web production bundle and verify that a second
   build produces no diff.
3. Run `scripts/ci.sh` from a clean working tree. This includes warning-free
   Clippy, workspace and frontend tests, release builds, performance and edge
   guards, cross-host resume, loopback service tests, and real TTY smoke.
4. Push the release branch and require the GitHub Ubuntu and macOS production
   CI jobs for that exact commit to succeed.
5. Create one annotated `vX.Y.Z` tag pointing to that verified commit. Never move or overwrite a published tag.
6. Create a non-draft, non-prerelease GitHub Release from the committed release
   notes, then verify its tag and public URL.
7. Verify the `Release assets` workflow uploaded the prebuilt archives and
   `.sha256` sidecars for all four targets to that Release.
8. Open a pull request from the release branch into `main`; do not bypass its
   required checks.

## Timem Web Release Evidence

For a Web-facing release, the notes and README must make the supported path
explicit: install, run `timem`, then configure the selected Session in the
browser. Document `timem --shell` as the optional terminal mode. Highlight authenticated local startup, per-Session model/API settings,
multi-Session work, reconnect/replay behavior, queued message recovery, MCP and
ToolRepo controls, Markdown rendering, responsive layout, and server-side
secret handling when those capabilities are present in the release.

Record any manual browser/terminal checks in `docs/manual-release-smoke.md`.
Do not claim exactly-once execution for irreversible external tools across a
process crash unless that tool supplies an idempotency or reconciliation
contract.

## Release assets

Every public release carries prebuilt archives produced by
`.github/workflows/release-assets.yml`. When the Release is published, the
workflow builds the release binary for `x86_64-pc-windows-msvc`,
`aarch64-apple-darwin`, `x86_64-apple-darwin`, and `x86_64-unknown-linux-gnu`,
packages it with `scripts/package_release_assets.sh`, smoke-tests `--help`,
and uploads `timem-vX.Y.Z-<target>.zip` (Windows) or `.tar.gz` (macOS/Linux)
plus a `.sha256` sidecar to the Release.

A manual `workflow_dispatch` run resolves the latest Release tag, checks out
exactly that tag, and rebuilds it. Published tags are immutable; a broken asset
is fixed by re-running the workflow for the same tag, where
`gh release upload --clobber` replaces only the asset files, never the tag.

Prebuilt archives are a download convenience. The reviewable install contract
remains the one-line installer and the checkout installers. Archives contain
only `timem`, the `timem-web` alias, the shipped `reminder_tips.json`, and a
package README; extracting a newer archive over an existing installation
replaces the executable only and preserves MEM workspaces, Sessions,
credentials, and user configuration.
