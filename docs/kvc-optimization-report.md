# KVC Optimization Report

Date: 2026-07-03

Scope: `kvc_optimize` branch after rebasing onto `v0.7.1`.

## Goal

Improve prompt KV-cache reuse for growing Timem conversations without returning
to the old "cache all old deltas as one block" behavior. The old block changes
every round, so provider prefix caches tend to create new cache entries instead
of reading existing ones.

## Simulator

The replay tool is:

`python3 scripts/kvc_replay.py --data-dir data`

It reads local `api_audit` files and replays `llm_request` events. The simulator
models Claude/Anthropic-style cache control as a prefix cache:

- A cache mark stores or reads the prompt prefix ending at that marked block.
- Each cache mark can look backward over a bounded 20-block window for a
  previously written prefix.
- The largest matching prefix in a request is counted as `read`.
- `created` is estimated as the newly cached suffix beyond the best read prefix.
- Character counts are used as a stable local proxy for token counts.

This is intentionally stricter than the earlier block-hash test. It catches the
important provider behavior: cache boundaries must remain stable as the prompt
grows. It also avoids the earlier overly pessimistic creation estimate that
counted a whole marked prefix as newly created.

## Replay Data

Current local replay input:

- Audit files scanned: 5
- LLM requests replayed: 1158

The replay includes historical local shell usage from `.test_mem` plus smoke
spaces. It does not send network requests and does not print prompt contents.

## Explored Strategies

The replay compares these strategy families:

- `static`: cache only the static prompt.
- `legacy`: cache static prompt plus one ever-growing old-deltas block.
- `checkpoint`: cache static prompt plus stable assistant-response checkpoints.
- `tail`: cache static prompt plus the latest N dynamic prompt slices.
- `tail_no_static`: cache only latest N dynamic prompt slices.
- `user_tail`: cache static prompt plus latest N `user_question` slices.
- `action_tail`: cache static prompt plus latest N `result_of_llm_action` slices.

The `tail` family is closest to provider automatic caching semantics for
append-only conversations: keep the cache breakpoint close to the newest prompt
tail and rely on provider lookback to find the previous cached prefix.

## Results

Summary table from the current local audit replay:

| Strategy | Setting | Hit rate | Create rate | Score hit-create | Avg cache marks |
|---|---:|---:|---:|---:|---:|
| static only | - | 30.8% | 1.3% | 29.5% | 1.00 |
| legacy old_deltas block | - | 31.0% | 66.2% | -35.2% | 1.90 |
| checkpoint | threshold=1, ckpt=2 | 69.0% | 3.7% | 65.3% | 2.20 |
| user_tail | tail=2 | 70.7% | 4.2% | 66.5% | 2.71 |
| action_tail | tail=2 | 93.2% | 5.9% | 87.3% | 2.58 |
| tail | tail=1 | 90.9% | 9.1% | 81.8% | 2.00 |
| tail | tail=2 | 92.2% | 7.8% | 84.3% | 2.90 |
| tail | tail=3 | 94.0% | 6.0% | 88.1% | 3.78 |
| tail | tail=4 | 94.0% | 6.0% | 88.1% | 4.61 |

`tail=3` and `tail=4` tie on hit/create score, but `tail=3` is selected because
it uses fewer cache marks and keeps the total explicit breakpoints to
`1 static + 3 dynamic = 4`. This fits common provider breakpoint limits while
capturing almost all benefit in the local replay.

Compared with the previous best stable-checkpoint strategy, `tail=3` improves
simulated prefix-cache hit rate from 69.0% to 94.0%, while creation rises from
3.7% to 6.0%. The score `hit_rate - create_rate` improves from 65.3% to 88.0%,
or +22.7 percentage points. Compared with static-only caching, hit rate improves
by +63.2 percentage points.

## Selected Algorithm

Current selected parameters:

- `DYNAMIC_TAIL_CACHE_BLOCKS = 3`

Runtime cache planning:

1. Always mark the static prompt cacheable.
2. Split dynamic prompt context into rendered prompt-delta slices.
3. Mark the latest three dynamic prompt slices cacheable.
4. Leave older dynamic slices uncached.

This intentionally allows the newest prompt delta to be cacheable. In an
append-only prompt, the previous tail remains present in the next request, so
provider lookback can reuse the previous cached prefix while the newest tail
writes the next cache boundary.

## Known Limits

- The replay uses local audit data, not live provider billing data.
- Character counts approximate token counts.
- Provider TTL behavior is not modeled because current audit records do not
  reliably include enough wall-clock timing metadata for cache expiry.
- Real provider cache behavior should still be monitored through `⌁` read and
  `✚` creation counters after release.

## Verification

The branch includes:

- Prefix-cache simulation tests, not only block-hash tests.
- A replay script for local audit data.
- A CI replay fixture test that verifies explicit `--data-dir` does not
  accidentally scan local `data/`.
- Documentation of the selected threshold and tradeoff.
