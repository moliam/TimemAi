import { describe, expect, it } from "vitest";
import {
  SELECTION_DRAG_TOLERANCE_PX,
  insideMergeableStreamRow,
  isTextSelectionGesture,
  startedInTextRegion,
  toggleGestureFromPointer,
} from "../src/selection_gesture";

const collapsed = { isCollapsed: true } as Selection;
const selected = { isCollapsed: false } as Selection;

describe("text selection gesture guard", () => {
  it("treats a live selection as a selection gesture, not a toggle", () => {
    expect(
      isTextSelectionGesture({ downX: 10, downY: 10, startedInTextRegion: false }, 10, 10, selected),
    ).toBe(true);
  });

  it("still ignores the click when the drag collapsed the selection before click", () => {
    // Chrome 会在 click 之前把选区塌成光标，所以只看 isCollapsed 会漏判；按下到抬起的
    // 位移是拖拽选词的可靠证据。
    expect(
      isTextSelectionGesture(
        { downX: 10, downY: 10, startedInTextRegion: false },
        10 + SELECTION_DRAG_TOLERANCE_PX + 30,
        10,
        collapsed,
      ),
    ).toBe(true);
  });

  it("ignores gestures that started inside a selectable text region", () => {
    expect(
      isTextSelectionGesture({ downX: 10, downY: 10, startedInTextRegion: true }, 10, 10, collapsed),
    ).toBe(true);
  });

  it("keeps a plain click on the toggle itself working", () => {
    expect(
      isTextSelectionGesture(
        { downX: 10, downY: 10, startedInTextRegion: false },
        10 + SELECTION_DRAG_TOLERANCE_PX,
        10,
        collapsed,
      ),
    ).toBe(false);
    expect(isTextSelectionGesture(null, 40, 40, collapsed)).toBe(false);
  });

  it("holds a folded tool group open only for mergeable rows", () => {
    const mergeable = { closest: (selector: string) => (selector.includes(".stream-tool-mergeable") ? {} : null) };
    const visibleDiffRow = { closest: () => null };
    expect(insideMergeableStreamRow(mergeable)).toBe(true);
    expect(insideMergeableStreamRow(visibleDiffRow)).toBe(false);
    expect(insideMergeableStreamRow(null)).toBe(false);
  });

  it("detects selectable text regions from the pointer-down target", () => {
    const inDiff = { closest: (selector: string) => (selector.includes(".file-edit-diff") ? {} : null) };
    const onHeader = { closest: () => null };
    expect(startedInTextRegion(inDiff)).toBe(true);
    expect(startedInTextRegion(onHeader)).toBe(false);
    expect(startedInTextRegion(null)).toBe(false);
    expect(toggleGestureFromPointer(null, 5, 7)).toEqual({
      downX: 5,
      downY: 7,
      startedInTextRegion: false,
    });
  });
});
