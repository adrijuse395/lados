// Local autofill preview: paste a page URL, see matching entries, copy username/password.
// Bridge to the browser comes later; this exercises the same Rust matching API.

import { backend } from "../services/backend";
import type { EntryField, UrlMatchHit } from "../types";
import { h } from "../ui/dom";
import { showDialog } from "../ui/modal";
import { toast, toastError } from "../ui/toast";

function matchLabel(kind: UrlMatchHit["matchKind"]): string {
  switch (kind) {
    case "exactHost":
      return "Same site";
    case "parentHost":
      return "Parent site";
    default:
      return kind;
  }
}

async function copy(id: string, field: EntryField, label: string): Promise<void> {
  try {
    await backend.copyField(id, field);
    toast(`${label} copied`);
  } catch (error) {
    toastError(error);
  }
}

function renderHits(container: HTMLElement, hits: UrlMatchHit[]): void {
  container.replaceChildren();
  if (hits.length === 0) {
    container.append(h("p", { class: "muted", text: "No entries match this URL." }));
    return;
  }
  for (const hit of hits) {
    const { entry } = hit;
    container.append(
      h(
        "div",
        { class: "url-match-hit" },
        h(
          "div",
          { class: "url-match-hit__meta" },
          h("strong", { class: "truncate", text: entry.title }),
          h("span", { class: "muted truncate", text: entry.username || "(no username)" }),
          h("span", { class: "muted truncate", text: entry.url ?? "" }),
          h("span", { class: "url-match-hit__kind", text: matchLabel(hit.matchKind) }),
        ),
        h(
          "div",
          { class: "url-match-hit__actions" },
          h(
            "button",
            {
              type: "button",
              class: "button",
              disabled: !entry.username,
              onClick: () => void copy(entry.id, { kind: "username" }, "Username"),
            },
            "Copy user",
          ),
          h(
            "button",
            {
              type: "button",
              class: "button",
              disabled: !entry.hasPassword,
              onClick: () => void copy(entry.id, { kind: "password" }, "Password"),
            },
            "Copy password",
          ),
        ),
      ),
    );
  }
}

export async function openUrlMatchTester(): Promise<void> {
  const input = h("input", {
    class: "input",
    type: "url",
    placeholder: "https://mail.example.com/login",
    spellcheck: false,
    autocomplete: "off",
    "data-autofocus": "true",
  }) as HTMLInputElement;

  const results = h("div", { class: "stack url-match-results" }, h("p", { class: "muted", text: "Enter a page URL and press Find." }));

  await showDialog({
    title: "Find for URL",
    size: "medium",
    body: h(
      "div",
      { class: "stack" },
      h("p", {
        class: "muted",
        text: "Preview of autofill matching. Same logic the browser bridge will use later.",
      }),
      h("label", { class: "field" }, h("span", { class: "field-label", text: "Page URL" }), input),
      results,
    ),
    submitLabel: "Find",
    cancelLabel: "Close",
    onSubmit: async () => {
      const url = input.value.trim();
      if (!url) throw new Error("Enter a URL first");
      const hits = await backend.findEntriesForUrl(url);
      renderHits(results, hits);
      return undefined;
    },
  });
}
