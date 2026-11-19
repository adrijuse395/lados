import { append, h, type Child } from "./dom";
import { svgIcon, type IconName } from "./icons";

export type ToastKind = "info" | "success" | "error";

/** A button shown inside a toast, such as "Undo" after a delete. */
export interface ToastAction {
  label: string;
  run: () => void;
}

const ICONS: Record<ToastKind, IconName> = { info: "info", success: "check", error: "warning" };

let container: HTMLElement | null = null;

export function toast(message: string, kind: ToastKind = "info", durationMs = 3500, action?: ToastAction): void {
  container ??= document.body.appendChild(h("div", { class: "toasts", role: "status", "aria-live": "polite" }));
  const element = h("div", { class: ["toast", `toast-${kind}`] });
  const children: Child[] = [svgIcon(ICONS[kind]), h("span", { text: message })];
  const dismiss = () => {
    element.classList.remove("visible");
    window.setTimeout(() => element.remove(), 300);
  };
  if (action) {
    children.push(
      h(
        "button",
        {
          type: "button",
          class: "toast-action",
          onClick: () => {
            action.run();
            dismiss();
          },
        },
        action.label,
      ),
    );
  }
  append(element, ...children);
  container.append(element);
  requestAnimationFrame(() => element.classList.add("visible"));
  window.setTimeout(dismiss, durationMs);
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return typeof error === "string" ? error : "Something went wrong";
}

export function toastError(error: unknown): void {
  toast(errorMessage(error), "error", 6000);
}
