/**
 * Session Insights dashboard.
 *
 * Scope:
 * - the header owns a compact chip that shows the user's top-priority item;
 * - clicking the chip opens a draggable floating panel (portal to body);
 * - items are reordered by dragging a row, and the order is persisted locally;
 * - CPU/memory samples come from the Host resource query and degrade to an
 *   explicit unavailable state when the platform cannot account for them.
 *
 * Constraints:
 * - the component owns no Session lifecycle state and does not invent usage;
 * - persisted values are presentation preferences only (order, position, pin);
 * - every sample command is read-only and bounded by the Host.
 */
import { useEffect, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent, ReactNode } from "react";
import { createPortal } from "react-dom";
import {
  Activity,
  ArrowBigDown,
  ArrowBigUp,
  ChevronDown,
  Cpu,
  Gauge,
  GripVertical,
  MemoryStick,
  Pin,
  PinOff,
  RotateCcw,
  Sparkles,
  Wrench,
} from "lucide-react";
import { t, useT } from "./i18n";
import type { Session } from "./protocol";
import { formatTokensCoarse } from "./token_format";
import {
  sessionCacheHitPercent,
  sessionContextUsage,
  sessionModelPulse,
  sessionModelTelemetry,
  sessionRuntimeUsage,
} from "./view_model";

export const INSIGHT_ORDER_STORAGE_KEY = "timem.sessionInsights.order.v1";
export const INSIGHT_POSITION_STORAGE_KEY = "timem.sessionInsights.position.v1";
export const INSIGHT_PINNED_STORAGE_KEY = "timem.sessionInsights.pinned.v1";

export const INSIGHT_ITEM_IDS = [
  "speed",
  "ttft",
  "cache",
  "context",
  "flow",
  "calls",
  "cpu",
  "mem",
] as const;
export type InsightItemId = (typeof INSIGHT_ITEM_IDS)[number];

export const DEFAULT_INSIGHT_ORDER: InsightItemId[] = [
  "speed",
  "ttft",
  "cache",
  "context",
  "flow",
  "calls",
  "cpu",
  "mem",
];

export type SessionResourcePulseSample = {
  at: number;
  cpuUsageUsec?: number;
  quotaCores?: number;
  memBytes?: number;
  swapBytes?: number;
  memHighBytes?: number;
  memMaxBytes?: number;
};

export type InsightPanelPosition = { left: number; top: number };

const PANEL_WIDTH = 376;
const PANEL_MARGIN = 8;
const RESOURCE_WINDOW_SAMPLES = 10;
const RESOURCE_SAMPLE_INTERVAL_MS = 2000;

export function normalizeInsightOrder(raw: unknown): InsightItemId[] {
  const known = new Set<string>(INSIGHT_ITEM_IDS);
  const seen = new Set<string>();
  const order: InsightItemId[] = [];
  if (Array.isArray(raw)) {
    for (const item of raw) {
      if (typeof item !== "string" || !known.has(item) || seen.has(item)) continue;
      seen.add(item);
      order.push(item as InsightItemId);
    }
  }
  for (const item of DEFAULT_INSIGHT_ORDER) {
    if (!seen.has(item)) order.push(item);
  }
  return order;
}

export function moveInsightItem(
  order: InsightItemId[],
  from: number,
  to: number,
): InsightItemId[] {
  if (from < 0 || from >= order.length || to < 0 || to >= order.length || from === to) {
    return order;
  }
  const next = [...order];
  const [moved] = next.splice(from, 1);
  next.splice(to, 0, moved);
  return next;
}

export function sparklinePoints(
  values: number[],
  width: number,
  height: number,
  padding = 1.5,
): string {
  if (values.length === 0) return "";
  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min;
  const innerWidth = Math.max(1, width - padding * 2);
  const innerHeight = Math.max(1, height - padding * 2);
  const step = values.length === 1 ? 0 : innerWidth / (values.length - 1);
  return values
    .map((value, index) => {
      const x = values.length === 1 ? width / 2 : padding + step * index;
      const y = span === 0 ? height / 2 : height - padding - ((value - min) / span) * innerHeight;
      return `${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(" ");
}

/** CPU cores consumed between consecutive samples. Missing or reset counters
 * are skipped instead of being rendered as zero load. */
export function resourceCpuCores(
  samples: SessionResourcePulseSample[] | undefined,
): number[] {
  if (!samples || samples.length < 2) return [];
  const cores: number[] = [];
  for (let index = 1; index < samples.length; index += 1) {
    const previous = samples[index - 1];
    const current = samples[index];
    const elapsedMs = current.at - previous.at;
    if (
      previous.cpuUsageUsec === undefined ||
      current.cpuUsageUsec === undefined ||
      elapsedMs <= 0 ||
      current.cpuUsageUsec < previous.cpuUsageUsec
    ) {
      continue;
    }
    cores.push((current.cpuUsageUsec - previous.cpuUsageUsec) / 1000 / elapsedMs);
  }
  return cores;
}

export function resourceMemorySeries(
  samples: SessionResourcePulseSample[] | undefined,
): number[] {
  if (!samples) return [];
  return samples
    .map((sample) => sample.memBytes)
    .filter((value): value is number => value !== undefined);
}

/** CPU quota attached to the latest sample; undefined when the platform
 * cannot account for a quota. */
export function resourceCpuCapCores(
  sample: SessionResourcePulseSample | undefined,
): number | undefined {
  return sample?.quotaCores;
}

/** Memory cap for the latest sample. The configured hard cap (memory.max) is
 * the user-visible limit; a platform that exposes only the throttle point
 * falls back to memory.high instead of claiming a limit that does not exist. */
export function resourceMemoryCapBytes(
  sample: SessionResourcePulseSample | undefined,
): number | undefined {
  return sample?.memMaxBytes ?? sample?.memHighBytes;
}

export function clampInsightPanelPosition(
  position: InsightPanelPosition,
  panelHeight: number,
  viewportWidth: number,
  viewportHeight: number,
): InsightPanelPosition {
  const maxLeft = Math.max(PANEL_MARGIN, viewportWidth - PANEL_WIDTH - PANEL_MARGIN);
  const maxTop = Math.max(PANEL_MARGIN, viewportHeight - panelHeight - PANEL_MARGIN);
  return {
    left: Math.min(Math.max(PANEL_MARGIN, position.left), maxLeft),
    top: Math.min(Math.max(PANEL_MARGIN, position.top), maxTop),
  };
}

export function parseInsightPanelPosition(raw: string | null): InsightPanelPosition | null {
  if (!raw) return null;
  try {
    const value = JSON.parse(raw) as { left?: unknown; top?: unknown };
    if (
      typeof value.left === "number" &&
      typeof value.top === "number" &&
      Number.isFinite(value.left) &&
      Number.isFinite(value.top)
    ) {
      return { left: value.left, top: value.top };
    }
  } catch {
    // Corrupt preference values fall back to the default anchor.
  }
  return null;
}

function readStorage(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function writeStorage(key: string, value: string | null) {
  try {
    if (value === null) window.localStorage.removeItem(key);
    else window.localStorage.setItem(key, value);
  } catch {
    // Presentation preferences are best effort; private mode must not break UI.
  }
}

function formatGiB(bytes: number): string {
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)}G`;
}

function defaultPanelPosition(chip: HTMLElement | null): InsightPanelPosition {
  const rect = chip?.getBoundingClientRect();
  const width = typeof window === "undefined" ? 1280 : window.innerWidth;
  const anchor = { left: rect?.left ?? 16, top: (rect?.bottom ?? 52) + PANEL_MARGIN };
  return clampInsightPanelPosition(anchor, 360, width, window.innerHeight);
}

function Sparkline({
  values,
  tone,
}: {
  values: number[];
  tone: string;
}) {
  if (values.length < 2) return null;
  const width = 58;
  const height = 14;
  return (
    <svg
      className={`session-insight-sparkline ${tone}`}
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      aria-hidden="true"
    >
      <polyline points={sparklinePoints(values, width, height)} fill="none" strokeWidth="1.2" />
    </svg>
  );
}

export type SessionInsightsProps = {
  session: Session | undefined;
  resourceSamples: SessionResourcePulseSample[] | undefined;
  onReset: (() => void) | null;
  /** Requests one read-only resource sample for the active session. */
  onSample: (() => void) | null;
};

export function SessionInsights({
  session,
  resourceSamples,
  onReset,
  onSample,
}: SessionInsightsProps) {
  useT();
  const [open, setOpen] = useState(false);
  const [pinned, setPinned] = useState(
    () => readStorage(INSIGHT_PINNED_STORAGE_KEY) === "1",
  );
  const [order, setOrder] = useState<InsightItemId[]>(() =>
    normalizeInsightOrder(safeOrder()),
  );
  const [position, setPosition] = useState<InsightPanelPosition | null>(() =>
    parseInsightPanelPosition(readStorage(INSIGHT_POSITION_STORAGE_KEY)),
  );
  const [draggingItemId, setDraggingItemId] = useState<InsightItemId | null>(null);
  const [dropTargetId, setDropTargetId] = useState<InsightItemId | null>(null);
  const chipRef = useRef<HTMLButtonElement | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<{
    pointerId: number;
    offsetX: number;
    offsetY: number;
  } | null>(null);
  const positionRef = useRef<InsightPanelPosition | null>(position);
  positionRef.current = position;
  const sampleRef = useRef(onSample);
  sampleRef.current = onSample;

  useEffect(() => {
    if (!session) return;
    const tick = () => {
      if (document.visibilityState !== "hidden") sampleRef.current?.();
    };
    tick();
    const timer = window.setInterval(tick, RESOURCE_SAMPLE_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [session?.session_id]);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (panelRef.current?.contains(target) || chipRef.current?.contains(target)) return;
      if (!pinned) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open, pinned]);

  const runtimeTotals = session ? sessionRuntimeUsage(session) : undefined;
  const telemetry = session ? sessionModelTelemetry(session) : undefined;
  const pulse = session ? sessionModelPulse(session) : undefined;
  const cacheHitPercent = session ? sessionCacheHitPercent(session) : undefined;
  const contextUsage = session ? sessionContextUsage(session) : undefined;
  const contextLimit = session?.max_llm_input_tokens || undefined;
  const contextTokens = contextUsage?.prompt_tokens ?? 0;
  const contextPercent = contextLimit
    ? Math.min(100, Math.ceil((contextTokens * 100) / contextLimit))
    : undefined;

  const recentResourceSamples = resourceSamples?.slice(-RESOURCE_WINDOW_SAMPLES);
  const cpuSeries = resourceCpuCores(recentResourceSamples);
  const cpuNow = cpuSeries[cpuSeries.length - 1];
  const memorySeries = resourceMemorySeries(recentResourceSamples);
  const memoryNow = memorySeries[memorySeries.length - 1];
  const latestResource = recentResourceSamples?.[recentResourceSamples.length - 1];

  const formatSpeed = (value: number | undefined): string =>
    value === undefined
      ? "\u2014"
      : value < 10
        ? value.toFixed(1)
        : Math.round(value).toLocaleString();
  const formatSeconds = (value: number | undefined): string =>
    value === undefined ? "\u2014" : `${value.toFixed(2)}s`;
  const speedAverage = telemetry?.tokensPerSecond;
  const speedLatest = pulse?.tokensPerSecond[pulse.tokensPerSecond.length - 1];
  const ttftAverage = telemetry?.ttftMeanSeconds;
  const ttftSpread = telemetry?.ttftStdDevSeconds;
  const ttftLatest = pulse?.ttftSeconds[pulse.ttftSeconds.length - 1];
  const speedLabel =
    speedAverage === undefined && speedLatest === undefined
      ? "\u2014"
      : t("insights.speedSummary", {
          average: formatSpeed(speedAverage),
          latest: formatSpeed(speedLatest),
        });
  const ttftLabel =
    ttftAverage === undefined && ttftLatest === undefined
      ? "\u2014"
      : t("insights.ttftSummary", {
          average: formatSeconds(ttftAverage),
          spread:
            ttftSpread === undefined ? "\u2014" : `\u00b1${formatSeconds(ttftSpread)}`,
          latest: formatSeconds(ttftLatest),
        });
  const cacheLabel =
    cacheHitPercent === undefined ? t("insights.unavailable") : `${cacheHitPercent.toFixed(1)}%`;
  const contextLabel = contextPercent === undefined
    ? t("insights.unavailable")
    : `${contextPercent}% \u00b7 ${formatTokensCoarse(contextTokens) ?? "0"}/${formatTokensCoarse(contextLimit) ?? "\u2014"}`;

  const flow = {
    input: runtimeTotals?.prompt_tokens ?? 0,
    cache: runtimeTotals?.cached_tokens ?? 0,
    output: runtimeTotals?.completion_tokens ?? 0,
    think: runtimeTotals?.reasoning_tokens ?? 0,
  };
  const flowMax = Math.max(flow.input, flow.cache, flow.output, flow.think, 1);

  const cpuCap = resourceCpuCapCores(latestResource);
  const memoryCap = resourceMemoryCapBytes(latestResource);
  const cpuValueLabel =
    cpuNow === undefined
      ? t("insights.unavailable")
      : t("insights.cpuUsed", { used: cpuNow.toFixed(2) });
  const memoryValueLabel =
    memoryNow === undefined ? t("insights.unavailable") : formatGiB(memoryNow);
  const cpuRowLabel =
    cpuCap === undefined
      ? t("insights.cpu")
      : t("insights.cpuWithCap", { cap: cpuCap.toFixed(2) });
  const memoryRowLabel =
    memoryCap === undefined
      ? t("insights.mem")
      : t("insights.memWithCap", {
          cap: (memoryCap / (1024 * 1024 * 1024)).toFixed(2),
        });
  const swapLabel =
    latestResource?.swapBytes && latestResource.swapBytes > 0
      ? t("insights.swap", { value: formatGiB(latestResource.swapBytes) })
      : undefined;

  const commitOrder = (next: InsightItemId[]) => {
    setOrder(next);
    writeStorage(INSIGHT_ORDER_STORAGE_KEY, JSON.stringify(next));
  };

  const togglePin = () => {
    setPinned((current) => {
      const next = !current;
      writeStorage(INSIGHT_PINNED_STORAGE_KEY, next ? "1" : null);
      return next;
    });
  };

  const openPanel = () => {
    setPosition((current) =>
      current
        ? clampInsightPanelPosition(
            current,
            panelRef.current?.offsetHeight ?? 360,
            window.innerWidth,
            window.innerHeight,
          )
        : defaultPanelPosition(chipRef.current),
    );
    setOpen(true);
  };

  const startPanelDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    if ((event.target as HTMLElement).closest("button")) return;
    const panel = panelRef.current;
    if (!panel) return;
    const rect = panel.getBoundingClientRect();
    dragRef.current = {
      pointerId: event.pointerId,
      offsetX: event.clientX - rect.left,
      offsetY: event.clientY - rect.top,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  };

  const movePanel = (event: ReactPointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const next = clampInsightPanelPosition(
      { left: event.clientX - drag.offsetX, top: event.clientY - drag.offsetY },
      panelRef.current?.offsetHeight ?? 360,
      window.innerWidth,
      window.innerHeight,
    );
    positionRef.current = next;
    setPosition(next);
  };

  const endPanelDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!dragRef.current || dragRef.current.pointerId !== event.pointerId) return;
    dragRef.current = null;
    const current = positionRef.current;
    if (current) writeStorage(INSIGHT_POSITION_STORAGE_KEY, JSON.stringify(current));
  };

  const renderFlowBlock = () => (
    <div className="session-insight-flow">
      {(
        [
          ["input", "insights.input", "input", flow.input],
          ["cacheTokens", "insights.cacheTokens", "cache", flow.cache],
          ["output", "insights.output", "output", flow.output],
          ["think", "insights.think", "think", flow.think],
        ] as const
      ).map(([key, labelKey, tone, value]) => (
        <div className="session-insight-flow-row" key={key}>
          <span className="session-insight-flow-label">{t(labelKey)}</span>
          <span className="session-insight-flow-number">
            {formatTokensCoarse(value) ?? "0"}
          </span>
          <span className="session-insight-flow-track" aria-hidden="true">
            <span
              className={`session-insight-flow-bar ${tone}`}
              style={{ width: `${Math.max(2, (value / flowMax) * 100)}%` }}
            />
          </span>
        </div>
      ))}
    </div>
  );

  const renderItem = (id: InsightItemId) => {
    let label = "";
    let value: ReactNode = "";
    let detail: ReactNode = null;
    let sparkline: number[] | undefined;
    let tone = "";
    let hint = "";
    switch (id) {
      case "speed":
        label = t("insights.speed");
        value = speedLabel;
        sparkline = pulse?.tokensPerSecond;
        tone = "teal";
        hint = t("insights.speedHint");
        break;
      case "ttft":
        label = t("insights.ttft");
        value = ttftLabel;
        sparkline = pulse?.ttftSeconds;
        tone = "blue";
        hint = t("insights.ttftHint");
        break;
      case "cache":
        label = t("insights.cache");
        value = cacheLabel;
        tone = "violet";
        hint = t("insights.cacheHint");
        break;
      case "context":
        label = t("insights.context");
        value = contextLabel;
        tone = "amber";
        hint = t("insights.contextHint");
        break;
      case "flow":
        label = t("insights.flow");
        value = `${formatTokensCoarse(flow.input) ?? "0"} \u2192 ${formatTokensCoarse(flow.output) ?? "0"}`;
        detail = renderFlowBlock();
        tone = "coral";
        hint = t("insights.flowHint");
        break;
      case "calls":
        label = t("insights.calls");
        value = (
          <span className="session-insight-inline">
            <Sparkles size={11} aria-hidden="true" />
            {formatTokensCoarse(runtimeTotals?.llm_calls) ?? "0"}
            <Wrench size={11} aria-hidden="true" />
            {formatTokensCoarse(runtimeTotals?.tool_calls) ?? "0"}
          </span>
        );
        hint = t("insights.callsHint");
        break;
      case "cpu":
        label = cpuRowLabel;
        value = cpuValueLabel;
        sparkline = cpuSeries;
        tone = "cpu";
        hint = t("insights.cpuHint");
        break;
      case "mem":
        label = memoryRowLabel;
        value = memoryValueLabel;
        detail = swapLabel ? <span className="session-insight-detail">{swapLabel}</span> : null;
        sparkline = memorySeries;
        tone = "mem";
        hint = t("insights.memHint");
        break;
    }
    return (
      <div
        key={id}
        className={`session-insight-row${draggingItemId === id ? " dragging" : ""}${dropTargetId === id ? " drop-target" : ""}`}
        draggable
        tabIndex={0}
        role="group"
        aria-label={`${label} ${typeof value === "string" ? value : ""}`.trim()}
        aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown"
        title={hint}
        onDragStart={(event) => {
          setDraggingItemId(id);
          event.dataTransfer.effectAllowed = "move";
          event.dataTransfer.setData("text/plain", id);
        }}
        onDragOver={(event) => {
          if (!draggingItemId || draggingItemId === id) return;
          event.preventDefault();
          event.dataTransfer.dropEffect = "move";
          setDropTargetId(id);
        }}
        onDragLeave={() => setDropTargetId((current) => (current === id ? null : current))}
        onDrop={(event) => {
          event.preventDefault();
          const source = draggingItemId ?? (event.dataTransfer.getData("text/plain") as InsightItemId);
          if (source && source !== id) {
            commitOrder(moveInsightItem(order, order.indexOf(source), order.indexOf(id)));
          }
          setDraggingItemId(null);
          setDropTargetId(null);
        }}
        onDragEnd={() => {
          setDraggingItemId(null);
          setDropTargetId(null);
        }}
        onKeyDown={(event) => {
          if (!event.altKey || (event.key !== "ArrowUp" && event.key !== "ArrowDown")) return;
          event.preventDefault();
          const index = order.indexOf(id);
          const target = event.key === "ArrowUp" ? index - 1 : index + 1;
          if (target >= 0 && target < order.length) commitOrder(moveInsightItem(order, index, target));
        }}
      >
        <span
          className="session-insight-grip"
          aria-hidden="true"
          title={t("insights.reorderHint")}
        >
          <GripVertical size={12} />
        </span>
        <div className="session-insight-body">
          <div className="session-insight-line">
            <span className="session-insight-label">{label}</span>
            <span className={`session-insight-value ${tone}`}>{value}</span>
            {sparkline && <Sparkline values={sparkline} tone={tone} />}
          </div>
          {detail}
        </div>
      </div>
    );
  };

  const compactItem = (id: InsightItemId): { label: string; value: string } => {
    switch (id) {
      case "speed":
        return {
          label: t("insights.speed"),
          value:
            speedAverage === undefined && speedLatest === undefined
              ? "\u2014"
              : `${formatSpeed(speedAverage)} / ${formatSpeed(speedLatest)}`,
        };
      case "ttft":
        return {
          label: "TTFT",
          value:
            ttftAverage === undefined && ttftLatest === undefined
              ? "\u2014"
              : `${formatSeconds(ttftAverage)} / ${formatSeconds(ttftLatest)}`,
        };
      case "cache":
        return { label: t("insights.cache"), value: cacheLabel };
      case "context":
        return { label: t("insights.context"), value: contextPercent === undefined ? "\u2014" : `${contextPercent}%` };
      case "flow":
        return {
          label: t("insights.flow"),
          value: `${formatTokensCoarse(flow.input) ?? "0"}/${formatTokensCoarse(flow.output) ?? "0"}`,
        };
      case "calls":
        return {
          label: "LLM",
          value: `${formatTokensCoarse(runtimeTotals?.llm_calls) ?? "0"}/${formatTokensCoarse(runtimeTotals?.tool_calls) ?? "0"}`,
        };
      case "cpu":
        return {
          label: "CPU",
          value: cpuNow === undefined ? "\u2014" : t("insights.cpuUsed", { used: cpuNow.toFixed(2) }),
        };
      case "mem":
        return { label: t("insights.mem"), value: memoryNow === undefined ? "\u2014" : formatGiB(memoryNow) };
    }
  };

  const topItem = compactItem(order[0]);
  const panelHeight = panelRef.current?.offsetHeight ?? 360;
  const clampedPosition = position
    ? clampInsightPanelPosition(position, panelHeight, window.innerWidth, window.innerHeight)
    : null;

  return (
    <>
      <button
        ref={chipRef}
        type="button"
        className={`session-insights-chip${open ? " open" : ""}`}
        aria-expanded={open}
        aria-haspopup="dialog"
        aria-label={t("insights.open")}
        title={t("insights.open")}
        onClick={() => (open ? setOpen(false) : openPanel())}
      >
        <Activity size={12} aria-hidden="true" />
        <span className="session-insights-chip-label">{topItem.label}</span>
        <span className="session-insights-chip-value">{topItem.value}</span>
        <ChevronDown size={11} aria-hidden="true" />
      </button>
      {open &&
        clampedPosition &&
        createPortal(
          <div
            ref={panelRef}
            className="session-insights-panel"
            style={{ left: clampedPosition.left, top: clampedPosition.top, width: PANEL_WIDTH }}
            role="dialog"
            aria-label={t("insights.panelAria")}
          >
            <div
              className="session-insights-head"
              onPointerDown={startPanelDrag}
              onPointerMove={movePanel}
              onPointerUp={endPanelDrag}
              onPointerCancel={endPanelDrag}
            >
              <span className="session-insights-title">
                <Activity size={12} aria-hidden="true" />
                {t("insights.title")}
              </span>
              <span className="session-insights-actions">
                <button
                  type="button"
                  className={pinned ? "active" : ""}
                  title={pinned ? t("insights.unpin") : t("insights.pin")}
                  aria-label={pinned ? t("insights.unpin") : t("insights.pin")}
                  aria-pressed={pinned}
                  onClick={togglePin}
                >
                  {pinned ? <Pin size={12} aria-hidden="true" /> : <PinOff size={12} aria-hidden="true" />}
                </button>
                {onReset && (
                  <button
                    type="button"
                    title={t("insights.reset")}
                    aria-label={t("insights.reset")}
                    onClick={onReset}
                  >
                    <RotateCcw size={12} aria-hidden="true" />
                  </button>
                )}
                <button
                  type="button"
                  title={t("insights.collapse")}
                  aria-label={t("insights.collapse")}
                  onClick={() => setOpen(false)}
                >
                  <ChevronDown size={13} aria-hidden="true" />
                </button>
              </span>
            </div>
            <div className="session-insights-body">{order.map(renderItem)}</div>
          </div>,
          document.body,
        )}
    </>
  );
}

function safeOrder(): unknown {
  const raw = readStorage(INSIGHT_ORDER_STORAGE_KEY);
  if (!raw) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}
