import { backend } from "../services/backend";
import { folderById, folderPath } from "../state/store";
import type { Icon, Uuid } from "../types";
import { field, itemIcon } from "../ui/controls";
import { h } from "../ui/dom";
import { showDialog } from "../ui/modal";
import { pickIcon } from "./icon-picker";

/** Creates a folder inside `parentId`, or edits `folderId`. Resolves with the folder id. */
export async function openFolderEditor(options: { folderId?: Uuid; parentId?: Uuid | null }): Promise<Uuid | null> {
  const existing = folderById(options.folderId ?? null);
  const parentId = options.parentId ?? null;
  let icon: Icon | null = existing?.icon ?? null;

  const name = h("input", {
    class: "input",
    value: existing?.name ?? "",
    placeholder: "Folder name",
    spellcheck: false,
    autocomplete: "off",
    "data-autofocus": "",
  });
  const iconButton = h(
    "button",
    {
      type: "button",
      class: "icon-picker-button",
      title: "Choose an icon",
      "aria-label": "Choose an icon",
      onClick: async () => {
        const chosen = await pickIcon(icon, "folder");
        if (chosen === undefined) return;
        icon = chosen;
        iconButton.replaceChildren(itemIcon(icon, "folder"));
      },
    },
    itemIcon(icon, "folder"),
  );
  const parentPath = folderPath(parentId).join(" › ");

  return showDialog<Uuid>({
    title: existing ? "Edit folder" : parentId ? "New subfolder" : "New folder",
    size: "small",
    body: h(
      "div",
      { class: "stack" },
      h("div", { class: "title-row" }, iconButton, field("Name", name)),
      !existing && parentPath ? h("p", { class: "field-hint", text: `It will be created inside “${parentPath}”.` }) : null,
    ),
    submitLabel: existing ? "Save" : "Create",
    onSubmit: async () => {
      const value = name.value.trim();
      if (!value) {
        name.focus();
        throw new Error("The folder needs a name");
      }
      if (existing) {
        await backend.updateFolder(existing.id, value, icon);
        return existing.id;
      }
      return backend.createFolder(parentId, value, icon);
    },
  });
}
