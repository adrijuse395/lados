import { h, type Child } from "../ui/dom";
import { field, keyFileField, passwordInput } from "../ui/controls";
import { showDialog } from "../ui/modal";

export interface Credentials {
  password: string;
  keyFile: string | null;
}

/**
 * Asks for a password and an optional key file, then runs `action` while the dialog stays
 * open, so a wrong password can be corrected on the spot.
 */
export async function credentialsDialog<T>(options: {
  title: string;
  message?: Child;
  submitLabel: string;
  busyLabel?: string;
  /** Asks twice and shows the strength, for protecting something new. */
  newSecret?: boolean;
  allowCreateKeyFile?: boolean;
  /** An empty password is accepted when a key file is chosen (KeePass allows it). */
  passwordOptional?: boolean;
  initialKeyFile?: string | null;
  action: (credentials: Credentials) => Promise<T>;
}): Promise<T | null> {
  const password = passwordInput({
    autofocus: true,
    strength: options.newSecret,
    placeholder: options.newSecret ? "Choose a strong password" : "Password",
  });
  const confirmation = options.newSecret ? passwordInput({ placeholder: "Type it again" }) : null;
  const keyFile = keyFileField({ value: options.initialKeyFile ?? null, allowCreate: options.allowCreateKeyFile });

  const result = await showDialog<{ value: T }>({
    title: options.title,
    size: "medium",
    body: h(
      "div",
      { class: "stack" },
      options.message ? h("div", { class: "dialog-message" }, options.message) : null,
      field(options.newSecret ? "New password" : "Password", password.element, { target: password.input }),
      confirmation ? field("Confirm the password", confirmation.element, { target: confirmation.input }) : null,
      field("Key file", keyFile.element, {
        hint: options.newSecret ? "Optional. If you use one, it will be needed together with the password." : "Only if it was protected with one.",
      }),
    ),
    submitLabel: options.submitLabel,
    busyLabel: options.busyLabel,
    onSubmit: async () => {
      const credentials = { password: password.value(), keyFile: keyFile.value() };
      if (!credentials.password && !(options.passwordOptional && credentials.keyFile)) {
        password.input.focus();
        throw new Error("Enter the password");
      }
      if (confirmation && confirmation.value() !== credentials.password) {
        confirmation.input.focus();
        throw new Error("The passwords do not match");
      }
      return { value: await options.action(credentials) };
    },
  });
  return result ? result.value : null;
}
