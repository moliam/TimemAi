import { applyAppearance, loadAppearance } from "./appearance";

// Apply the persisted theme before React and the main stylesheet load. Keeping
// this in a same-origin module avoids weakening the host CSP for inline code.
applyAppearance(loadAppearance());
