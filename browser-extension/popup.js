const api = globalThis.browser ?? globalThis.chrome;

const statusEl = document.getElementById("status");
const msgEl = document.getElementById("msg");
const openBtn = document.getElementById("open");

function render(kind, message, showOpen) {
  statusEl.className = kind;
  msgEl.textContent = message;
  openBtn.hidden = !showOpen;
}

async function refresh() {
  let response;
  try {
    response = await api.runtime.sendMessage({ type: "status" });
  } catch {
    response = null;
  }

  if (response && response.ok) {
    render("ok", "Connected — vault unlocked", false);
    return;
  }

  const state = response && response.state;
  if (state === "locked") {
    render("warn", "Lados is locked", true);
    openBtn.textContent = "Unlock Lados";
  } else if (state === "closed") {
    render("warn", "No vault is open in Lados", true);
  } else {
    render("off", "Lados is not running", true);
    openBtn.textContent = "Open Lados";
  }
}

openBtn.addEventListener("click", () => {
  // Custom-scheme link; the OS opens or focuses the Lados desktop app.
  window.location.href = "lados://open";
});

refresh();
