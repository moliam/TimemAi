import { useLayoutEffect, useRef } from "react";

type FlippingTimeSegment = {
  kind: "digits" | "text";
  key: string;
  value: string;
};

export function flippingTimeSegments(value: string): FlippingTimeSegment[] {
  const runs = value.match(/\d+|\D+/g) ?? [];
  return runs.map((run, index) => {
    if (!/^\d+$/.test(run)) {
      return { kind: "text", key: `text:${index}:${run}`, value: run };
    }
    const suffix = runs[index + 1] ?? "end";
    const occurrenceFromRight = runs.slice(index + 1).reduce((count, candidate, candidateIndex) => {
      const absoluteIndex = index + candidateIndex + 1;
      if (!/^\d+$/.test(candidate) || (runs[absoluteIndex + 1] ?? "end") !== suffix) {
        return count;
      }
      return count + 1;
    }, 0);
    return {
      kind: "digits",
      key: `digits:${suffix}:${occurrenceFromRight}`,
      value: run,
    };
  });
}

function FlippingDigit({ value }: { value: string }) {
  const previousRef = useRef(value);
  const previous = previousRef.current;
  const changed = previous !== value;
  useLayoutEffect(() => {
    previousRef.current = value;
  }, [value]);
  return (
    <span className={`time-flip-digit${changed ? " changing" : ""}`}>
      <span className="time-flip-digit-placeholder">{value}</span>
      {changed ? (
        <span className="time-flip-digit-transition" key={`${previous}-${value}`}>
          <span className="time-flip-digit-face time-flip-digit-out">{previous}</span>
          <span className="time-flip-digit-face time-flip-digit-in">{value}</span>
        </span>
      ) : (
        <span className="time-flip-digit-face">{value}</span>
      )}
    </span>
  );
}

export function FlippingTime({ value }: { value: string }) {
  return (
    <span className="time-flip" aria-hidden="true">
      {flippingTimeSegments(value).map((segment) =>
        segment.kind === "text" ? (
          <span className="time-flip-static" key={segment.key}>{segment.value}</span>
        ) : (
          <span className="time-flip-number" key={segment.key}>
            {Array.from(segment.value).map((digit, index) => (
              <FlippingDigit
                key={segment.value.length - index - 1}
                value={digit}
              />
            ))}
          </span>
        ),
      )}
    </span>
  );
}
