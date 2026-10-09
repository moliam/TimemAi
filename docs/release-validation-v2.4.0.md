# 2.4.0 release validation

## Publication status

**Blocked; no release tag or GitHub release has been created.**

The initial candidate is `7a82d2ee516a9c1807a7f2ca765118f203a10ff3` on
`2.0`. PR #24 targets the repository's default branch, `main`.
The standalone benchmark harness was removed and the release notes prepared.
Local production CI passed for that initial candidate, but native CI did not.
The fixes below change production code: the initial local pass does not certify
the revised candidate. This is a release checklist, not a claim of certification.

## Failure evidence and disposition

| ID | Failure | Evidence / bounded conclusion | Disposition |
|---|---|---|---|
| R1 | `session_runtime::tests::session_turn_injects_due_focus_reminder_before_the_next_model_request` on macOS latest | A controlled slow rebuild crossed another reminder period and made old code inject twice before one dispatch. The new `session_runtime::tests::reminder_rebuild_crossing_time_boundary_does_not_repeat_before_dispatch` checks time and round reminders together, no initial injection, and rearming on a subsequent round. | Production fix evaluates all reminders once per completed-round dispatch boundary. Revised session-runtime group: 98 passed; enhanced exact regression: 1 passed. Full revised-candidate gates pending. The historical runner pause itself was not captured. |
| R2 | Real-browser readfile scenario on Ubuntu | The old fixture was 34 UTF-8 bytes, but actions requested inclusive byte ends 199 and 99. A strict local negative control confirmed `SelectorNotFound`; merely seeing a running tool label was not success. | Fixture is now 280 UTF-8 bytes (measured from the actual string). Both streaming and nonstreaming scenarios check exactly three successful semantic action finishes and fixture content in the next model input. Both passed locally; the full product suite also passed. Byte selector semantics are unchanged. |
| R3 | Large-tool answer handoff left stale scroll space on macOS 15 | Delaying outline-related ResizeObserver notifications by 500ms reproduces stale space with old code. A local sample also observed transient stale geometry, but the historical runner's cause was not captured. | Known collapse/archive commits now directly invalidate outline geometry. Rebuilt browser acceptance passed all four stream/motion combinations with the original 300ms check and trailing-space limit, under the delayed-notification control. Full revised-candidate gates pending. |
| R4 | `server::tests::manual_toolgen_rejects_bad_source_state_and_duplicate_clicks` on Windows | Original trace reached a model response but remained working beyond the existing 3s deadline. Event-consumption timestamps do not identify which worker or storage stage caused the delay. Later diagnostic passes do not explain the original failure. | **Unresolved release blocker.** No timeout increase, skip, or speculative production fix. |
| R5 | `server::tests::immediate_message_after_core_finalization_starts_a_new_turn` on Windows diagnostic run | The worker did not close its supplement window within the existing 2s deadline; the new-turn assertion was not reached. A later instrumented suite passed. | **Unresolved release blocker.** Do not describe this as a wrong-turn routing failure or as fixed. |

## Native evidence

Links retain the exact run/commit association. Diagnostic branch changes are
not release changes and must not be merged into the product branch.

- Initial push: [37888984828](https://github.com/moliam/TimemAi/actions/runs/37888984828).
  Windows, Ubuntu, and Intel macOS passed; macOS latest failed R1 and macOS 15 failed R3.
- Initial PR: [37888990333](https://github.com/moliam/TimemAi/actions/runs/37888990333).
  Windows failed R4; Ubuntu failed R2; macOS latest and macOS 15 passed.
  Intel macOS later completed successfully (job 113685385002 at
  2026-10-09T06:31:29Z); the run had already failed overall on Windows and
  Ubuntu, so the evidence record is final.
- First isolated diagnostics: [37891103389](https://github.com/moliam/TimemAi/actions/runs/37891103389),
  commit `fac450b21f4ff6b27d1e1fd9e17939a4b2abb1031`.
  Windows host group: 352 passed, 1 failed, 1 ignored (R5).
  Three exact R4 runs passed. Three macOS acceptance runs passed without the
  later controlled notification delay; these do not invalidate R3.
- Storage diagnostics: [37891801014](https://github.com/moliam/TimemAi/actions/runs/37891801014),
  commit `835e7b97f59bca583fc2a292ea1ff82f126352d4`.
  Windows host group: 353 passed, 0 failed, 1 ignored; three exact runs of each
  Windows target passed. Slow atomic-write paths were observed elsewhere, but
  no failing target was captured. The measured write interval includes directory
  creation, writing and sync; it is not an isolated fsync measurement and is not
  proof of either Windows root cause.

## Completion gates

- [ ] Resolve R4 and R5 with attributable evidence, or obtain an explicit release
      risk decision; neither later green runs nor this document silently waive them.
- [x] Finish the initial PR evidence record, including Intel macOS.
- [x] Complete narrow checks and reproducible Web assets for the fixes:
      709 Web tests in 52 files passed; two production builds emitted the same
      main asset `index-DwPT61vT.js`; architecture self-test, module-boundary,
      test-contract, Rust formatting and diff-whitespace checks passed.
      Only the main JavaScript asset and its HTML reference changed; other
      tracked assets retained their hashes. Full production CI remains pending.
- [ ] Freeze a new candidate commit; run complete local `scripts/ci.sh` with no
      concurrent edits or builds.
- [ ] Pass the new candidate's native Windows, Ubuntu, macOS latest, macOS 15,
      and Intel macOS gates. Cross-compilation is not native execution.
- [ ] Merge PR preserving the tested commits; verify release commit ancestry and
      tree. Do not overwrite a divergent local `main` checkout.
- [ ] Publish immutable annotated `v2.4.0` and a non-draft, non-prerelease release
      with major-feature and performance notes, identifying the tested commit.
- [ ] Remove owned temporary diagnostic branch/worktree after preserving evidence.

Do not install or restart the developer's running Timem instance as part of
release verification. Do not reduce existing assertions, raise timing budgets,
or repeat unchanged suites merely to replace red evidence with green evidence.
