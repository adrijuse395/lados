// Publishes the unlocked vault to the Android AutofillService (in-memory only).
// On desktop there is no LadosAutofill interface — these calls are no-ops.

import { backend } from "./backend";

type AutofillNative = {
  publish(json: string): void;
  clear(): void;
  isReady?(): boolean;
};

function nativeBridge(): AutofillNative | null {
  const bridge = (window as unknown as { LadosAutofill?: AutofillNative }).LadosAutofill;
  return bridge ?? null;
}

/** Push current vault credentials into the Android autofill cache. */
export async function syncAutofillCache(): Promise<void> {
  const bridge = nativeBridge();
  if (!bridge) return;
  try {
    const payload = await backend.getAutofillPayload();
    bridge.publish(JSON.stringify(payload));
  } catch {
    bridge.clear();
  }
}

/** Wipe the autofill cache (lock / close / quit). */
export function clearAutofillCache(): void {
  nativeBridge()?.clear();
}
