# Manual Release Smoke

This checklist covers release evidence that is intentionally not part of
default CI because it depends on local browsers, terminal emulators,
credentials, or a disposable machine. Run the relevant rows before a broad
public release or when changing the touched surface.

Record the date, commit, host OS, and short result beside each row in the
release notes or PR summary.

## Web Browser Matrix

Run `timem-web` with a fake provider for deterministic behavior, then repeat a
short live-provider turn only when credentials are intentionally available.

Required rows before a broad Web release:

| Browser | Scope | Acceptance |
|---|---|---|
| Chromium or Chrome | Baseline automated-equivalent smoke | Page opens from the local URL, access token works, Session0 appears, composer stays docked, one normal turn completes, final answer and token telemetry render, no body horizontal overflow at desktop and narrow mobile width. |
| Safari | Engine-specific manual smoke | Same as Chromium, plus reconnect after refresh keeps the session, local storage appearance preferences survive refresh, code blocks/tables render without layout breakage. |
| Firefox | Engine-specific manual smoke | Same as Chromium, plus WebSocket reconnect, scroll anchoring after older-history load, and attachment remove/submit behavior remain correct. |

Suggested fake-provider sequence:

1. Start `target/release/timem-web --no-open` with
   `TIMEM_BASE_URL=http://127.0.0.1:<fake-provider-port>/v1`.
2. Submit one simple task.
3. Submit one action-producing task.
4. Rename the session.
5. Upload and remove one pending attachment.
6. Refresh the page and confirm state is not stale.
7. Narrow the window to a mobile width and confirm the composer remains usable.

## Terminal Emulator Matrix

Run the release binary in each target terminal when changing input, paste,
redraw, status, or cancellation code.

| Environment | Scope | Acceptance |
|---|---|---|
| iTerm2 | Interactive shell baseline | `/help`, `/config`, multiline paste, edited paste recovery, Ctrl+C/Esc cancel, and mid-turn supplement behave like the pseudo-TTY smoke. |
| Terminal.app | macOS default terminal | Same as iTerm2; no duplicate prompt rows or broken CJK/backspace behavior. |
| tmux | Multiplexer path | Bracketed paste, redraw, and Ctrl+C remain usable inside a pane; if broken, document the limitation before release. |
| SSH session | Remote pure shell path | Shell UI starts without desktop assumptions, paste/cancel works, and no Web-only behavior is required. |

## Clean-Machine Install

Use a disposable VM, container, or fresh user account.

Acceptance:

- `./install.sh` installs `timem`, `timem-native-rs`, and `timem-web` without
  requiring undocumented manual steps.
- `cp env_template env && source env` works after adding a test API key/model.
- `timem --help` matches README startup/config guidance.
- `timem` starts, `/help` is intercepted by the shell UI, and `/exit` exits.
- `timem-web --no-open` prints a loopback URL and serves the embedded Web UI.
- `./uninstall.sh` removes installed commands without deleting user-managed env
  files or memory data.

## Live Provider Smoke

Run only with explicit throwaway credentials. Do not add credentials or raw live
audit logs to git.

Acceptance:

- One normal XML response completes without protocol repair.
- One run_bash action completes and renders as a tool event.
- One malformed fixture or naturally malformed response enters protocol repair
  and recovers, or produces a bounded user-visible failure after the configured
  repair limit.
- Provider usage fields update input/output/cache telemetry.
- API audit redacts keys and does not store private prompt text outside the
  intended audit files.

## Evidence Rules

- CI remains authoritative for deterministic checks.
- Manual smoke is evidence for host-specific behavior only; do not use it to
  waive failing unit, integration, Web, or script checks.
- If a manual row fails, either fix it before release or explicitly document the
  unsupported environment and the reason.

## Recent Local Evidence

| Date | Commit | Host | Scope | Result |
|---|---|---|---|---|
| 2026-07-16 | `fd6ac7f` | macOS Darwin 25.5.0 arm64, Codex in-app Chromium browser | Fake-provider Web smoke on `timem-web --no-open`: initial desktop layout, task submit, rapid repeated Stop/Send during an active turn, second turn completion, console error check, 390px mobile viewport composer/overflow check. | Passed: no horizontal overflow, composer stayed visible, session state returned from busy to ready, no `active_turn_not_found`/runtime-error text, no browser console errors. |

Rows not covered by this local smoke remain explicit manual release work:
Safari, Firefox, iTerm2, Terminal.app, tmux, SSH, clean-machine install, and
live-provider behavior with throwaway credentials.
