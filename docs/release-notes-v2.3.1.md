# TimemAi 2.3.1

TimemAi 2.3.1 refines the memo reminder semantics, fixes the WebUI memo
tooltip visibility, and tightens the periodic reasoning review trigger.

## Memo reminder refinements

- The finish guard no longer blocks after a same-turn memo delete: the
  deletion instead triggers a one-shot re-verify trailer on the next model
  request ("You just deleted the memo ..."), quoting the deleted memo text
  and requiring re-verification of the final goal before `task_finished`.
- The trailer is attached on every prompt build path (including the
  running-job snapshot early-return path) and is inserted before the
  response protocol trailer so the protocol instruction stays last.
- Per-round memo restating in prompts was removed in favor of the one-shot
  trailer; the memo create/delete tool replies carry continuous-work
  guidance without runtime internals, and the memo manifest description
  anchors on goal semantics.
- Same-turn delete-then-finish is challenged once before the task finish is
  accepted, with wording aligned across the guard helper.

## Web fixes

- The working indicator now fuses the memo pin into the spinning arc with
  instant hover content: the memo tooltip renders through a portal on
  `document.body` with fixed positioning and viewport-adaptive placement
  (prefers left, flips right beside the sidebar, clamps vertically), so it
  always floats above the chat area and session sidebar instead of being
  clipped or covered.
- The debug statistics page refreshes in place, keeping the browser find
  bar open.

## Reasoning review

- The periodic reasoning review threshold now counts native exchanges, with
  message element counting maintained incrementally at write sites and
  recounted only at replacement points, replacing full scans.

## Upgrade notes

- No breaking changes. Restart the `timem` process after upgrading so the
  embedded Web build is served from the new release.
