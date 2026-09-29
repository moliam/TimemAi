# TimemAi 2.3.3

TimemAi 2.3.3 adds user-initiated context compaction with a live
"compacting" experience, polishes the stream UI, and hardens protocol
parsing against stringified array arguments.

## Manual context compaction

- The ctx menu next to the header usage meter is now a hand icon with a
  dropdown: Compact and Clear.
- Compacting works while the model is busy: a busy turn queues a mailbox
  marker that forces the next model dispatch after a short (10s)
  dedicated timeout; an idle session starts a direct-resume turn whose
  first request leads with the compaction.
- The manual request reuses the forced-shrink machinery but the trailer
  says "User manually requests context compaction. Please compact
  context before further work." The wording persists across retries
  until the compaction succeeds.
- Live UI: an indeterminate "Context compacting..." notice appears at
  click; the completed notice supersedes it (and a turn ending without
  completion retires its own requested notice). The ctx menu shows
  "Compacting..." while pending to prevent double submits.

## Stream UI

- The working trailer shows the model request count, e.g. `9m42s (✦ 23)`,
  and a waiting-model indicator from the turn projection.
- Running tool rows show a live per-second elapsed label.
- Memo notices distinguish created vs updated lifecycle ops correctly
  (an existing memo replaced by create/update now reports "updated").

## Fixes

- Parse stringified string-list arguments (e.g. run_bash `edit` passed as
  `"[\"/a/b.py\"]"`) as JSON so brackets never leak into first-touch
  reminder paths; the comma fallback also trims stray brackets/quotes.
- Cross-platform CI: timing-sensitive tests gated/relaxed appropriately
  and first-touch expectations mirror platform path forms.

## Upgrade notes

- No breaking changes. Restart the `timem` process after upgrading so the
  embedded Web build is served from the new release.
