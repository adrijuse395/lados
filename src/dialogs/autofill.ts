// The prompt shown when the browser extension asks for a password.
//
// This is the boundary that actually protects the vault from the local machine: a web page cannot
// reach the bridge at all, and a program that *can* (by reading the pairing code) still cannot
// click "Allow" here.

import { backend } from "../services/backend";
import type { AutofillRevealRequest } from "../types";
import { h } from "../ui/dom";
import { showDialog } from "../ui/modal";
import { toastError } from "../ui/toast";

/** Asks whether the extension may fill one login. The Rust side gives up after 30 seconds. */
export async function askAutofillReveal(request: AutofillRevealRequest): Promise<void> {
  const answer = async (allow: boolean) => {
    try {
      await backend.autofillAnswer(request.id, allow);
    } catch {
      // Expired, already answered, or the vault locked: nothing to send.
    }
  };

  try {
    const allowed = await showDialog<true>({
      title: "Fill this login?",
      size: "small",
      body: h(
        "div",
        { class: "stack" },
        h("p", { text: "The Lados browser extension wants to fill a login." }),
        h("p", {
          class: "path-box",
          text: request.username ? `${request.title} — ${request.username}` : request.title,
        }),
        request.url ? h("p", { class: "muted", text: request.url }) : null,
        h("p", {
          class: "muted",
          text: "Allow it only if you just asked to fill a login. If you deny, no password is sent.",
        }),
      ),
      cancelLabel: "Deny",
      submitLabel: "Allow",
      onSubmit: () => true,
    });
    await answer(allowed === true);
  } catch (error) {
    await answer(false);
    toastError(error);
  }
}
