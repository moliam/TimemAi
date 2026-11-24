import { useRef } from "react";

// 用户反馈：在 diff 框里选字时会误触命令的展开/收起，页面跟着跳动。
// 选中文字是纯阅读动作，判定规则（三条任一命中就交还给浏览器）：
// 1. 点击时仍留下非折叠选区；
// 2. 指针按下点落在可选中文本区（diff 行、命令、输出、正文）——用户是从那里开始拖的；
// 3. 按下到抬起移动超过容差——这就是一次拖拽选词，不是点按。
export const SELECTION_DRAG_TOLERANCE_PX = 4;

const TEXT_REGION_SELECTOR = [
  ".file-edit-diff",
  ".stream-tool-command",
  ".tool-activity-code-shell",
  ".stream-tool-detail",
  ".turn-work-detail",
  ".markdown-body",
  "pre",
].join(",");

export type ClosestCapable = { closest(selector: string): unknown };

// 单独抽出：既能被指针对手调用，也能在没有 DOM 的单测里用桩对象验证。
export function startedInTextRegion(element: ClosestCapable | null | undefined): boolean {
  return !!element?.closest(TEXT_REGION_SELECTOR);
}

/// 折叠组只收纳可合并的行；diff、运行中行等始终可见，选中它们不应触发组的
/// 展开/收起。这个标记让交互保护只作用于真正会被收起的行。
export function insideMergeableStreamRow(element: ClosestCapable | null | undefined): boolean {
  return !!element?.closest(".stream-tool-mergeable");
}

export type ToggleGesture = {
  downX: number;
  downY: number;
  startedInTextRegion: boolean;
};

export function toggleGestureFromPointer(
  target: EventTarget | null,
  clientX: number,
  clientY: number,
): ToggleGesture {
  const element =
    typeof Element !== "undefined" && target instanceof Element
      ? (target as unknown as ClosestCapable)
      : null;
  return {
    downX: clientX,
    downY: clientY,
    startedInTextRegion: startedInTextRegion(element),
  };
}

export function isTextSelectionGesture(
  gesture: ToggleGesture | null,
  upX: number,
  upY: number,
  selection: Selection | null | undefined = typeof window === "undefined"
    ? null
    : window.getSelection(),
): boolean {
  if (selection && !selection.isCollapsed) return true;
  if (!gesture) return false;
  if (gesture.startedInTextRegion) return true;
  return Math.hypot(upX - gesture.downX, upY - gesture.downY) > SELECTION_DRAG_TOLERANCE_PX;
}

// 展开/收起控件的统一守卫：键盘操作不经过指针，因此只有在 click 路径上使用。
export function useSelectionGestureGuard() {
  const gesture = useRef<ToggleGesture | null>(null);
  const onPointerDown = (event: {
    target: EventTarget | null;
    clientX: number;
    clientY: number;
  }) => {
    gesture.current = toggleGestureFromPointer(event.target, event.clientX, event.clientY);
  };
  const ignoreToggle = (event: { clientX: number; clientY: number }) => {
    const current = gesture.current;
    gesture.current = null;
    return isTextSelectionGesture(current, event.clientX, event.clientY);
  };
  return { onPointerDown, ignoreToggle };
}
