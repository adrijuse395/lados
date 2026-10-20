import { backend } from "../services/backend";
import { flattenedFolders, store } from "../state/store";
import type { CustomField, EntryContent, Icon, Uuid } from "../types";
import { field, iconButton, itemIcon, passwordInput } from "../ui/controls";
import { h } from "../ui/dom";
import { svgIcon } from "../ui/icons";
import { confirmDialog, showDialog } from "../ui/modal";
import { openGenerator } from "./generator";
import { pickIcon } from "./icon-picker";

function emptyContent(): EntryContent {
  return { title: "", username: "", password: "", url: null, notes: null, expirationDate: null, icon: null, customFields: [] };
}

function hostOf(url: string): string {
  const withoutScheme = url.trim().replace(/^[a-z][a-z0-9+.-]*:\/\//i, "");
  const host = withoutScheme.split(/[/?#]/)[0].split("@").pop()?.split(":")[0] ?? "";
  return host.replace(/^www\./i, "");
}

export function folderSelect(current: Uuid | null): HTMLSelectElement {
  return h(
    "select",
    { class: "input" },
    h("option", { value: "", selected: current === null, text: "No folder" }),
    flattenedFolders().map(({ folder, depth }) =>
      h("option", { value: folder.id, selected: folder.id === current, text: `${"\u2003".repeat(depth)}${folder.name}` }),
    ),
  );
}

interface CustomFieldRow {
  element: HTMLElement;
  value(): CustomField;
}

function customFieldRow(initial: CustomField, onRemove: (row: CustomFieldRow) => void): CustomFieldRow {
  let isProtected = initial.protected;
  const name = h("input", { class: "input", value: initial.name, placeholder: "Name", spellcheck: false, autocomplete: "off" });
  const value = h("input", {
    class: "input mono",
    value: initial.value,
    placeholder: "Value",
    spellcheck: false,
    autocomplete: "off",
    type: isProtected ? "password" : "text",
  });
  const reveal = iconButton("eye", "Show value", () => {
    value.type = value.type === "password" ? "text" : "password";
    reveal.replaceChildren(svgIcon(value.type === "password" ? "eye" : "eyeOff"));
  });
  const protect = h(
    "button",
    {
      type: "button",
      class: "icon-button",
      title: "Protected values are masked and left out of search",
      "aria-label": "Protect this value",
      onClick: () => {
        isProtected = !isProtected;
        update();
      },
    },
    svgIcon("lock"),
  );
  function update() {
    protect.classList.toggle("active", isProtected);
    protect.setAttribute("aria-pressed", String(isProtected));
    reveal.hidden = !isProtected;
    value.type = isProtected ? "password" : "text";
    reveal.replaceChildren(svgIcon("eye"));
  }

  const row: CustomFieldRow = {
    element: h(
      "div",
      { class: "custom-field-row" },
      name,
      h("div", { class: "input-group" }, value, reveal),
      protect,
      iconButton("trash", "Remove this field", () => onRemove(row)),
    ),
    value: () => ({ name: name.value, value: value.value, protected: isProtected }),
  };
  update();
  return row;
}

/** Creates an entry in `folderId`, or edits `entryId`. Resolves with the entry id. */
export async function openEntryEditor(options: { entryId?: Uuid; folderId?: Uuid | null }): Promise<Uuid | null> {
  const entryId = options.entryId ?? null;
  const initial = entryId ? await backend.getEntryForEdit(entryId) : emptyContent();
  const initialFolder = entryId
    ? (store.vault?.entries.find((entry) => entry.id === entryId)?.folderId ?? null)
    : (options.folderId ?? null);
  let icon: Icon | null = initial.icon;

  const title = h("input", { class: "input", value: initial.title, spellcheck: false, autocomplete: "off", "data-autofocus": "" });
  const username = h("input", { class: "input", value: initial.username, spellcheck: false, autocomplete: "off" });
  const password = passwordInput({
    value: initial.password,
    strength: true,
    onGenerate: async () => {
      const generated = await openGenerator({ pick: true });
      if (generated !== null) password.setValue(generated);
    },
  });
  const url = h("input", { class: "input", value: initial.url ?? "", placeholder: "https://", spellcheck: false, autocomplete: "off" });
  url.addEventListener("change", () => {
    if (!title.value.trim() && url.value.trim()) title.value = hostOf(url.value);
  });
  const folder = folderSelect(initialFolder);
  const expires = h("input", { class: "input", type: "date", value: initial.expirationDate ?? "" });
  const notes = h("textarea", { class: "input notes-input", rows: 4, value: initial.notes ?? "", spellcheck: false });

  const iconChoice = h(
    "button",
    {
      type: "button",
      class: "icon-picker-button",
      title: "Choose an icon",
      "aria-label": "Choose an icon",
      onClick: async () => {
        const chosen = await pickIcon(icon, "key");
        if (chosen === undefined) return;
        icon = chosen;
        iconChoice.replaceChildren(itemIcon(icon, "key"));
      },
    },
    itemIcon(icon, "key"),
  );

  const rows: CustomFieldRow[] = [];
  const fieldList = h("div", { class: "custom-field-list" });
  const removeRow = (row: CustomFieldRow) => {
    rows.splice(rows.indexOf(row), 1);
    row.element.remove();
  };
  const addRow = (fieldValue: CustomField) => {
    const row = customFieldRow(fieldValue, removeRow);
    rows.push(row);
    fieldList.append(row.element);
    return row;
  };
  initial.customFields.forEach(addRow);
  const addField = h(
    "button",
    {
      type: "button",
      class: "button small",
      onClick: () => addRow({ name: "", value: "", protected: false }).element.querySelector("input")?.focus(),
    },
    svgIcon("plus"),
    "Add field",
  );

  const selectedFolder = () => folder.value || null;
  const collect = (): EntryContent => ({
    title: title.value,
    username: username.value,
    password: password.value(),
    url: url.value.trim() || null,
    notes: notes.value.trim() ? notes.value : null,
    expirationDate: expires.value || null,
    icon,
    customFields: rows.map((row) => row.value()),
  });
  const snapshot = () => JSON.stringify([collect(), selectedFolder()]);
  const original = snapshot();

  const body = h(
    "div",
    { class: "entry-editor" },
    h("div", { class: "title-row" }, iconChoice, field("Title", title)),
    h("div", { class: "grid-2" }, field("Username", username), field("Password", password.element, { target: password.input })),
    field("Website", url),
    h(
      "div",
      { class: "grid-2" },
      field("Folder", folder),
      field("Expires", expires, { hint: "Leave it empty if it never expires." }),
    ),
    field("Notes", notes),
    h(
      "section",
      { class: "custom-fields" },
      h("div", { class: "section-header" }, h("h3", { text: "Custom fields" }), addField),
      fieldList,
      h("p", { class: "field-hint", text: "For PINs, security answers, recovery codes… Use the lock to mask a value." }),
    ),
  );
  // Ctrl+S saves, as in most editors.
  body.addEventListener("keydown", (event) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
      event.preventDefault();
      body.closest("form")?.requestSubmit();
    }
  });

  return showDialog<Uuid>({
    title: entryId ? "Edit entry" : "New entry",
    size: "large",
    body,
    submitLabel: entryId ? "Save" : "Create",
    busyLabel: "Saving…",
    onSubmit: async () => {
      const content = collect();
      if (!content.title.trim()) {
        title.focus();
        throw new Error("The entry needs a title");
      }
      if (content.customFields.some((custom) => !custom.name.trim() && custom.value)) {
        throw new Error("Every custom field needs a name");
      }
      if (!entryId) return backend.createEntry(selectedFolder(), content);
      await backend.updateEntry(entryId, content);
      if (selectedFolder() !== initialFolder) await backend.moveEntry(entryId, selectedFolder());
      return entryId;
    },
    confirmDismiss: async () =>
      snapshot() === original ||
      confirmDialog({
        title: "Discard changes?",
        message: "The changes made to this entry have not been saved.",
        confirmLabel: "Discard",
        danger: true,
      }),
  });
}
