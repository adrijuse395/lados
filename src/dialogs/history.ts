import { backend } from "../services/backend";
import { preferences } from "../state/store";
import type { RevisionSummary, Uuid } from "../types";
import { h } from "../ui/dom";
import { svgIcon } from "../ui/icons";
import { confirmDialog, showDialog, type DialogContext } from "../ui/modal";
import { toast, toastError } from "../ui/toast";
import { formatDateTime } from "../utils/format";

/** Shows the previous versions of an entry. Resolves to `true` if one was restored. */
export async function openHistory(entryId: Uuid): Promise<boolean> {
  const revisions = await backend.getEntryHistory(entryId);

  const copyPassword = async (revision: RevisionSummary) => {
    try {
      await backend.copyRevisionPassword(entryId, revision.index);
      const seconds = preferences().clipboardClearSeconds;
      toast(seconds > 0 ? `Old password copied. The clipboard will be cleared in ${seconds} s` : "Old password copied", "success");
    } catch (error) {
      toastError(error);
    }
  };

  const restore = async (revision: RevisionSummary, context: DialogContext<boolean>) => {
    const confirmed = await confirmDialog({
      title: "Restore this version?",
      message: `The entry will go back to how it was on ${formatDateTime(revision.modifiedAt)}. The current version is kept in the history, so this can be undone.`,
      confirmLabel: "Restore",
    });
    if (!confirmed) return;
    try {
      await backend.restoreRevision(entryId, revision.index);
      context.close(true);
    } catch (error) {
      context.setError(error instanceof Error ? error.message : String(error));
    }
  };

  const item = (revision: RevisionSummary, context: DialogContext<boolean>) =>
    h(
      "li",
      { class: "history-item" },
      h(
        "div",
        { class: "history-text" },
        h("div", { class: "history-date", text: formatDateTime(revision.modifiedAt) }),
        h(
          "div",
          { class: "history-summary" },
          h("span", { class: "history-title", text: revision.title }),
          revision.username ? h("span", { class: "muted", text: revision.username }) : null,
          revision.passwordDiffers ? h("span", { class: "badge info", text: "Different password" }) : null,
        ),
      ),
      h(
        "div",
        { class: "history-actions" },
        h("button", { type: "button", class: "button small", title: "Copy the password of this version", onClick: () => void copyPassword(revision) }, svgIcon("copy"), "Password"),
        h("button", { type: "button", class: "button small", onClick: () => void restore(revision, context) }, svgIcon("history"), "Restore"),
      ),
    );

  const restored = await showDialog<boolean>({
    title: "Previous versions",
    size: "medium",
    body: (context) =>
      revisions.length > 0
        ? h("ul", { class: "history-list" }, revisions.map((revision) => item(revision, context)))
        : h("p", { class: "muted", text: "This entry has not been modified since it was created." }),
    hideCancel: true,
    submitLabel: "Close",
    onSubmit: () => false,
  });
  return restored === true;
}
