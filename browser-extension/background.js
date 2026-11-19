// Talks to the Lados desktop app over the local loopback bridge.
//
// Two things are needed to reach it: the app must be running with the vault unlocked, and this
// extension must have been *paired* with a code the user copies from Lados ▸ Settings. The code
// keeps web pages, other extensions and scripts on the machine out; handing over a password is
// still confirmed by the user inside Lados, which no program here can click.
const api = globalThis.browser ?? globalThis.chrome;

const BRIDGE = "http://127.0.0.1:18765";
const TIMEOUT_MS = 4000;
const TOKEN_KEY = "ladosPairingToken";

/** The pairing code the user copied from Lados, or "" when this extension is not paired. */
async function pairingToken() {
  try {
    const stored = await api.storage.local.get(TOKEN_KEY);
    const token = stored && stored[TOKEN_KEY];
    return typeof token === "string" ? token : "";
  } catch {
    return "";
  }
}

async function saveToken(token) {
  const value = typeof token === "string" ? token.trim() : "";
  try {
    if (value) await api.storage.local.set({ [TOKEN_KEY]: value });
    else await api.storage.local.remove(TOKEN_KEY);
  } catch {
    // Storage unavailable: the extension simply stays unpaired.
  }
}

async function bridgeGet(path, timeoutMs = TIMEOUT_MS) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const headers = { Accept: "application/json" };
    const token = await pairingToken();
    if (token) headers["X-Lados-Token"] = token;
    const response = await fetch(`${BRIDGE}${path}`, {
      method: "GET",
      headers,
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

/** Returns the app session state: "unlocked", "locked", "closed", "unpaired" or "offline". */
async function appState() {
  try {
    const status = await bridgeGet("/v1/status");
    return status && typeof status.state === "string" ? status.state : "offline";
  } catch (error) {
    // 403 means the app refused us: no pairing code, or a code it no longer recognises.
    if (error && error.status === 403) return "unpaired";
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

/** Fetches one entry's secrets. The user allows it inside Lados before anything is sent. */
async function revealEntry(id) {
  try {
    const data = await bridgeGet(`/v1/reveal?id=${encodeURIComponent(id || "")}`, 32000);
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
    case "pair":
      saveToken(message.token).then(() => appState().then((state) => sendResponse({ state })));
      return true;
    case "unpair":
      saveToken("").then(() => sendResponse({ state: "unpaired" }));
      return true;
    default:
      return false;
  }
});
