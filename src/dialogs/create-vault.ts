import { backend } from "../services/backend";
import type { AppStatus } from "../types";
import { field, fileNameOf, keyFileField, passwordInput } from "../ui/controls";
import { h } from "../ui/dom";
import { svgIcon } from "../ui/icons";
import { confirmDialog, showDialog } from "../ui/modal";
import { estimateStrength } from "../utils/strength";

/** Characters that Windows does not accept in file names. */
export function safeFileName(name: string, fallback: string): string {
  const cleaned = name.replace(/[\\/:*?"<>|]/g, "-").trim();
  return cleaned || fallback;
}

export async function createVaultDialog(): Promise<AppStatus | null> {
  const mobile = await backend.isMobileShell();
  let location: string | null = null;
  /** When true, renaming the vault updates the standard app vaults path. */
  let usingSuggestedPath = mobile;

  const name = h("input", {
    class: "input",
    value: "My vault",
    spellcheck: false,
    autocomplete: "off",
    "data-autofocus": "",
  });
  const locationName = h("span", { class: "location-name", text: "Not chosen yet" });
  const locationPath = h("span", { class: "location-path" });

  const showLocation = (path: string) => {
    location = path;
    locationName.textContent = fileNameOf(path);
    locationPath.textContent = path;
  };

  const applySuggestedPath = async () => {
    const path = await backend.suggestedVaultPath(name.value.trim() || "My vault");
    usingSuggestedPath = true;
    showLocation(path);
  };

  const chooseLocation = async (): Promise<boolean> => {
    if (mobile) {
      // Android save pickers return content:// URIs that std::fs cannot write.
      await applySuggestedPath();
      return true;
    }
    const suggested = `${safeFileName(name.value, "Vault")}.lados`;
    const picked = await backend.pickSavePath("vault", "Where should the vault be saved?", suggested);
    if (picked) {
      usingSuggestedPath = false;
      showLocation(picked);
    }
    return picked !== null;
  };

  if (mobile) {
    await applySuggestedPath();
  }

  name.addEventListener("input", () => {
    if (usingSuggestedPath) void applySuggestedPath().catch(() => undefined);
  });

  const locationButton = mobile
    ? null
    : h("button", { type: "button", class: "button small", onClick: () => void chooseLocation() }, "Choose…");

  const password = passwordInput({ strength: true, placeholder: "Choose a strong password" });
  const confirmation = passwordInput({ placeholder: "Type it again" });
  const keyFile = keyFileField({ allowCreate: true });

  return showDialog<AppStatus>({
    title: "Create a new vault",
    size: "medium",
    body: h(
      "div",
      { class: "stack" },
      field("Vault name", name),
      field(
        "File",
        h(
          "div",
          { class: "location-field" },
          svgIcon("folder"),
          h("div", { class: "location-text" }, locationName, locationPath),
          locationButton,
        ),
        {
          ...(locationButton ? { target: locationButton } : {}),
          ...(mobile
            ? {
                hint: "Saved in Documents › Lados on this phone. That folder stays even if you uninstall the app.",
              }
            : {}),
        },
      ),
      field("Master password", password.element, { target: password.input }),
      field("Confirm the master password", confirmation.element, { target: confirmation.input }),
      field("Key file", keyFile.element, {
        hint: "Optional second factor: a file that must be present, together with the password, to open the vault.",
      }),
      h(
        "div",
        { class: "notice warning" },
        svgIcon("warning"),
        h("span", {
          text: "Nobody can recover a forgotten master password or a lost key file, not even Lados. Keep them safe.",
        }),
      ),
    ),
    submitLabel: "Create vault",
    busyLabel: "Creating…",
    onSubmit: async () => {
      if (!name.value.trim()) {
        name.focus();
        throw new Error("Give the vault a name");
      }
      if (!password.value()) {
        password.input.focus();
        throw new Error("Choose a master password");
      }
      if (password.value() !== confirmation.value()) {
        confirmation.input.focus();
        throw new Error("The passwords do not match");
      }
      if (!location) {
        if (mobile) await applySuggestedPath();
        else if (!(await chooseLocation())) throw new Error("Choose where to save the vault");
      }
      if (!location) throw new Error("Choose where to save the vault");
      if (estimateStrength(password.value()).score < 2) {
        const proceed = await confirmDialog({
          title: "Weak master password",
          message: "This password would be easy to guess. Use it anyway?",
          confirmLabel: "Use it anyway",
          danger: true,
        });
        if (!proceed) return undefined;
      }
      return backend.createVault(location, name.value.trim(), password.value(), keyFile.value());
    },
  });
}
