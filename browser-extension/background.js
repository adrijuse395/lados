// Talks to the Lados desktop app over the local loopback bridge.
// Runs in the extension's privileged context, so it can reach 127.0.0.1 even
// though web pages cannot read those responses (the bridge sends no CORS header).
const api = globalThis.browser ?? globalThis.chrome;

const BRIDGE = "http://127.0.0.1:18765";
const TIMEOUT_MS = 4000;

async function bridgeGet(path, timeoutMs = TIMEOUT_MS) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(`${BRIDGE}${path}`, {
      method: "GET",
      headers: { Accept: "application/json" },
      cache: "no-store",
      signal: controller.signal,
    });
    if (!response.ok) {
      const error = new Error(`http-${response.status}`);
      error.status = response.status;
      throw error;
    }
    return await response.json();
  } finally {
    clearTimeout(timer);
  }
}

/** Returns the app session state: "unlocked", "locked", "closed" or "offline". */
async function appState() {
  try {
    const status = await bridgeGet("/v1/status");
    return status && typeof status.state === "string" ? status.state : "offline";
  } catch {
    return "offline";
  }
}

async function findEntries(url) {
  try {
    const data = await bridgeGet(`/v1/entries?url=${encodeURIComponent(url || "")}`);
    return { ok: true, entries: Array.isArray(data.entries) ? data.entries : [] };
  } catch (error) {
    // 401 = the vault is locked; anything else means the app is not reachable.
    const state = error && error.status === 401 ? "locked" : "offline";
    return { ok: false, state, entries: [] };
  }
}

/** Fetches one entry's secrets. Called only when the user chooses to fill. */
async function revealEntry(id) {
  try {
    const data = await bridgeGet(`/v1/reveal?id=${encodeURIComponent(id || "")}`);
    return { ok: true, username: data.username, password: data.password };
  } catch (error) {
    const state = error && error.status === 401 ? "locked" : "offline";
    return { ok: false, state };
  }
}

api.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (!message || typeof message.type !== "string") return false;

  switch (message.type) {
    case "status":
      appState().then((state) => sendResponse({ ok: state === "unlocked", state }));
      return true;
    case "find":
      findEntries(message.url).then(sendResponse);
      return true;
    case "reveal":
      revealEntry(message.id).then(sendResponse);
      return true;
    default:
      return false;
  }
});
