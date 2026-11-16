# Store listing copy — Lados browser extension

Copy‑paste fields for the **Chrome Web Store** and **Microsoft Edge Add‑ons** dashboards.
Firefox (AMO) is already published; its fields are the same.

- Package: `browser-extension/build/lados-chrome.zip` (rebuild with `python3 scripts/build-extension.py`)
- Version: `0.3.0` (from `manifest.chrome.json`)
- Icon: `browser-extension/icons/icon-128.png`

---

## Fields

**Name**
```
Lados
```

**Summary / short description** (≤132 characters)
```
Autofill logins from your local Lados vault. Offline, no account, no cloud.
```

**Category** — `Productivity`

**Language** — `English`

**Homepage URL**
```
https://github.com/adrijuse395/lados
```

**Detailed description**
```
Lados fills your logins from a password vault that stays on your computer.

• Works with the free Lados desktop app (Windows, macOS, Linux and Android).
• Reads your local vault only while it is unlocked — nothing is synced or sent anywhere.
• No account, no cloud, no telemetry. The extension talks only to the Lados app on the
  same computer, over a loopback connection (127.0.0.1).
• A password is fetched only at the moment you click to fill, and is never stored in the page.

Install the Lados app, unlock your vault, then use this extension to fill login forms.
```

---

## Chrome Web Store → “Privacy practices”

**Single purpose**
```
Fill username and password fields on web pages from the user's own local Lados vault.
```

**Justification — host permission `http://127.0.0.1:18765/*`**
```
Communicate only with the Lados desktop application running on the same computer. This is
the local loopback bridge; no external server is ever contacted.
```

**Justification — content script on all sites (`http://*/*`, `https://*/*`)**
```
Detect login forms on the page the user is viewing so they can be filled on request from
their local vault. The extension reads only the page URL and the login fields; it does not
read or transmit page content.
```

**Remote code** — `No, I am not using remote code`
```
All code is bundled in the package. Nothing is downloaded or evaluated at runtime.
```

**Data usage** — declare **no data collected**:
```
This extension collects no user data. Nothing is transmitted off the device; there are no
analytics, no cookies, and no local storage.
Check none of the "What user data do you plan to collect?" boxes.
```

**Privacy policy URL** — see `docs/PRIVACY.md` (must be a public URL; see below).

### Permission summary (for reference)

| Requested | Why |
| --- | --- |
| `host_permissions: http://127.0.0.1:18765/*` | Talk to the local Lados app only |
| Content script `http(s)://*/*` | Find login forms on the page you are on |
| `permissions: []` | Nothing else — no `storage`, `tabs`, `scripting`, etc. |

---

## Microsoft Edge Add‑ons

Same fields. Additionally:

**Search terms**
```
password manager, autofill, offline, local, privacy, lados
```

**Category** — `Productivity`

---

## Publishing checklist

- [x] Build the package — `python3 scripts/build-extension.py` → `build/lados-chrome.zip`
- [x] Store icon 128×128 — `icons/icon-128.png`
- [x] `manifest.json` at the zip root (required)
- [x] No remote code; `permissions: []`; loopback `host_permissions` only
- [ ] **Screenshots** — at least one, 1280×800 or 640×400 (see below)
- [ ] **Privacy policy URL** live on the web (see below)
- [ ] Chrome Web Store: pay the one‑time $5 developer fee
- [ ] Edge Add‑ons: no fee

### Screenshots

Chrome requires ≥1 screenshot sized **1280×800** or **640×400** (Edge: 1280×800). Capture the
extension in use:

1. `chrome://extensions` → **Developer mode** → **Load unpacked** → pick `browser-extension/build/chrome`.
2. Open any site with a login form while the Lados app is running and the vault is unlocked.
3. Screenshot the **“Fill with Lados”** pill (and, for a second shot, the multi‑login picker).

### Hosting the privacy policy

Chrome and Edge need a public URL. Two options:

- **Quick:** the rendered GitHub page
  `https://github.com/adrijuse395/lados/blob/main/docs/PRIVACY.md`
- **Cleaner:** enable GitHub Pages and link `https://adrijuse395.github.io/lados/PRIVACY.md`
  (or a small HTML page).
