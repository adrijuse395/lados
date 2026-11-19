import { h, type Child } from "../ui/dom";
import { field, keyFileField, passwordInput } from "../ui/controls";
import { showDialog } from "../ui/modal";
import { estimateStrength } from "../utils/strength";

export interface Credentials {
  password: string;
  keyFile: string | null;
}

/**
 * Asks again for the credentials that opened the vault, before an action that cannot be undone.
 * A shared dialog keeps the three callers consistent, and a vault opened with a key file and no
 * password can still confirm here.
 */
export function confirmCredentialsDialog(title: string, message: Child): Promise<Credentials | null> {
  return credentialsDialog<Credentials>({
    title,
    message,
    submitLabel: "Confirm",
    passwordOptional: true,
    action: async (credentials) => credentials,
  });
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
  /** When set, the new password must reach this strength score (0–4). */
  minStrength?: number;
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
    // The body is a function so the shared error can be cleared as soon as the user edits anything:
    // a stale "wrong password" left under a field while it is being retyped looks like a failure
    // that has already happened (the red-team report, L12).
    body: (context) =>
      h(
        "div",
        { class: "stack", onInput: () => context.setError(null) },
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
      if (options.minStrength && credentials.password && estimateStrength(credentials.password).score < options.minStrength) {
        password.input.focus();
        throw new Error("Choose a very strong master password — it protects the whole vault.");
      }
      return { value: await options.action(credentials) };
    },
  });
  return result ? result.value : null;
}
