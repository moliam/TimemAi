import { useEffect, useRef, useState } from "react";

/**
 * Returns a value that refreshes at most once per interval. Streaming text
 * changes at delta frequency; consumers use this to cap expensive render
 * work while still converging to the latest value between ticks. A stable
 * value short-circuits through Object.is so React can bail out of the
 * re-render entirely.
 */
export function useThrottledValue<T>(value: T, intervalMs: number): T {
  const [shown, setShown] = useState(value);
  const latestRef = useRef(value);
  latestRef.current = value;
  useEffect(() => {
    const timer = window.setInterval(() => {
      setShown((current) =>
        Object.is(current, latestRef.current) ? current : latestRef.current,
      );
    }, intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs]);
  return shown;
}
