// Shown when a vault is selected but locked.

import { showStatus } from "../app";
import { backend, isCredentialError } from "../services/backend";
import { store } from "../state/store";
import { field, keyFileField, passwordInput } from "../ui/controls";
import { h, mount } from "../ui/dom";
import { logo } from "../ui/icons";
import { errorMessage, toastError } from "../ui/toast";

export function renderUnlock(container: HTMLElement): void {
  const status = store.status;
  const password = passwordInput({ placeholder: "Master password" });
  const keyFile = keyFileField({ value: status?.keyFile ?? null });
  const error = h("p", { class: "form-error", role: "alert", hidden: true });
  const submit = h("button", { type: "submit", class: "button primary large block" }, "Unlock");

  const setBusy = (busy: boolean) => {
    submit.disabled = busy;
    password.input.disabled = busy;
    submit.replaceChildren(...(busy ? [h("span", { class: "spinner", "aria-hidden": "true" }), "Unlocking…"] : ["Unlock"]));
  };
  const showError = (message: string) => {
    error.textContent = message;
    error.hidden = false;
    card.classList.remove("shake");
    void card.offsetWidth;
    card.classList.add("shake");
  };

  const unlock = async () => {
    if (!password.value()) {
      showError("Enter the master password");
      password.input.focus();
      return;
    }
    error.hidden = true;
    setBusy(true);
    try {
      const unlocked = await backend.unlockVault(password.value(), keyFile.value());
      password.setValue("");
      await showStatus(unlocked);
    } catch (failure) {
      if (!card.isConnected) return;
      setBusy(false);
      showError(isCredentialError(failure) ? "Wrong master password or key file" : errorMessage(failure));
      password.input.focus();
      password.input.select();
    }
  };

  const openAnother = async () => {
    try {
      const path = await backend.pickOpenPath("vault", "Open a vault");
      if (path) await showStatus(await backend.selectVault(path));
    } catch (failure) {
      toastError(failure);
    }
  };
  const backToStart = async () => {
    try {
      await showStatus(await backend.closeVault());
    } catch (failure) {
      toastError(failure);
    }
  };

  const card = h(
    "form",
    {
      class: "card unlock-card",
      novalidate: true,
      onSubmit: (event: SubmitEvent) => {
        event.preventDefault();
        void unlock();
      },
    },
    h(
      "div",
      { class: "unlock-header" },
      logo("logo medium"),
      h(
        "div",
        { class: "unlock-title" },
        h("h1", { text: status?.vaultFileName ?? "Vault" }),
        h("p", { class: "unlock-path", title: status?.vaultPath ?? "", text: status?.vaultPath ?? "" }),
      ),
    ),
    field("Master password", password.element, { target: password.input }),
    field("Key file", keyFile.element, { hint: "Only needed if the vault was created with one." }),
    error,
    submit,
    h(
      "div",
      { class: "unlock-links" },
      h("button", { type: "button", class: "link-button", onClick: () => void openAnother() }, "Open another vault…"),
      h("button", { type: "button", class: "link-button", onClick: () => void backToStart() }, "Back to start"),
    ),
  );

  mount(container, h("main", { class: "center-screen unlock-screen" }, card));
  requestAnimationFrame(() => password.input.focus());
}
