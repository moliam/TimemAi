#!/usr/bin/env python3
"""Inspect observed cache usage and replay prompt-cache strategies.

Provider-observed input/cached token counts are paired directly from audit
request/response records. Separately, the tool measures canonical prompt-item
prefix continuity and simulates Claude/Anthropic-style explicit cache marks.
The structural replay uses character counts as a stable local proxy for tokens;
it is diagnostic evidence, not a claim about a provider's internal KV cache.
"""

from __future__ import annotations

import argparse
import json
import re
import statistics
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


def canonical_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def tagged_provider_item(item: dict[str, Any]) -> str:
    item_type = str(item.get("type") or "message")
    role = str(item.get("role") or item_type)
    return f"<provider_item kind={role}>\n{canonical_json(item)}\n</provider_item>"


def extract_prompt(body: Any) -> tuple[str, str] | None:
    if not isinstance(body, dict):
        return None

    system_parts: list[str] = []
    dynamic_parts: list[str] = []

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

    # Tool definitions are part of the cacheable Responses/Chat request prefix.
    # Keep their canonical structure in the static side of the replay instead
    # of silently omitting thousands of real request tokens.
    tools = body.get("tools")
    if isinstance(tools, list) and tools:
        system_parts.append(f"<provider_tools>\n{canonical_json(tools)}\n</provider_tools>")

    input_value = body.get("input")
    if isinstance(input_value, str):
        dynamic_parts.append(input_value)
    elif isinstance(input_value, list):
        for item in input_value:
            if not isinstance(item, dict):
                dynamic_parts.append(canonical_json(item))
                continue
            role = item.get("role")
            item_type = item.get("type")
            text = "\n".join(content_texts(item.get("content")))
            if role == "system" and text:
                system_parts.append(text)
            elif role == "user" and text:
                dynamic_parts.append(text)
            elif role == "assistant" and text:
                # Native assistant messages do not contain prompt-delta markers,
                # but they are still exact provider input and must participate in
                # prefix continuity.
                dynamic_parts.append(tagged_provider_item(item))
            elif item_type in {"function_call", "function_call_output"}:
                dynamic_parts.append(tagged_provider_item(item))
            elif text:
                dynamic_parts.append(tagged_provider_item(item))
            else:
                dynamic_parts.append(tagged_provider_item(item))

    for message in body.get("messages") or []:
        if not isinstance(message, dict):
            continue
        text = "\n".join(content_texts(message.get("content")))
        if not text:
            continue
        if message.get("role") == "system":
            system_parts.append(text)
        elif message.get("role") == "user":
            dynamic_parts.append(text)
        else:
            dynamic_parts.append(tagged_provider_item(message))

    if not system_parts and not dynamic_parts:
        return None
    return "\n".join(system_parts).strip(), "\n".join(dynamic_parts).strip()


PROMPT_SEGMENT_START_RE = re.compile(
    r"(?m)^(?:\[BEGIN DELTA(?:\]|[ \t])|\[BEGIN SEGMENT[ \t]|<prompt_delta[ \t])"
)
INLINE_DELTA_ID_RE = re.compile(r"^\[BEGIN DELTA[ \t]+delta_id:[ \t]*([^]\s]+)")


def prompt_segment_starts(text: str) -> list[int]:
    return [match.start() for match in PROMPT_SEGMENT_START_RE.finditer(text)]


def segment_delta_id(segment: str) -> str | None:
    inline = INLINE_DELTA_ID_RE.match(segment)
    if inline:
        return inline.group(1)
    return segment_field(segment, "delta_id")


def segment_field(segment: str, name: str) -> str | None:
    prefix = name + ":"
    for line in segment.splitlines():
        if line.startswith(prefix):
            value = line[len(prefix) :].strip()
            return value or None
    return None


def segment_prompt_type(segment: str) -> str | None:
    explicit = segment_field(segment, "prompt_type")
    if explicit:
        return explicit
    if (
        "\n## TIMEM_ASSISTANT\n" in segment
        or segment.startswith("## TIMEM_ASSISTANT\n")
        or "\n<ASSISTANT " in segment
        or segment.startswith("<ASSISTANT ")
    ):
        return "llm_response"
    if (
        "\n## USER\n" in segment
        or segment.startswith("## USER\n")
        or "\n<USER>" in segment
        or segment.startswith("<USER>")
    ):
        return "user_question"
    if (
        "\n## ACTIONS\n" in segment
        or segment.startswith("## ACTIONS\n")
        or "\n<RUNTIME>" in segment
        or segment.startswith("<RUNTIME>")
    ):
        return "result_of_llm_action"
    if "\n## SYSTEM\n" in segment or segment.startswith("## SYSTEM\n"):
        return "system"
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
                "delta_id": segment_delta_id(text),
                "prompt_type": segment_prompt_type(text),
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
    prompt_extracted_count = 0
    skipped_no_prompt = 0
    skipped_no_delta_boundary = 0
    total_chars = 0
    read_chars = 0
    created_chars = 0
    cache_marks = 0

    for event in events:
        prompt = extract_prompt(event.get("body"))
        if not prompt:
            skipped_no_prompt += 1
            continue
        prompt_extracted_count += 1
        static_prompt, dynamic_prompt = prompt
        if not prompt_segment_starts(dynamic_prompt):
            skipped_no_delta_boundary += 1
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
            str(event.get("endpoint") or "?"),
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
        "input_requests": len(events),
        "prompt_extracted_requests": prompt_extracted_count,
        "skipped_no_prompt": skipped_no_prompt,
        "skipped_no_delta_boundary": skipped_no_delta_boundary,
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
    seen: set[tuple[Any, ...]] = set()
    for path in paths:
        for event in iter_events(path):
            event_type = event.get("type")
            if event_type not in {"llm_request", "llm_response"}:
                continue
            request_id = event.get("audit_request_id")
            time_ms = event.get("time_ms")
            key = (
                (event_type, request_id, time_ms)
                if request_id is not None or time_ms is not None
                else (event_type, canonical_json(event))
            )
            if key in seen:
                continue
            seen.add(key)
            events.append(event)
    events.sort(key=lambda event: (event.get("time_ms") or 0, event.get("type") or ""))
    return events


def segment_files(directory: Path, recent_segments: int | None) -> list[Path]:
    files: list[Path] = []
    manifest = directory / "manifest.json"
    if manifest.is_file():
        try:
            doc = json.loads(manifest.read_text(errors="replace"))
            for entry in doc.get("segments") or []:
                name = entry.get("file_name") if isinstance(entry, dict) else None
                if isinstance(name, str):
                    candidate = directory / name
                    if candidate.is_file():
                        files.append(candidate)
        except Exception:
            files = []
    if not files:
        files = sorted(directory.glob("segment-*.jsonl"))
    if recent_segments is not None:
        files = files[-max(0, recent_segments) :]
    return files


def expand_audit_path(path: Path, recent_segments: int | None) -> list[Path]:
    if path.is_dir():
        if path.name.endswith(".segments"):
            return segment_files(path, recent_segments)
        out: list[Path] = []
        for candidate in sorted(path.rglob("api_audit.json")) + sorted(
            path.rglob("api_audit.jsonl")
        ):
            out.extend(expand_audit_path(candidate, recent_segments))
        for directory in sorted(path.rglob("api_audit.jsonl.segments")):
            out.extend(segment_files(directory, recent_segments))
        return out
    if not path.is_file():
        # A logical rolling stream can have no physical base file while its
        # sibling segment directory contains the complete stream.
        segment_dir = Path(str(path) + ".segments")
        return segment_files(segment_dir, recent_segments) if segment_dir.is_dir() else []

    out = [path]
    segment_dir = Path(str(path) + ".segments")
    if segment_dir.is_dir():
        out.extend(segment_files(segment_dir, recent_segments))
    elif path.name == "api_audit.json":
        rolling_dir = path.with_name("api_audit.jsonl.segments")
        if rolling_dir.is_dir():
            out.extend(segment_files(rolling_dir, recent_segments))
    return out


def audit_paths(args) -> list[Path]:
    candidates: list[Path] = []
    if args.audit:
        candidates.extend(Path(item).expanduser() for item in args.audit)
    for data_dir in args.data_dir:
        candidates.append(Path(data_dir).expanduser())

    paths: list[Path] = []
    for candidate in candidates:
        paths.extend(expand_audit_path(candidate, args.recent_segments))
    # Preserve manifest/segment order while removing duplicate physical files.
    return list(dict.fromkeys(path.resolve() for path in paths if path.is_file()))


def request_sequence(body: Any) -> list[str]:
    if not isinstance(body, dict):
        return []
    sequence: list[str] = []
    for field in ("instructions", "system", "tools"):
        value = body.get(field)
        if value not in (None, "", []):
            sequence.append(f"{field}:{canonical_json(value)}")
    input_value = body.get("input")
    if isinstance(input_value, list):
        sequence.extend(f"input:{canonical_json(item)}" for item in input_value)
    elif input_value not in (None, ""):
        sequence.append(f"input:{canonical_json(input_value)}")
    messages = body.get("messages")
    if isinstance(messages, list):
        sequence.extend(f"message:{canonical_json(item)}" for item in messages)
    return sequence


IDENTITY_PATTERNS = {
    "session": re.compile(r"Current session_id:\s*([^\s<]+)"),
    "context": re.compile(r"Current context_id:\s*([^\s<]+)"),
    "worker": re.compile(r"Current worker_id:\s*([^\s<]+)"),
}


def request_stream_key(event: dict[str, Any], sequence: list[str]) -> tuple[str, ...]:
    searchable = "\n".join(sequence)
    identity = []
    for name, pattern in IDENTITY_PATTERNS.items():
        match = pattern.search(searchable)
        identity.append(f"{name}={match.group(1) if match else '?'}")
    return (
        str(event.get("endpoint") or "?"),
        str(event.get("model") or event.get("body", {}).get("model") or "?"),
        *identity,
    )


def common_prefix_items(left: list[str], right: list[str]) -> tuple[int, int]:
    count = 0
    chars = 0
    for previous, current in zip(left, right):
        if previous != current:
            break
        count += 1
        chars += len(current)
    return count, chars


def usage_from_response(event: dict[str, Any]) -> tuple[int, int] | None:
    body = event.get("body")
    if not isinstance(body, dict):
        return None
    usage = body.get("usage")
    if not isinstance(usage, dict):
        return None
    input_tokens = usage.get("input_tokens")
    details = usage.get("input_tokens_details")
    cached_tokens = details.get("cached_tokens") if isinstance(details, dict) else None
    if not isinstance(input_tokens, int) or not isinstance(cached_tokens, int):
        return None
    return input_tokens, cached_tokens


def observed_report(events: list[dict[str, Any]]) -> dict[str, Any]:
    requests: dict[str, dict[str, Any]] = {}
    previous_by_stream: dict[tuple[str, ...], list[str]] = {}
    transitions: list[float] = []

    for event in events:
        if event.get("type") != "llm_request" or not isinstance(event.get("body"), dict):
            continue
        request_id = event.get("audit_request_id")
        if not isinstance(request_id, str):
            continue
        sequence = request_sequence(event["body"])
        stream = request_stream_key(event, sequence)
        previous = previous_by_stream.get(stream)
        prefix_items = prefix_chars = 0
        if previous is not None:
            prefix_items, prefix_chars = common_prefix_items(previous, sequence)
            total_chars = sum(len(item) for item in sequence)
            transitions.append(prefix_chars / total_chars if total_chars else 0.0)
        previous_by_stream[stream] = sequence
        row = {
            "id": request_id,
            "time_ms": event.get("time_ms") or 0,
            "sequence_items": len(sequence),
            "prefix_items": prefix_items,
            "prefix_chars": prefix_chars,
        }
        requests[request_id] = row

    paired = []
    for event in events:
        if event.get("type") != "llm_response":
            continue
        request_id = event.get("audit_request_id")
        row = requests.get(request_id)
        usage = usage_from_response(event)
        if row is None or usage is None:
            continue
        input_tokens, cached_tokens = usage
        paired.append(
            {
                **row,
                "input_tokens": input_tokens,
                "cached_tokens": cached_tokens,
                "hit_rate": cached_tokens / input_tokens if input_tokens else 0.0,
            }
        )

    total_input = sum(row["input_tokens"] for row in paired)
    total_cached = sum(row["cached_tokens"] for row in paired)
    rates = [row["hit_rate"] for row in paired if row["input_tokens"] > 0]
    low = [row for row in paired if row["input_tokens"] > 0 and row["hit_rate"] < 0.90]
    return {
        "pairs": len(paired),
        "total_input": total_input,
        "total_cached": total_cached,
        "aggregate_rate": total_cached / total_input if total_input else 0.0,
        "median_rate": statistics.median(rates) if rates else 0.0,
        "below_90": len(low),
        "transitions": len(transitions),
        "median_prefix_rate": statistics.median(transitions) if transitions else 0.0,
        "low_examples": sorted(low, key=lambda row: (row["hit_rate"], -row["input_tokens"]))[:8],
    }


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
        default=None,
        help="Data directory to scan recursively. Default: data",
    )
    parser.add_argument("--audit", action="append", help="Specific api_audit file")
    parser.add_argument(
        "--recent-segments",
        type=int,
        help="For rolling audit directories, read only the newest N segments.",
    )
    parser.add_argument("--observed-only", action="store_true")
    parser.add_argument("--max-threshold", type=int, default=12)
    parser.add_argument("--max-checkpoints", type=int, default=3)
    parser.add_argument("--max-tail-blocks", type=int, default=4)
    args = parser.parse_args()

    if args.data_dir is None:
        args.data_dir = ["data"]

    paths = audit_paths(args)
    events = load_events(paths)
    requests = [
        event
        for event in events
        if event.get("type") == "llm_request" and isinstance(event.get("body"), dict)
    ]
    observed = observed_report(events)
    print(f"audit_files: {len(paths)}")
    print(f"llm_requests: {len(requests)}")
    print(f"observed_usage_pairs: {observed['pairs']}")
    print(
        "observed_cache: "
        f"aggregate={pct(observed['aggregate_rate'])} "
        f"median_request={pct(observed['median_rate'])} "
        f"below_90={observed['below_90']}"
    )
    print(
        "canonical_prompt_prefix: "
        f"transitions={observed['transitions']} "
        f"median_shared_chars={pct(observed['median_prefix_rate'])}"
    )
    if observed["low_examples"]:
        print("observed_low_examples:")
        for row in observed["low_examples"]:
            print(
                f"  time_ms={row['time_ms']} input={row['input_tokens']} "
                f"cached={row['cached_tokens']} hit={pct(row['hit_rate'])} "
                f"canonical_prefix_items={row['prefix_items']}/{row['sequence_items']}"
            )
    if args.observed_only:
        return 0
    print()
    print(
        "simulation_note: local character-prefix model; not provider-observed KV-cache usage"
    )
    print("strategy        threshold  ckpt  requests  hit_rate  create_rate  avg_marks")
    print("--------------  ---------  ----  --------  --------  -----------  ---------")

    results = []
    coverage = simulate(requests, "static")
    print(
        "simulation_coverage: "
        f"supported={coverage['requests']}/{coverage['input_requests']} "
        f"prompt_extracted={coverage['prompt_extracted_requests']} "
        f"skipped_no_prompt={coverage['skipped_no_prompt']} "
        f"skipped_no_delta_boundary={coverage['skipped_no_delta_boundary']}"
    )
    for strategy in ("static", "legacy"):
        result = coverage if strategy == "static" else simulate(requests, strategy)
        results.append((result["score"], strategy, "-", "-", result))
        print_row(strategy, "-", "-", result)

    best = None
    for checkpoints in range(1, args.max_checkpoints + 1):
        for threshold in range(1, args.max_threshold + 1):
            result = simulate(
                requests,
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
            result = simulate(requests, strategy, tail_blocks=tail_blocks)
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
