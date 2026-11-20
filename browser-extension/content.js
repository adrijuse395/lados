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
  // A field smaller than this is not one a user can use: it is hidden or a honeypot.
  const MIN_FIELD_SIZE = 8;

  let host = null;
  let shadow = null;
  let pill = null;
  let pillLabel = null;
  let pillChev = null;
  let panel = null;
  let matches = [];
  let fields = null;
  let state = "idle"; // idle | loading | ready | locked | offline | unpaired | denied
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

  /** Alpha channel of a CSS colour: 0 for `transparent`/`rgba(...,0)`, 1 when opaque. */
  function colorAlpha(color) {
    if (!color) return 1;
    const value = String(color).trim().toLowerCase();
    if (value === "transparent") return 0;
    const match = value.match(/rgba?\(([^)]+)\)/);
    if (!match) return 1;
    const parts = match[1].split(",").map((part) => part.trim());
    return parts.length >= 4 ? Number(parts[3]) : 1;
  }

  function isVisible(el) {
    if (!(el instanceof HTMLElement)) return false;
    if (el.disabled || el.readOnly) return false;

    const style = getComputedStyle(el);
    // The field itself: not display:none, not visibility:hidden, not fully transparent text
    // (the hp-color trap), and at least a real field's size.
    if (style.display === "none" || style.visibility === "hidden") return false;
    if (Number(style.opacity) === 0) return false;
    if (colorAlpha(style.color) === 0) return false;
    if (style.webkitTextFillColor && colorAlpha(style.webkitTextFillColor) === 0) return false;
    if (style.contentVisibility === "hidden") return false;

    const rect = el.getBoundingClientRect();
    // Too small to be a field (the sr-only 1x1 pattern), or moved off to the top/left
    // (the classic `position:absolute; left:-9999px` trap).
    if (rect.width < MIN_FIELD_SIZE || rect.height < MIN_FIELD_SIZE) return false;
    if (rect.right <= 0 || rect.bottom <= 0) return false;
    // Moved beyond the document itself: unreachable, so hidden. A field merely below the fold is
    // still reachable by scrolling, so it is *not* rejected here (the red-team report, N4).
    const docWidth = document.documentElement.scrollWidth || 0;
    const docHeight = document.documentElement.scrollHeight || 0;
    if (docWidth > 0 && rect.left >= docWidth) return false;
    if (docHeight > 0 && rect.top >= docHeight) return false;

    // Clipped to nothing by `clip`/`clip-path` (sr-only is a 1x1 box plus a clip).
    if (style.clip && style.clip !== "auto" && style.clip !== "none") {
      const lengths = style.clip.match(/-?\d+(\.\d+)?px/g);
      if (lengths && lengths.length === 4) {
        const [top, right, bottom, left] = lengths.map(parseFloat);
        if (right - left < MIN_FIELD_SIZE || bottom - top < MIN_FIELD_SIZE) return false;
      }
    }
    if (style.clipPath && style.clipPath !== "none") return false;

    // Every ancestor must be shown too: a parent with `opacity:0` (hp-parent-opacity), a hidden
    // one, or one that clips the field away with `overflow:hidden` and no height (hp-overflow).
    for (let node = el.parentElement; node instanceof HTMLElement; node = node.parentElement) {
      const parentStyle = getComputedStyle(node);
      if (parentStyle.display === "none" || parentStyle.visibility === "hidden") return false;
      if (Number(parentStyle.opacity) === 0) return false;
      if (/hidden|clip/.test(parentStyle.overflow + parentStyle.overflowX + parentStyle.overflowY)) {
        const box = node.getBoundingClientRect();
        const outside =
          rect.bottom <= box.top || rect.top >= box.bottom || rect.right <= box.left || rect.left >= box.right;
        if (outside) return false;
      }
    }

    // Something drawn on top of the field (hp-overlay): hit-test its centre. Only when the centre
    // is inside the viewport; a below-the-fold field is hit-tested again when it is filled.
    const cx = rect.left + rect.width / 2;
    const cy = rect.top + rect.height / 2;
    if (cx >= 0 && cy >= 0 && cx < window.innerWidth && cy < window.innerHeight) {
      const top = document.elementFromPoint(cx, cy);
      const ours = top === host || (host && host.contains(top));
      if (top && top !== el && !el.contains(top) && !ours) return false;
    }

    return true;
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
    // Re-detect now, so the fill targets a field that is visible *at this moment* (a page can move
    // the field between the scan and the click), not the one captured when the pill was drawn.
    const current = findLoginFields();
    if (!current) return;

    // Ask the app for the secret only now. If it is locked or closed, nothing
    // was ever cached in this page, so filling simply fails safely.
    const secret = await api.runtime
      .sendMessage({ type: "reveal", id: entry.id })
      .catch(() => ({ ok: false, state: "offline" }));

    if (!secret || !secret.ok) {
      matches = [];
      const known = ["locked", "unpaired", "denied"];
      state = secret && known.includes(secret.state) ? secret.state : "offline";
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
    if (state === "denied") return { cls: "locked", text: "Not filled — not allowed in Lados", chev: false };
    if (state === "unpaired") return { cls: "offline", text: "Pair the extension in Lados", chev: false };
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
    if (state === "locked" || state === "offline" || state === "denied" || state === "unpaired") openApp();
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
    } else if (response && (response.state === "locked" || response.state === "unpaired" || response.state === "offline")) {
      matches = [];
      state = response.state;
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
  // A field below the fold becomes usable only when the user scrolls to it (N4): re-scan when
  // focus lands anywhere, which covers clicking such a field.
  document.addEventListener("focusin", () => scheduleRefresh(), true);

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
