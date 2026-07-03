#!/usr/bin/env python3
"""Replay local api_audit data against prompt-cache strategies.

The simulator models Claude/Anthropic-style prompt caching as a prefix cache:
a cache mark writes the prompt prefix ending at that block, and later requests
can look backward from each marked block to find a recently written prefix.
It reports character counts as a stable local proxy for tokens.
"""

from __future__ import annotations

import argparse
import json
from collections import defaultdict
from pathlib import Path
from typing import Any

LOOKBACK_BLOCKS = 20


def iter_events(path: Path):
    text = path.read_text(errors="replace")
    if not text.strip():
        return
    try:
        doc = json.loads(text)
        if isinstance(doc, dict) and isinstance(doc.get("events"), list):
            yield from doc["events"]
            return
    except Exception:
        pass

    for line in text.splitlines():
        if not line.strip():
            continue
        try:
            yield json.loads(line)
        except Exception:
            continue


def content_texts(content: Any) -> list[str]:
    if isinstance(content, str):
        return [content]
    if not isinstance(content, list):
        return []
    out = []
    for item in content:
        if not isinstance(item, dict):
            continue
        text = item.get("text") or item.get("content")
        if isinstance(text, str):
            out.append(text)
    return out


def extract_prompt(body: Any) -> tuple[str, str] | None:
    if not isinstance(body, dict):
        return None

    system_parts: list[str] = []
    user_parts: list[str] = []

    system = body.get("system")
    if isinstance(system, str):
        system_parts.append(system)
    elif isinstance(system, list):
        for item in system:
            if not isinstance(item, dict):
                continue
            text = item.get("text") or item.get("content")
            if isinstance(text, str):
                system_parts.append(text)

    instructions = body.get("instructions")
    if isinstance(instructions, str):
        system_parts.append(instructions)

    input_text = body.get("input")
    if isinstance(input_text, str):
        user_parts.append(input_text)

    for message in body.get("messages") or []:
        if not isinstance(message, dict):
            continue
        text = "\n".join(content_texts(message.get("content")))
        if not text:
            continue
        if message.get("role") == "system":
            system_parts.append(text)
        elif message.get("role") == "user":
            user_parts.append(text)

    if not system_parts and not user_parts:
        return None
    return "\n".join(system_parts).strip(), "\n".join(user_parts).strip()


def prompt_segment_starts(text: str) -> list[int]:
    starts: list[int] = []
    if text.startswith("[BEGIN SEGMENT "):
        starts.append(0)
    offset = 0
    marker = "\n[BEGIN SEGMENT "
    while True:
        idx = text.find(marker, offset)
        if idx < 0:
            break
        starts.append(idx + 1)
        offset = idx + 2
    return starts


def segment_field(segment: str, name: str) -> str | None:
    prefix = name + ":"
    for line in segment.splitlines():
        if line.startswith(prefix):
            value = line[len(prefix) :].strip()
            return value or None
    return None


def split_segments(dynamic_prompt: str) -> list[dict[str, str | None]]:
    dynamic_prompt = dynamic_prompt.strip()
    starts = prompt_segment_starts(dynamic_prompt)
    if not starts:
        return (
            [{"text": dynamic_prompt, "delta_id": None, "prompt_type": None}]
            if dynamic_prompt
            else []
        )

    segments = []
    for idx, start in enumerate(starts):
        end = starts[idx + 1] if idx + 1 < len(starts) else len(dynamic_prompt)
        text = dynamic_prompt[start:end].strip()
        segments.append(
            {
                "text": text,
                "delta_id": segment_field(text, "delta_id"),
                "prompt_type": segment_field(text, "prompt_type"),
            }
        )
    return segments


def plan_prefixes(blocks: list[tuple[str, str]], cache_indexes: set[int]):
    prefix = ""
    prefixes: list[tuple[str, int]] = []
    total_chars = 0
    for idx, (role, text) in enumerate(blocks):
        part = f"\n<{role}>\n{text}"
        prefix += part
        total_chars += len(part)
        prefixes.append((prefix, total_chars))
    return prefixes, cache_indexes, total_chars


def plan_static(static_prompt: str, dynamic_prompt: str):
    segments = split_segments(dynamic_prompt)
    blocks = [("system", static_prompt)] + [
        ("user", str(segment["text"])) for segment in segments if segment["text"]
    ]
    return plan_prefixes(blocks, {0})


def plan_legacy(static_prompt: str, dynamic_prompt: str):
    segments = split_segments(dynamic_prompt)
    if not segments:
        return plan_prefixes([("system", static_prompt)], {0})

    last_delta_id = segments[-1]["delta_id"]
    cut = len(segments) - 1
    if last_delta_id:
        for idx, segment in enumerate(segments):
            if segment["delta_id"] == last_delta_id:
                cut = idx
                break

    old_deltas = "\n".join(str(segment["text"]) for segment in segments[:cut])
    new_delta = "\n".join(str(segment["text"]) for segment in segments[cut:])
    blocks = [("system", static_prompt)]
    if old_deltas.strip():
        blocks.append(("user", old_deltas))
    if new_delta.strip():
        blocks.append(("user", new_delta))
    cache_indexes = {0}
    if old_deltas.strip():
        cache_indexes.add(1)
    return plan_prefixes(blocks, cache_indexes)


def plan_checkpoint(
    static_prompt: str,
    dynamic_prompt: str,
    threshold: int,
    dynamic_checkpoints: int,
):
    segments = split_segments(dynamic_prompt)
    blocks = [("system", static_prompt)] + [
        ("user", str(segment["text"])) for segment in segments if segment["text"]
    ]
    cache_indexes = {0}

    last_delta_id = segments[-1]["delta_id"] if segments else None
    stable_assistant_indexes = [
        idx
        for idx, segment in enumerate(segments)
        if segment["prompt_type"] == "llm_response"
        and segment["delta_id"] != last_delta_id
    ]
    if stable_assistant_indexes:
        b_ordinal = ((len(stable_assistant_indexes) - 1) // threshold) * threshold
        for checkpoint_offset in reversed(range(dynamic_checkpoints)):
            offset = checkpoint_offset * threshold
            if b_ordinal >= offset:
                segment_idx = stable_assistant_indexes[b_ordinal - offset]
                cache_indexes.add(1 + segment_idx)

    return plan_prefixes(blocks, cache_indexes)


def plan_tail(
    static_prompt: str,
    dynamic_prompt: str,
    tail_blocks: int,
    include_static: bool = True,
):
    segments = split_segments(dynamic_prompt)
    blocks = [("system", static_prompt)] + [
        ("user", str(segment["text"])) for segment in segments if segment["text"]
    ]
    cache_indexes = {0} if include_static else set()
    if len(blocks) > 1 and tail_blocks > 0:
        first_tail = max(1, len(blocks) - tail_blocks)
        cache_indexes.update(range(first_tail, len(blocks)))
    return plan_prefixes(blocks, cache_indexes)


def plan_type_tail(
    static_prompt: str,
    dynamic_prompt: str,
    prompt_type: str,
    tail_count: int,
):
    segments = split_segments(dynamic_prompt)
    blocks = [("system", static_prompt)] + [
        ("user", str(segment["text"])) for segment in segments if segment["text"]
    ]
    cache_indexes = {0}
    indexes = [
        idx
        for idx, segment in enumerate(segments)
        if segment["prompt_type"] == prompt_type
    ]
    for segment_idx in indexes[-tail_count:]:
        cache_indexes.add(1 + segment_idx)
    return plan_prefixes(blocks, cache_indexes)


def replay_cache_marks(prefixes, cache_indexes, store: set[str]):
    """Return read/create chars for one request and update the simulated store.

    Claude prompt caching writes only at cache breakpoints. On a later request,
    each breakpoint can look backward over a bounded number of earlier blocks
    for a prefix that was written by a previous request. Creation is estimated
    as the newly cached suffix after the best read prefix in this request.
    """
    read = 0
    write_end = 0
    for idx in sorted(cache_indexes):
        if idx >= len(prefixes):
            continue
        lookback_start = max(0, idx - LOOKBACK_BLOCKS + 1)
        best_hit = 0
        for probe_idx in range(idx, lookback_start - 1, -1):
            key, prefix_chars = prefixes[probe_idx]
            if key in store:
                best_hit = prefix_chars
                break
        read = max(read, best_hit)
        key, prefix_chars = prefixes[idx]
        if key not in store:
            write_end = max(write_end, prefix_chars)

    created = max(0, write_end - read)
    for idx in cache_indexes:
        if idx < len(prefixes):
            store.add(prefixes[idx][0])
    return read, created


def simulate(
    events,
    strategy: str,
    threshold: int = 2,
    dynamic_checkpoints: int = 2,
    tail_blocks: int = 1,
):
    stores: dict[tuple[str, str], set[str]] = defaultdict(set)
    request_count = 0
    total_chars = 0
    read_chars = 0
    created_chars = 0
    cache_marks = 0

    for event in events:
        prompt = extract_prompt(event.get("body"))
        if not prompt:
            continue
        static_prompt, dynamic_prompt = prompt
        if "[BEGIN SEGMENT " not in dynamic_prompt:
            continue

        if strategy == "static":
            prefixes, cache_indexes, prompt_chars = plan_static(static_prompt, dynamic_prompt)
        elif strategy == "legacy":
            prefixes, cache_indexes, prompt_chars = plan_legacy(static_prompt, dynamic_prompt)
        elif strategy == "checkpoint":
            prefixes, cache_indexes, prompt_chars = plan_checkpoint(
                static_prompt, dynamic_prompt, threshold, dynamic_checkpoints
            )
        elif strategy == "tail":
            prefixes, cache_indexes, prompt_chars = plan_tail(
                static_prompt, dynamic_prompt, tail_blocks
            )
        elif strategy == "tail_no_static":
            prefixes, cache_indexes, prompt_chars = plan_tail(
                static_prompt, dynamic_prompt, tail_blocks, include_static=False
            )
        elif strategy == "user_tail":
            prefixes, cache_indexes, prompt_chars = plan_type_tail(
                static_prompt, dynamic_prompt, "user_question", tail_blocks
            )
        elif strategy == "action_tail":
            prefixes, cache_indexes, prompt_chars = plan_type_tail(
                static_prompt, dynamic_prompt, "result_of_llm_action", tail_blocks
            )
        else:
            raise ValueError(strategy)

        store_key = (
            str(event.get("provider") or "?"),
            str(event.get("model") or event.get("body", {}).get("model") or "?"),
        )
        store = stores[store_key]
        hit, created = replay_cache_marks(prefixes, cache_indexes, store)

        request_count += 1
        total_chars += prompt_chars
        read_chars += hit
        created_chars += created
        cache_marks += len(cache_indexes)

    hit_rate = read_chars / total_chars if total_chars else 0.0
    create_rate = created_chars / total_chars if total_chars else 0.0
    avg_cache_marks = cache_marks / request_count if request_count else 0.0
    return {
        "requests": request_count,
        "total_chars": total_chars,
        "read_chars": read_chars,
        "created_chars": created_chars,
        "hit_rate": hit_rate,
        "create_rate": create_rate,
        "avg_cache_marks": avg_cache_marks,
        "score": hit_rate - create_rate,
    }


def load_events(paths: list[Path]):
    events = []
    for path in paths:
        for event in iter_events(path):
            if event.get("type") == "llm_request" and isinstance(event.get("body"), dict):
                events.append(event)
    return events


def audit_paths(args) -> list[Path]:
    paths: list[Path] = []
    if args.audit:
        paths.extend(Path(item).expanduser() for item in args.audit)
    for data_dir in args.data_dir:
        root = Path(data_dir).expanduser()
        if root.exists():
            paths.extend(root.rglob("*api_audit*"))
    return sorted({path for path in paths if path.is_file()})


def pct(value: float) -> str:
    return f"{value * 100:5.1f}%"


def print_row(name: str, threshold, checkpoints, result):
    print(
        f"{name:<14}  {str(threshold):>9}  {str(checkpoints):>4}  "
        f"{result['requests']:>8}  {pct(result['hit_rate'])}  "
        f"{pct(result['create_rate'])}  {result['avg_cache_marks']:>9.2f}"
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--data-dir",
        action="append",
        default=["data"],
        help="Data directory to scan recursively. Default: data",
    )
    parser.add_argument("--audit", action="append", help="Specific api_audit file")
    parser.add_argument("--max-threshold", type=int, default=12)
    parser.add_argument("--max-checkpoints", type=int, default=3)
    parser.add_argument("--max-tail-blocks", type=int, default=4)
    args = parser.parse_args()

    paths = audit_paths(args)
    events = load_events(paths)
    print(f"audit_files: {len(paths)}")
    print(f"llm_requests: {len(events)}")
    print()
    print("strategy        threshold  ckpt  requests  hit_rate  create_rate  avg_marks")
    print("--------------  ---------  ----  --------  --------  -----------  ---------")

    results = []
    for strategy in ("static", "legacy"):
        result = simulate(events, strategy)
        results.append((result["score"], strategy, "-", "-", result))
        print_row(strategy, "-", "-", result)

    best = None
    for checkpoints in range(1, args.max_checkpoints + 1):
        for threshold in range(1, args.max_threshold + 1):
            result = simulate(
                events,
                "checkpoint",
                threshold=threshold,
                dynamic_checkpoints=checkpoints,
            )
            score = result["hit_rate"] - result["create_rate"]
            if best is None or score > best[0]:
                best = (score, threshold, checkpoints, result)
            results.append((score, "checkpoint", threshold, checkpoints, result))
            print_row("checkpoint", threshold, checkpoints, result)

    for strategy in ("tail", "tail_no_static", "user_tail", "action_tail"):
        for tail_blocks in range(1, args.max_tail_blocks + 1):
            result = simulate(events, strategy, tail_blocks=tail_blocks)
            score = result["score"]
            results.append((score, strategy, f"tail={tail_blocks}", "-", result))
            print_row(strategy, f"tail={tail_blocks}", "-", result)

    if best:
        _, threshold, checkpoints, result = best
        print()
        print(
            "best_checkpoint_by_hit_minus_create: "
            f"threshold={threshold}, checkpoints={checkpoints}, "
            f"hit_rate={pct(result['hit_rate'])}, "
            f"create_rate={pct(result['create_rate'])}"
        )
    print()
    print("top_by_hit_minus_create:")
    for score, strategy, threshold, checkpoints, result in sorted(
        results, key=lambda item: item[0], reverse=True
    )[:8]:
        print(
            f"{strategy:<14} threshold={threshold} checkpoints={checkpoints} "
            f"score={pct(score)} hit={pct(result['hit_rate'])} "
            f"create={pct(result['create_rate'])} marks={result['avg_cache_marks']:.2f}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
