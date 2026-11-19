// A full-window spinner for work with no progress to report.

import { spinner } from "./controls";
import { h } from "./dom";

/**
 * Covers the window with a spinner and a line of text while a long task runs.
 *
 * Importing tens of thousands of rows left the window blank for minutes and looked like a hang
 * (the red-team report, M15), so the task now says what it is doing until it finishes.
 */
export async function withBusy<T>(message: string, task: () => Promise<T>): Promise<T> {
  const overlay = h(
    "div",
    { class: "busy-overlay", role: "alert", "aria-live": "assertive" },
    h("div", { class: "busy-box" }, spinner(), h("p", { text: message })),
  );
  document.body.append(overlay);
  try {
    return await task();
  } finally {
    overlay.remove();
  }
}
