import type { ImportSummary, MergeReport } from "../types";
import { h } from "../ui/dom";
import { alertDialog } from "../ui/modal";
import { plural } from "../utils/format";

function statList(rows: [string, number][]): HTMLElement {
  return h(
    "dl",
    { class: "stats" },
    rows.filter(([, value]) => value > 0).map(([label, value]) => [h("dt", { text: label }), h("dd", { text: String(value) })]),
  );
}

export function showImportSummary(summary: ImportSummary): void {
  const notes: string[] = [];
  if (summary.duplicatesSkipped > 0) {
    notes.push(`${plural(summary.duplicatesSkipped, "entry was", "entries were")} already in the vault and skipped.`);
  }
  if (summary.rowsSkipped > 0) notes.push(`${plural(summary.rowsSkipped, "empty row was", "empty rows were")} ignored.`);
  if (summary.attachmentsSkipped > 0) {
    notes.push(`${plural(summary.attachmentsSkipped, "attachment was", "attachments were")} not imported: Lados does not store files.`);
  }
  void alertDialog(
    "Import finished",
    h(
      "div",
      { class: "stack" },
      h("p", {
        text: summary.entriesImported > 0
          ? `${plural(summary.entriesImported, "entry was", "entries were")} imported into the folder “${summary.folderName}”.`
          : "No new entries were found in that file.",
      }),
      summary.foldersCreated > 0 ? h("p", { text: `${plural(summary.foldersCreated, "folder was", "folders were")} created.` }) : null,
      notes.map((note) => h("p", { class: "muted", text: note })),
    ),
  );
}

export function hasChanges(report: MergeReport): boolean {
  return Object.values(report).some((value) => value > 0);
}

/** One line for notifications, such as "3 entries added, 1 updated". */
export function describeChanges(report: MergeReport): string {
  const parts = [
    report.entriesAdded > 0 && `${plural(report.entriesAdded, "entry", "entries")} added`,
    report.entriesUpdated > 0 && `${plural(report.entriesUpdated, "entry", "entries")} updated`,
    report.entriesDeleted > 0 && `${plural(report.entriesDeleted, "entry", "entries")} deleted`,
    report.foldersAdded + report.foldersUpdated + report.foldersDeleted > 0 && "folders updated",
  ].filter((part): part is string => typeof part === "string");
  return parts.length > 0 ? parts.join(", ") : "no visible changes";
}

export function showMergeReport(report: MergeReport): void {
  if (!hasChanges(report)) {
    void alertDialog("Vaults merged", h("p", { text: "This vault already contained everything in the other one. Nothing changed." }));
    return;
  }
  void alertDialog(
    "Vaults merged",
    h(
      "div",
      { class: "stack" },
      h("p", { text: "The other vault was merged into this one. This is what changed here:" }),
      statList([
        ["Entries added", report.entriesAdded],
        ["Entries updated", report.entriesUpdated],
        ["Entries deleted", report.entriesDeleted],
        ["Folders added", report.foldersAdded],
        ["Folders updated", report.foldersUpdated],
        ["Folders deleted", report.foldersDeleted],
        ["Duplicates skipped", report.duplicatesSkipped],
        ["Conflicts resolved", report.conflictsResolved],
      ]),
      report.conflictsResolved > 0
        ? h("p", {
            class: "muted",
            text: "Entries edited in both vaults were combined field by field. Where both changed the same field, the most recent value was kept. The versions they replaced are in each entry's history.",
          })
        : null,
      h("p", { class: "muted", text: "The other vault file was not modified." }),
    ),
  );
}
