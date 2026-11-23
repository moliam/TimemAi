import { createContext, useContext } from "react";

// Interface-only context: whether the current turn is still working. The file
// edit cards use it to stay expanded during work (the user's default), while
// the finished turn falls back to the normal auto-collapse rule.
const StreamWorkContext = createContext(false);

export const StreamWorkProvider = StreamWorkContext.Provider;

export function useStreamWorkInProgress() {
  return useContext(StreamWorkContext);
}
