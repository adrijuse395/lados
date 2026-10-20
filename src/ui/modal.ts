// Modal dialogs: stacked, keyboard friendly (Escape, focus trap) and returning promises.

import { h, type Child } from "./dom";
import { svgIcon } from "./icons";
import { errorMessage } from "./toast";

export type ModalSize = "small" | "medium" | "large";

interface Modal {
  element: HTMLElement;
  /** What Escape and the close button do. */
  dismiss(): void;
  /** Removes the dialog unconditionally. */
  close(): void;
}

const stack: Modal[] = [];
let counter = 0;

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function isModalOpen(): boolean {
  return stack.length > 0;
}

/** Closes every dialog, for example when the vault gets locked. */
export function closeAllModals(): void {
  while (stack.length > 0) stack[stack.length - 1].close();
}

document.addEventListener(
  "keydown",
  (event) => {
    const top = stack[stack.length - 1];
    if (!top) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      top.dismiss();
    } else if (event.key === "Tab") {
      trapFocus(top.element, event);
    }
  },
  true,
);

function focusables(container: HTMLElement): HTMLElement[] {
  return [...container.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((element) => element.offsetParent !== null);
}

function trapFocus(container: HTMLElement, event: KeyboardEvent): void {
  const elements = focusables(container);
  if (elements.length === 0) return;
  const first = elements[0];
  const last = elements[elements.length - 1];
  const active = document.activeElement;
  if (!container.contains(active)) {
    event.preventDefault();
    first.focus();
  } else if (event.shiftKey && active === first) {
    event.preventDefault();
    last.focus();
  } else if (!event.shiftKey && active === last) {
    event.preventDefault();
    first.focus();
  }
}

function focusInitial(container: HTMLElement): void {
  const preferred = container.querySelector<HTMLElement>("[data-autofocus]");
  const target = preferred ?? focusables(container.querySelector<HTMLElement>(".modal-body") ?? container)[0];
  target?.focus();
  if (target instanceof HTMLInputElement && target.type !== "checkbox") target.select();
}

export function openModal(options: {
  title: string;
  content: Child;
  size?: ModalSize;
  className?: string;
  /** Return `false` to keep the dialog open. */
  onDismiss?: () => boolean | void;
}): Modal {
  const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const titleId = `modal-title-${++counter}`;

  const close = () => {
    const index = stack.indexOf(modal);
    if (index < 0) return;
    stack.splice(index, 1);
    backdrop.remove();
    if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true });
  };
  const dismiss = () => {
    if (options.onDismiss?.() !== false) close();
  };

  const dialog = h(
    "div",
    {
      class: ["modal", `modal-${options.size ?? "medium"}`, options.className],
      role: "dialog",
      "aria-modal": "true",
      "aria-labelledby": titleId,
    },
    h(
      "header",
      { class: "modal-header" },
      h("h2", { id: titleId, text: options.title }),
      h("button", { type: "button", class: "icon-button", title: "Close", "aria-label": "Close", onClick: dismiss }, svgIcon("close")),
    ),
    options.content,
  );
  const backdrop = h("div", { class: "modal-backdrop" }, dialog);
  const modal: Modal = { element: dialog, dismiss, close };

  stack.push(modal);
  document.body.append(backdrop);
  requestAnimationFrame(() => focusInitial(dialog));
  return modal;
}

export interface DialogContext<T> {
  close(result: T): void;
  setError(message: string | null): void;
  submit(): void;
}

export interface DialogOptions<T> {
  title: string;
  size?: ModalSize;
  body: Child | ((context: DialogContext<T>) => Child);
  submitLabel?: string;
  /** Shown on the submit button while `onSubmit` runs. */
  busyLabel?: string;
  cancelLabel?: string;
  hideCancel?: boolean;
  danger?: boolean;
  focusCancel?: boolean;
  /** Buttons shown on the left side of the footer. */
  extraActions?: Child;
  /**
   * Returns the value that closes the dialog, or `undefined` to keep it open.
   * Errors thrown here are shown inside the dialog.
   */
  onSubmit: () => Promise<T | undefined> | T | undefined;
  /** Asked before closing without submitting. Return `false` to stay. */
  confirmDismiss?: () => boolean | Promise<boolean>;
}

/** A dialog with a form. Resolves with the submitted value, or `null` if cancelled. */
export function showDialog<T>(options: DialogOptions<T>): Promise<T | null> {
  return new Promise((resolve) => {
    let settled = false;
    let busy = false;

    const finish = (result: T | null) => {
      if (settled) return;
      settled = true;
      modal.close();
      resolve(result);
    };
    const setError = (message: string | null) => {
      error.textContent = message ?? "";
      error.hidden = !message;
    };
    const submitLabel = options.submitLabel ?? "Save";
    const setBusy = (value: boolean) => {
      busy = value;
      submitButton.disabled = value;
      if (cancelButton) cancelButton.disabled = value;
      submitButton.replaceChildren(
        ...(value ? [h("span", { class: "spinner", "aria-hidden": "true" }), options.busyLabel ?? submitLabel] : [submitLabel]),
      );
    };
    const attemptDismiss = async () => {
      if (busy) return;
      if (options.confirmDismiss && !(await options.confirmDismiss())) return;
      finish(null);
    };

    const context: DialogContext<T> = {
      close: (result) => finish(result),
      setError,
      submit: () => form.requestSubmit(),
    };

    const error = h("div", { class: "form-error", role: "alert", hidden: true });
    const submitButton = h(
      "button",
      { type: "submit", class: ["button", options.danger ? "danger" : "primary"], "data-autofocus": options.focusCancel ? undefined : "" },
      submitLabel,
    );
    const cancelButton = options.hideCancel
      ? null
      : h(
          "button",
          { type: "button", class: "button", "data-autofocus": options.focusCancel ? "" : undefined, onClick: () => void attemptDismiss() },
          options.cancelLabel ?? "Cancel",
        );
    const body = typeof options.body === "function" ? options.body(context) : options.body;

    const form = h(
      "form",
      {
        class: "modal-form",
        novalidate: true,
        onSubmit: async (event: SubmitEvent) => {
          event.preventDefault();
          if (busy || settled) return;
          setError(null);
          setBusy(true);
          try {
            const result = await options.onSubmit();
            if (result !== undefined) finish(result);
          } catch (failure) {
            setError(errorMessage(failure));
          } finally {
            if (!settled) setBusy(false);
          }
        },
      },
      h("div", { class: "modal-body" }, body, error),
      h(
        "footer",
        { class: "modal-footer" },
        h("div", { class: "modal-footer-extra" }, options.extraActions),
        cancelButton,
        submitButton,
      ),
    );

    // Fields marked for autofocus win over the default button.
    if (form.querySelector(".modal-body [data-autofocus]")) {
      submitButton.removeAttribute("data-autofocus");
      cancelButton?.removeAttribute("data-autofocus");
    }

    const modal = openModal({
      title: options.title,
      size: options.size,
      content: form,
      onDismiss: () => {
        void attemptDismiss();
        return false;
      },
    });
  });
}

export async function confirmDialog(options: {
  title: string;
  message: Child;
  confirmLabel?: string;
  danger?: boolean;
}): Promise<boolean> {
  const result = await showDialog<true>({
    title: options.title,
    size: "small",
    body: h("div", { class: "confirm-message" }, options.message),
    submitLabel: options.confirmLabel ?? "OK",
    danger: options.danger,
    focusCancel: options.danger,
    onSubmit: () => true,
  });
  return result === true;
}

export function alertDialog(title: string, message: Child, buttonLabel = "OK"): Promise<true | null> {
  return showDialog<true>({
    title,
    size: "small",
    body: h("div", { class: "confirm-message" }, message),
    submitLabel: buttonLabel,
    hideCancel: true,
    onSubmit: () => true,
  });
}

export function promptDialog(options: {
  title: string;
  label: string;
  value?: string;
  placeholder?: string;
  submitLabel?: string;
  /** Returns an error message, or `null` if the value is acceptable. */
  validate?: (value: string) => string | null;
}): Promise<string | null> {
  const input = h("input", {
    class: "input",
    value: options.value ?? "",
    placeholder: options.placeholder,
    spellcheck: false,
    autocomplete: "off",
    "data-autofocus": "",
  });
  return showDialog<string>({
    title: options.title,
    size: "small",
    body: h("label", { class: "field" }, h("span", { class: "field-label", text: options.label }), input),
    submitLabel: options.submitLabel ?? "Save",
    onSubmit: () => {
      const value = input.value.trim();
      const problem = options.validate?.(value) ?? (value ? null : "This cannot be empty");
      if (problem) throw new Error(problem);
      return value;
    },
  });
}
