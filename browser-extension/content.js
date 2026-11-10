// In-page autofill UI for Lados.
// Injects a small Shadow-DOM widget next to login forms and fills credentials
// from the local Lados desktop app (never talks to a server).
(() => {
  if (window.__ladosAutofillLoaded) return;
  window.__ladosAutofillLoaded = true;

  const api = globalThis.browser ?? globalThis.chrome;
  const APP_SCHEME = "lados://open";
  const DEBOUNCE_MS = 300;
  const SHIELD_PATH =
    "M12 2 4 5v6c0 5 3.4 9.4 8 11 4.6-1.6 8-6 8-11V5l-8-3zm0 3.2 4.8 1.8v4c0 3.3-2 6.5-4.8 7.8-2.8-1.3-4.8-4.5-4.8-7.8v-4L12 5.2z";
  const SVG_NS = "http://www.w3.org/2000/svg";

  let host = null;
  let shadow = null;
  let pill = null;
  let pillLabel = null;
  let pillChev = null;
  let panel = null;
  let matches = [];
  let fields = null;
  let state = "idle"; // idle | loading | ready | locked | offline
  let debounce = null;
  let lastUrl = location.href;
  let positionRaf = 0;

  // -------------------------------------------------------------------------
  // Small DOM helpers built with DOM APIs only
  // -------------------------------------------------------------------------

  function shieldSvg() {
    const svg = document.createElementNS(SVG_NS, "svg");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("fill", "currentColor");
    svg.setAttribute("aria-hidden", "true");
    const path = document.createElementNS(SVG_NS, "path");
    path.setAttribute("d", SHIELD_PATH);
    svg.append(path);
    return svg;
  }

  function logo(className) {
    const span = document.createElement("span");
    span.className = className;
    span.append(shieldSvg());
    return span;
  }

  // -------------------------------------------------------------------------
  // Field detection
  // -------------------------------------------------------------------------

  function isVisible(el) {
    if (!(el instanceof HTMLElement)) return false;
    if (el.disabled || el.readOnly) return false;
    const style = getComputedStyle(el);
    if (style.display === "none" || style.visibility === "hidden") return false;
    return !!(el.offsetWidth || el.offsetHeight || el.getClientRects().length);
  }

  function pickUsername(scope) {
    const candidates = scope.querySelectorAll(
      'input[type="text"], input[type="email"], input[type="tel"], input:not([type])',
    );
    let fallback = null;
    for (const el of candidates) {
      if (!isVisible(el)) continue;
      if (/user|email|login|account|phone/i.test(el.name + " " + el.id + " " + el.autocomplete)) return el;
      if (/user|email|login|account/i.test(el.getAttribute("aria-label") || "")) return el;
      if (el.autocomplete === "username") return el;
      if (!fallback && el.autocomplete !== "off") fallback = el;
    }
    return fallback;
  }

  function findLoginFields() {
    const passwords = [...document.querySelectorAll('input[type="password"]')].filter(isVisible);
    if (passwords.length === 0) return null;
    const password = passwords[0];
    const scope = password.form || password.closest("form") || document;
    return { password, username: pickUsername(scope) };
  }

  // -------------------------------------------------------------------------
  // Filling (React/Vue aware)
  // -------------------------------------------------------------------------

  function setNativeValue(el, value) {
    const proto = Object.getPrototypeOf(el);
    const descriptor = Object.getOwnPropertyDescriptor(proto, "value");
    if (descriptor && descriptor.set) {
      descriptor.set.call(el, value);
    } else {
      el.value = value;
    }
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  }

  async function fill(index) {
    const entry = matches[index];
    if (!entry) return;
    const current = findLoginFields() || fields;
    if (!current) return;

    // Ask the app for the secret only now. If it is locked or closed, nothing
    // was ever cached in this page, so filling simply fails safely.
    const secret = await api.runtime
      .sendMessage({ type: "reveal", id: entry.id })
      .catch(() => ({ ok: false, state: "offline" }));

    if (!secret || !secret.ok) {
      matches = [];
      state = secret && secret.state === "locked" ? "locked" : "offline";
      renderPill();
      return;
    }

    if (current.username && secret.username) setNativeValue(current.username, secret.username);
    if (current.password && secret.password != null) setNativeValue(current.password, secret.password);
    hidePanel();
  }

  // -------------------------------------------------------------------------
  // Shadow-DOM UI
  // -------------------------------------------------------------------------

  const CSS = `
    :host { all: initial; }
    * { box-sizing: border-box; }
    .pill {
      display: inline-flex; align-items: center; gap: 7px;
      padding: 5px 10px 5px 8px; border-radius: 8px;
      background: #111418; color: #f4f5f7;
      font: 600 12.5px/1.1 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, system-ui, sans-serif;
      box-shadow: 0 3px 12px rgba(0,0,0,.28); border: 1px solid rgba(255,255,255,.08);
      cursor: pointer; user-select: none; white-space: nowrap; max-width: 280px;
    }
    .pill:hover { background: #1c2129; }
    .pill .logo { width: 15px; height: 15px; color: #f4f5f7; flex: 0 0 auto; display: inline-flex; }
    .pill .label { overflow: hidden; text-overflow: ellipsis; }
    .pill .chev { opacity: .55; font-size: 10px; }
    .pill.locked { background: #2a2010; }
    .pill.offline { background: #241a1a; }
    .panel {
      margin-top: 6px; min-width: 232px; max-width: 320px;
      background: #ffffff; color: #111418; border: 1px solid #d8dde3;
      border-radius: 10px; box-shadow: 0 8px 28px rgba(0,0,0,.22);
      font: 13px/1.4 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, system-ui, sans-serif;
      overflow: hidden;
    }
    .panel[hidden] { display: none; }
    .row { display: flex; align-items: center; gap: 9px; padding: 9px 11px; cursor: pointer; }
    .row:hover { background: #f1f3f6; }
    .row .logo { width: 18px; height: 18px; color: #111418; flex: 0 0 auto; display: inline-flex; }
    .row .txt { min-width: 0; }
    .row .title { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .row .sub { font-size: 11px; color: #5c6570; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .foot { padding: 7px 11px; font-size: 11.5px; color: #5c6570; text-align: center; cursor: pointer; }
    .foot:hover { background: #f1f3f6; }
    @media (prefers-color-scheme: dark) {
      .panel { background: #161a21; color: #f4f5f7; border-color: #2a2f38; }
      .row:hover, .foot:hover { background: #1e242d; }
      .row .sub, .foot { color: #9aa4b2; }
      .row .logo { color: #f4f5f7; }
    }
  `;

  function ensureRoot() {
    if (host) return;
    host = document.createElement("div");
    host.id = "lados-autofill-root";
    host.style.cssText = "all:initial;position:fixed;z-index:2147483647;top:0;left:0;";
    shadow = host.attachShadow({ mode: "open" });

    const style = document.createElement("style");
    style.textContent = CSS;

    pill = document.createElement("div");
    pill.className = "pill";
    pill.hidden = true;
    const pillLogo = logo("logo");
    pillLabel = document.createElement("span");
    pillLabel.className = "label";
    pill.append(pillLogo, pillLabel);

    panel = document.createElement("div");
    panel.className = "panel";
    panel.hidden = true;

    shadow.append(style, pill, panel);
    pill.addEventListener("click", onPillClick);

    document.documentElement.append(host);
  }

  function pillContent() {
    if (state === "loading") return { cls: "", text: "Checking Lados…", chev: false };
    if (state === "locked") return { cls: "locked", text: "Lados is locked — unlock to fill", chev: false };
    if (state === "offline") return { cls: "offline", text: "Open Lados to fill", chev: false };
    const n = matches.length;
    return { cls: "", text: n === 1 ? "Fill with Lados" : `${n} Lados logins`, chev: n > 1 };
  }

  function renderPill() {
    ensureRoot();
    const { cls, text, chev } = pillContent();
    pill.className = `pill ${cls}`.trim();
    pillLabel.textContent = text;
    if (chev && !pillChev) {
      pillChev = document.createElement("span");
      pillChev.className = "chev";
      pillChev.textContent = "▾";
      pill.append(pillChev);
    } else if (!chev && pillChev) {
      pillChev.remove();
      pillChev = null;
    }
    pill.hidden = false;
    renderPanel();
    positionPill();
  }

  function renderPanel() {
    if (!panel) return;
    if (state !== "ready" || matches.length <= 1) {
      panel.hidden = true;
      return;
    }
    panel.replaceChildren();
    matches.forEach((entry, index) => {
      const row = document.createElement("div");
      row.className = "row";

      const txt = document.createElement("div");
      txt.className = "txt";
      const title = document.createElement("div");
      title.className = "title";
      title.textContent = entry.title || "Untitled";
      const sub = document.createElement("div");
      sub.className = "sub";
      sub.textContent = entry.username || entry.url || "";
      txt.append(title, sub);

      row.append(logo("logo"), txt);
      row.addEventListener("click", () => fill(index));
      panel.append(row);
    });

    const foot = document.createElement("div");
    foot.className = "foot";
    foot.textContent = "Open Lados";
    foot.addEventListener("click", openApp);
    panel.append(foot);

    panel.hidden = true;
  }

  function showPanel() {
    if (!panel || panel.children.length === 0) return;
    panel.hidden = false;
  }

  function hidePanel() {
    if (panel) panel.hidden = true;
  }

  function positionPill() {
    if (!host) return;
    const anchor = (findLoginFields() || fields)?.password;
    if (!anchor) return;
    const rect = anchor.getBoundingClientRect();
    const width = pill ? pill.offsetWidth : 160;
    const left = Math.max(6, Math.min(window.innerWidth - width - 6, rect.left));
    const top = Math.max(6, rect.top - 40);
    host.style.left = `${left}px`;
    host.style.top = `${top}px`;
  }

  function teardown() {
    if (host) host.remove();
    host = shadow = pill = pillLabel = pillChev = panel = null;
    matches = [];
    state = "idle";
  }

  // -------------------------------------------------------------------------
  // Actions
  // -------------------------------------------------------------------------

  function openApp() {
    // Custom-scheme link; the OS opens (or focuses) the Lados desktop app.
    try {
      window.location.href = APP_SCHEME;
    } catch {
      /* ignore */
    }
  }

  function onPillClick() {
    if (state === "ready") {
      if (matches.length === 1) fill(0);
      else if (panel && panel.hidden) showPanel();
      else hidePanel();
      return;
    }
    if (state === "locked" || state === "offline") openApp();
  }

  // -------------------------------------------------------------------------
  // Refresh cycle
  // -------------------------------------------------------------------------

  async function refresh() {
    fields = findLoginFields();
    if (!fields) {
      teardown();
      return;
    }

    state = "loading";
    renderPill();

    const response = await api.runtime
      .sendMessage({ type: "find", url: location.href })
      .catch(() => ({ ok: false, state: "offline", entries: [] }));

    // The page may have changed while we waited.
    if (location.href !== lastUrl) return;

    if (response && response.ok && response.entries.length > 0) {
      matches = response.entries;
      state = "ready";
      renderPill();
    } else if (response && response.state === "locked") {
      matches = [];
      state = "locked";
      renderPill();
    } else if (response && response.state === "offline") {
      matches = [];
      state = "offline";
      renderPill();
    } else {
      // No saved login for this page: stay out of the way.
      teardown();
    }
  }

  function scheduleRefresh() {
    if (debounce) clearTimeout(debounce);
    debounce = setTimeout(refresh, DEBOUNCE_MS);
  }

  function onLocationChange() {
    if (location.href === lastUrl) return;
    lastUrl = location.href;
    hidePanel();
    scheduleRefresh();
  }

  function onScrollOrResize() {
    if (positionRaf) return;
    positionRaf = requestAnimationFrame(() => {
      positionRaf = 0;
      positionPill();
    });
  }

  function start() {
    ensureRoot();
    refresh();
  }

  // React to explicit navigation, then rely on a light observer for SPA route changes.
  window.addEventListener("popstate", onLocationChange);
  window.addEventListener("hashchange", onLocationChange);
  const observer = new MutationObserver(onLocationChange);
  observer.observe(document, { subtree: true, childList: true });
  window.addEventListener("scroll", onScrollOrResize, { passive: true });
  window.addEventListener("resize", onScrollOrResize, { passive: true });

  // Re-check whenever the tab regains focus, so a Lados that was locked or closed
  // since the last visit is reflected instead of showing stale candidates.
  window.addEventListener("focus", () => {
    hidePanel();
    scheduleRefresh();
  });
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) {
      hidePanel();
      scheduleRefresh();
    }
  });

  start();
})();
