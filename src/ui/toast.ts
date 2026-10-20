import { h } from "./dom";
import { svgIcon, type IconName } from "./icons";

export type ToastKind = "info" | "success" | "error";

const ICONS: Record<ToastKind, IconName> = { info: "info", success: "check", error: "warning" };

let container: HTMLElement | null = null;

export function toast(message: string, kind: ToastKind = "info", durationMs = 3500): void {
  container ??= document.body.appendChild(h("div", { class: "toasts", role: "status", "aria-live": "polite" }));
  const element = h("div", { class: ["toast", `toast-${kind}`] }, svgIcon(ICONS[kind]), h("span", { text: message }));
  container.append(element);
  requestAnimationFrame(() => element.classList.add("visible"));
  window.setTimeout(() => {
    element.classList.remove("visible");
    window.setTimeout(() => element.remove(), 300);
  }, durationMs);
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return typeof error === "string" ? error : "Something went wrong";
}

export function toastError(error: unknown): void {
  toast(errorMessage(error), "error", 6000);
}
