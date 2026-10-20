// Tells the core that the user is still here, which postpones the automatic lock.

import { backend } from "../services/backend";

const REPORT_INTERVAL_MS = 15_000;
const ACTIVITY_EVENTS = ["keydown", "pointerdown", "pointermove", "wheel"] as const;

export function trackActivity(): void {
  let lastReport = 0;
  const report = () => {
    const now = Date.now();
    if (now - lastReport < REPORT_INTERVAL_MS) return;
    lastReport = now;
    backend.touch().catch(() => undefined);
  };
  for (const type of ACTIVITY_EVENTS) {
    window.addEventListener(type, report, { capture: true, passive: true });
  }
}
