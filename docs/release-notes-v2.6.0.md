# TimemAi 2.6.0

TimemAi 2.6.0 turns upgrades into a one-click action inside the Web UI and
makes the release pipeline robust for every supported platform.

## In-app self upgrade

- Settings -> System now has a Software upgrade card: check for updates,
  download with a progress bar, automatic integrity verification, and a
  restart button that finishes the switch to the new version.
- The updater downloads the platform's prebuilt `tar.gz` from the official
  GitHub release, verifies the published sha256 sidecar, validates the new
  binary with `--help` before swapping it in, and keeps a timestamped backup
  next to the executable. Any failure leaves the running version untouched.
- The restart is a handover, not a stop: the new process takes the same port
  and access token, and the browser session keeps working after the reload.
- MEM workspaces, Sessions, credentials, and user configuration are never
  modified by an upgrade.

## Release pipeline

- Windows releases now ship a `tar.gz` archive next to the zip.
- Fixed release-asset backfill failures when the workflow passes a relative
  output directory to the packager.

## Upgrade from 2.5.0

Open Settings -> System and click "Check for updates", or keep using the
root `upgrade.sh` / `upgrade-online.ps1` scripts.
