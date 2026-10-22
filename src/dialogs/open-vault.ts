// Pick a vault to open. On mobile, default to Documents/Lados (same folder as create);
// the system picker is only for bringing a file in from somewhere else.

import { backend } from "../services/backend";
import type { RecentVault } from "../types";
import { h } from "../ui/dom";
import { svgIcon } from "../ui/icons";
import { showDialog } from "../ui/modal";

async function browseElsewhere(mobile: boolean): Promise<string | null> {
  return backend.pickOpenPath("vault", mobile ? "Choose a .lados file to import" : "Open a vault");
}

/**
 * Returns a vault path, or `null` if cancelled.
 * Mobile: choose from Documents › Lados first; optionally browse elsewhere (copied in).
 * Desktop: native open dialog, starting in the standard vaults folder when possible.
 */
export async function pickVaultToOpen(): Promise<string | null> {
  const mobile = await backend.isMobileShell().catch(() => false);
  if (!mobile) {
    return browseElsewhere(false);
  }

  const vaults = await backend.listDeviceVaults().catch(() => [] as RecentVault[]);

  return showDialog<string>({
    title: "Open a vault",
    size: "medium",
    cancelLabel: "Cancel",
    submitLabel: "Browse elsewhere…",
    body: (context) =>
      h(
        "div",
        { class: "stack open-vault-picker" },
        h("p", {
          class: "dialog-message",
          text:
            vaults.length > 0
              ? "These are the vaults in Documents › Lados — the same folder where new vaults are saved."
              : "No vaults in Documents › Lados yet. Create one, or import a .lados file from another folder.",
        }),
        vaults.length > 0
          ? h(
              "ul",
              { class: "open-vault-list" },
              vaults.map((vault) =>
                h(
                  "li",
                  null,
                  h(
                    "button",
                    {
                      type: "button",
                      class: "open-vault-item",
                      title: vault.path,
                      onClick: () => context.close(vault.path),
                    },
                    svgIcon("lock", "icon"),
                    h(
                      "span",
                      { class: "open-vault-text" },
                      h("span", { class: "open-vault-name", text: vault.fileName }),
                      h("span", { class: "open-vault-path muted", text: vault.path }),
                    ),
                  ),
                ),
              ),
            )
          : null,
      ),
    onSubmit: async () => {
      const path = await browseElsewhere(true);
      return path ?? undefined;
    },
  });
}
