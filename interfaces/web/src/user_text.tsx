import { memo } from "react";

/** User-authored text is displayed literally; Markdown syntax has no presentation semantics. */
export const UserText = memo(function UserText({ text }: { text: string }) {
  return <div className="user-plain-text">{text}</div>;
});
