# 2.4.0 release validation

## Publication status

**Published.**

- Tag `v2.4.0` (annotated, `b25c034b4f2195c8a9fd558e100d39ecde1042fc`) points to the
  exact tested commit `cab703540f898407d74e420bf470f7ed32bc1771`.
- GitHub release "TimemAi 2.4.0" published 2026-10-09T08:47:14Z, non-draft,
  non-prerelease, notes from `docs/release-notes-v2.4.0.md`.
- PR #24 merged with a merge commit (`4981ece`) preserving the tested history;
  its tree is identical to `cab7035`.
- Local full `scripts/ci.sh` on `cab7035`: `ci: ok` (exit 0, clean tree).
- Native CI on `cab7035`: PR run 37896185985 passed all five platforms
  (Windows, Ubuntu, macOS latest, macOS 15, macOS 15 Intel).

## Native evidence and infrastructure incidents

- Initial push [37888984828](https://github.com/moliam/TimemAi/actions/runs/37888984828)
  and initial PR run [37888990333](https://github.com/moliam/TimemAi/actions/runs/37888990333)
  on the first candidate `7a82d2e`: four platform failures led to fixes R1-R3 below
  and blocked release at that point.
- Final push run [37896192646](https://github.com/moliam/TimemAi/actions/runs/37896192646)
  on `cab7035`: Windows, macOS latest, macOS 15, and Intel passed. Ubuntu failed twice
  on runner infrastructure only, never on code:
  1. Chrome for the real-browser acceptance never started (dbus parse errors, main
     process in D state, IO pressure 40%, DevTools port never ready).
  2. After rerunning the failed job, the full CI gate printed `ci: ok`, the I/O
     report artifact uploaded, and only the post-job cache-save step failed with a
     Path Validation Error. A `BrokenPipeError` in that log is a handler-thread
     exception in the fake model server after a client disconnected; the gate kept
     passing.
  The same commit's Ubuntu job passed in the final PR run, so no code regression is
  involved.
- Final PR run [37896185985](https://github.com/moliam/TimemAi/actions/runs/37896185985):
  its first Windows attempt stalled on a wedged runner ("Rust check and tests" step
  frozen for ~85 minutes with zero step progress, versus ~10 minutes for the same
  job in the push run). It was cancelled and rerun to completion; all five jobs then
  passed. No code or assertion changed during any rerun.

## Failure evidence and disposition

| ID | Failure on first candidate | Disposition |
|---|---|---|
| R1 | `session_turn_injects_due_focus_reminder_before_the_next_model_request` on macOS latest | Root cause reproduced under control: a slow prompt rebuild crossing another reminder period re-injected before one dispatch. Fix evaluates progress/time/round reminders once per completed-round dispatch boundary; new regression `reminder_rebuild_crossing_time_boundary_does_not_repeat_before_dispatch`; 98 session-runtime tests plus the exact regression passed. |
| R2 | Real-browser readfile scenario on Ubuntu | Old fixture was 34 UTF-8 bytes while actions read to inclusive byte 199; strict local control confirmed `SelectorNotFound`, and the old DOM check could pass on a still-running label. Fixture is now 280 bytes (measured); both stream and non-stream scenarios assert exactly three successful readfile finishes and fixture content in the next model input; full product suite passed. Byte selector semantics unchanged. |
| R3 | Large-tool handoff left stale scroll space on macOS 15 | Delayed-ResizeObserver control (500 ms) reproduced the stale space with old code. Fix directly invalidates outline geometry on known collapse/archive commits (`layoutKey`); all four stream/motion combinations pass under the control with the original 300 ms deadline. The historical runner pause itself was not captured; the fix is stated as the controlled mechanism, not proof of the historical cause. |
| R4 | `manual_toolgen_rejects_bad_source_state_and_duplicate_clicks` 3 s deadline on Windows (first candidate) | Did not reproduce on the release candidate: Windows passed natively on `cab7035` in both final runs. Root cause of the original one-off timeout remains unattributed; bounded diagnostics (event-handler timing, isolated exact reruns, atomic-write measurements) captured no failing target. Release proceeded under the user's criterion: the exact tagged commit green locally and on all five native runners, with the unattributed history known to the user before continuation. |
| R5 | `immediate_message_after_core_finalization_starts_a_new_turn` 2 s supplement window on a Windows diagnostic run | Same as R4: not present on the release candidate, not observed with attributable evidence in diagnostics, and released on full native green of the exact commit. |

## Completion gates

- [x] R4/R5 either attributed or explicitly released with unattributed history
      (full native green on the exact tagged commit; user informed and told to continue).
- [x] Initial PR evidence record completed, including Intel macOS success.
- [x] Narrow checks and reproducible Web assets: 709 Web tests in 52 files,
      two production builds with identical main asset `index-DwPT61vT.js`,
      architecture self-test, module-boundary, test-contract, formatting,
      whitespace, sensitive scan, and version consistency all passed.
- [x] Frozen candidate `cab7035`; complete local `scripts/ci.sh` on a clean tree
      with no concurrent edits (`/tmp/timem-v2.4.0-followup-ci.*`).
- [x] Native Windows, Ubuntu, macOS latest, macOS 15, Intel gates on `cab7035`
      (final PR run 37896185985 all green; push-run Ubuntu failures were
      post-gate runner infrastructure only).
- [x] PR merged preserving tested commits; ancestry (`cab7035` ancestor of main)
      and tree identity (`df3f094` both sides) verified; divergent local `main`
      checkout untouched.
- [x] Immutable annotated `v2.4.0` tag and non-draft, non-prerelease release
      published with major-feature and performance notes.
- [x] Temporary diagnostic branch/worktree removed locally and remotely after
      evidence was preserved in logs and this record.
