const api = globalThis.browser ?? globalThis.chrome;

const statusEl = document.getElementById("status");
const msgEl = document.getElementById("msg");
const openBtn = document.getElementById("open");
const pairBox = document.getElementById("pair");
const tokenInput = document.getElementById("token");
const pairBtn = document.getElementById("pairBtn");

function render(kind, message, actionLabel) {
  statusEl.className = kind;
  msgEl.textContent = message;
  openBtn.hidden = !actionLabel;
  if (actionLabel) openBtn.textContent = actionLabel;
}

function showPairing(show) {
  pairBox.hidden = !show;
}

async function refresh() {
  let response;
  try {
    response = await api.runtime.sendMessage({ type: "status" });
  } catch {
    response = null;
  }

  const state = response && response.state;

  // The app refused us (no code, or a code it no longer recognises).
  if (state === "unpaired") {
    render("warn", "Not paired with Lados", null);
    showPairing(true);
    return;
  }

  showPairing(false);

  if (response && response.ok) {
    render("ok", "Connected — vault unlocked", null);
    return;
  }

  if (state === "locked") {
    render("warn", "Lados is locked", "Unlock Lados");
  } else if (state === "closed") {
    render("warn", "No vault is open in Lados", "Open Lados");
  } else {
    render("off", "Lados is not running", "Open Lados");
  }
}

openBtn.addEventListener("click", () => {
  // Custom-scheme link; the OS opens or focuses the Lados desktop app.
  window.location.href = "lados://open";
});

pairBtn.addEventListener("click", async () => {
  const token = tokenInput.value.trim();
  if (!token) {
    tokenInput.focus();
    return;
  }
  pairBtn.disabled = true;
  let response = null;
  try {
    response = await api.runtime.sendMessage({ type: "pair", token });
  } catch {
    response = null;
  }
  pairBtn.disabled = false;

  const state = response && response.state;
  if (state === "unlocked" || state === "locked" || state === "closed") {
    tokenInput.value = "";
    await refresh();
  } else {
    render("off", "That code was refused — check it in Lados ▸ Settings", null);
    showPairing(true);
  }
});

tokenInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") void pairBtn.click();
});

refresh();
