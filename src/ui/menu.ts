// Drop-down and context menus.

import { h } from "./dom";
import { svgIcon, type IconName } from "./icons";

export interface MenuItem {
  label: string;
  icon?: IconName;
  shortcut?: string;
  danger?: boolean;
  disabled?: boolean;
  onSelect(): void;
}

export type MenuEntry = MenuItem | "separator";

interface OpenMenu {
  close(restoreFocus: boolean): void;
}

let current: OpenMenu | null = null;

export function closeMenu(): void {
  current?.close(false);
}

/** Opens a menu below `anchor`, or at the pointer position for context menus. */
export function openMenu(anchor: HTMLElement | MouseEvent, entries: MenuEntry[]): void {
  closeMenu();
  const returnFocus = anchor instanceof HTMLElement ? anchor : document.activeElement;

  const items: HTMLButtonElement[] = [];
  const menu = h(
    "div",
    { class: "menu", role: "menu" },
    entries.map((entry) => {
      if (entry === "separator") return h("div", { class: "menu-separator", role: "separator" });
      const item = h(
        "button",
        {
          type: "button",
          class: ["menu-item", entry.danger && "danger"],
          role: "menuitem",
          disabled: entry.disabled,
          onClick: () => {
            close(true);
            entry.onSelect();
          },
        },
        entry.icon ? svgIcon(entry.icon) : h("span", { class: "icon" }),
        h("span", { class: "menu-label", text: entry.label }),
        entry.shortcut ? h("kbd", { text: entry.shortcut }) : null,
      );
      if (!entry.disabled) items.push(item);
      return item;
    }),
  );
  document.body.append(menu);
  position(menu, anchor);

  const onPointerDown = (event: PointerEvent) => {
    if (!menu.contains(event.target as Node)) close(false);
  };
  const onKeyDown = (event: KeyboardEvent) => {
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === "Escape") {
      close(true);
    } else if (event.key === "ArrowDown") {
      items[(index + 1) % items.length]?.focus();
    } else if (event.key === "ArrowUp") {
      items[(index - 1 + items.length) % items.length]?.focus();
    } else if (event.key === "Home") {
      items[0]?.focus();
    } else if (event.key === "End") {
      items[items.length - 1]?.focus();
    } else if (event.key === "Tab") {
      close(false);
      return;
    } else {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
  };
  const onDismiss = () => close(false);

  function close(restoreFocus: boolean) {
    if (current !== handle) return;
    current = null;
    menu.remove();
    window.removeEventListener("pointerdown", onPointerDown, true);
    window.removeEventListener("keydown", onKeyDown, true);
    window.removeEventListener("resize", onDismiss);
    window.removeEventListener("blur", onDismiss);
    document.removeEventListener("scroll", onDismiss, true);
    if (restoreFocus && returnFocus instanceof HTMLElement && returnFocus.isConnected) returnFocus.focus();
  }

  const handle: OpenMenu = { close };
  current = handle;
  window.addEventListener("pointerdown", onPointerDown, true);
  window.addEventListener("keydown", onKeyDown, true);
  window.addEventListener("blur", onDismiss);
  // Scrolling caused while opening the menu, such as revealing the row it belongs to, is
  // reported in the next frame and must not close it.
  requestAnimationFrame(() => {
    if (current !== handle) return;
    window.addEventListener("resize", onDismiss);
    document.addEventListener("scroll", onDismiss, true);
  });
  items[0]?.focus();
}

function position(menu: HTMLElement, anchor: HTMLElement | MouseEvent): void {
  const margin = 8;
  const { width, height } = menu.getBoundingClientRect();
  let left: number;
  let top: number;
  if (anchor instanceof HTMLElement) {
    const rect = anchor.getBoundingClientRect();
    left = rect.right - width;
    top = rect.bottom + 4;
    if (top + height > window.innerHeight - margin) top = rect.top - height - 4;
  } else {
    left = anchor.clientX;
    top = anchor.clientY;
    if (top + height > window.innerHeight - margin) top = anchor.clientY - height;
  }
  menu.style.left = `${Math.max(margin, Math.min(left, window.innerWidth - width - margin))}px`;
  menu.style.top = `${Math.max(margin, Math.min(top, window.innerHeight - height - margin))}px`;
}
