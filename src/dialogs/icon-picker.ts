import { reloadVault } from "../app";
import { backend } from "../services/backend";
import { store } from "../state/store";
import type { Icon } from "../types";
import { itemIcon } from "../ui/controls";
import { h, mount } from "../ui/dom";
import { ENTRY_ICONS, svgIcon, type IconName } from "../ui/icons";
import { showDialog, type DialogContext } from "../ui/modal";
import { toastError } from "../ui/toast";

function sameIcon(a: Icon | null, b: Icon | null): boolean {
  return a === b || (a !== null && b !== null && a.kind === b.kind && a.value === b.value);
}

/** Resolves with the chosen icon, `null` for the default one, or `undefined` if cancelled. */
export async function pickIcon(current: Icon | null, fallback: IconName): Promise<Icon | null | undefined> {
  let selected = current;
  let context: DialogContext<{ icon: Icon | null }> | null = null;
  const grid = h("div", { class: "icon-grid", role: "listbox", "aria-label": "Icons" });

  const option = (icon: Icon | null, label: string) => {
    const active = sameIcon(icon, selected);
    return h(
      "button",
      {
        type: "button",
        class: ["icon-option", active && "selected"],
        role: "option",
        title: label,
        "aria-label": label,
        "aria-selected": String(active),
        onClick: () => {
          selected = icon;
          render();
        },
        onDblClick: () => context?.close({ icon }),
      },
      itemIcon(icon, fallback, "icon-preview"),
    );
  };

  function render() {
    mount(
      grid,
      option(null, "Default icon"),
      ENTRY_ICONS.map((name) => option({ kind: "builtin", value: name }, name)),
      (store.vault?.customIcons ?? []).map((custom) => option({ kind: "custom", value: custom.id }, "Custom image")),
    );
    grid.querySelector<HTMLElement>(".icon-option.selected")?.focus();
  }

  const addImage = h(
    "button",
    {
      type: "button",
      class: "button",
      onClick: async () => {
        try {
          const id = await backend.addCustomIcon();
          if (!id) return;
          await reloadVault();
          selected = { kind: "custom", value: id };
          render();
        } catch (error) {
          toastError(error);
        }
      },
    },
    svgIcon("image"),
    "Add image…",
  );

  const result = await showDialog<{ icon: Icon | null }>({
    title: "Choose an icon",
    size: "medium",
    body: (dialog) => {
      context = dialog;
      render();
      return h("div", { class: "stack" }, grid, h("p", { class: "field-hint", text: "Images are stored inside the vault, so they travel with it." }));
    },
    extraActions: addImage,
    submitLabel: "Choose",
    onSubmit: () => ({ icon: selected }),
  });
  return result === null ? undefined : result.icon;
}
