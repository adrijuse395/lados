# Lados browser extension

Fills login forms from your **local** Lados vault. No account, no cloud, no telemetry —
the extension only talks to the Lados desktop app over `http://127.0.0.1:18765`.

## How it works

```
login page ── content.js (Shadow-DOM widget)
                 │  1. runtime.sendMessage({type:"find", url})   → candidates (no secrets)
                 │  2. runtime.sendMessage({type:"reveal", id})  → one password, on click
                 ▼
            background.js ── fetch ──► http://127.0.0.1:18765/v1/entries?url=…
                 ▲                          │
                 └────── { entries } ◄──────┘   (only while the vault is unlocked)
```

- The **content script** never fetches the bridge directly (a page's CSP would block it);
  it asks the background script, which is privileged and exempt from page CSP.
- **No password is ever stored in the page.** `/v1/entries` returns only candidates
  (id, title, username, URL). The secret is fetched from `/v1/reveal?id=…` at the
  instant you click fill — so if Lados is locked or closed, nothing stale can be filled.
- The bridge sends **no CORS header**, so web pages cannot read its responses even if
  they guess the port. Only the extension (which holds `host_permissions`) can.
- Passwords are only returned while the vault is **unlocked**; locking Lados stops autofill.

## Files

| File | Role |
| ---- | ---- |
| `manifest.json` | Firefox (MV2). Minimal permissions: only `http://127.0.0.1:18765/*`. |
| `manifest.chrome.json` | Chrome/Edge (MV3, `service_worker`). Same code, Chromium-shaped manifest. |
| `background.js` | Privileged bridge client (status + entry lookup + on-demand reveal, with a timeout). |
| `content.js` | Detects login forms, draws the in-page widget (Shadow DOM), fills inputs. |
| `popup.html` / `popup.js` | Toolbar popup: connection state + "Open Lados". |
| `icons/` | 16 / 32 / 48 / 128 px icons. |

The per-store packaging script lives at `scripts/build-extension.py`.

## Build the store packages

```bash
python3 scripts/build-extension.py
```

Produces (under `browser-extension/build/`, git-ignored):

- `firefox/` and `lados-firefox.zip` — for addons.mozilla.org
- `chrome/` and `lados-chrome.zip` — for the Chrome Web Store / Edge

## Load it (development)

**Firefox:** `about:debugging` → **This Firefox** → **Load Temporary Add-on…** → pick `manifest.json`.

**Chrome / Edge:** `chrome://extensions` (or `edge://extensions`) → **Developer mode** → **Load unpacked** → pick the generated `browser-extension/build/chrome` folder. Reload the add-on after editing any file.

## Publishing

**Firefox Add-ons (AMO)** — free, **published** ✅
- Live listing: <https://addons.mozilla.org/firefox/addon/lados/> (v0.3.0, reviewed 2026‑11‑13)
- Extension ID: `lados@adrijuse395.github.io`
1. Bump `version`, run `python3 scripts/build-extension.py`.
2. Upload `build/lados-firefox.zip` at <https://addons.mozilla.org/en-US/developers/>.
3. Distribution: **On this site**.

> **0.4.0 requires the app build that asks for pairing**, and adds the `storage` permission so the
> extension can keep the pairing code. Publish it *before* (or together with) an app release that
> requires pairing: the published 0.3.x sends no code and would be refused by the new bridge.

**Chrome Web Store** — one‑time $5 developer fee:
1. Upload `browser-extension/build/lados-chrome.zip` at <https://chrome.google.com/webstore/devconsole>.
2. Fill the listing from [`STORE-LISTING.md`](./STORE-LISTING.md) (copy‑paste fields + checklist).
3. Privacy: **no data collected**.

**Edge Add-ons** — free, same `lados-chrome.zip`.

> **Blocked since Nov 2026 — Microsoft-side.** Enrollment can't be completed: `Account settings →
> Programs → Microsoft Edge → Get started` redirects to an empty Partner Center Home ("Go to Account
> Settings on your Dashboard menu to start setting up your account"), and the registration form
> never appears. It affects many developers and is tracked in
> <https://github.com/microsoft/MicrosoftEdge-Extensions/issues/905>. **Chrome is unaffected.** The
> package and the listing copy below are ready; reopen this once Microsoft fixes the provisioning.

1. Sign in with a **personal Microsoft account (MSA)** — an Outlook/Live/Hotmail address, or a
   personal GitHub account. Work/school accounts, and a plain Gmail address, are **not** accepted.
2. Upload the same `lados-chrome.zip` at <https://partner.microsoft.com/dashboard/microsoftedge>.
3. Reuse the listing copy from [`STORE-LISTING.md`](./STORE-LISTING.md). Edge wants a **300×300**
   logo and, optionally, a **440×280** tile (screenshots may be 640×480 or 1280×800).
4. The certification notes must say the extension does nothing without the Lados desktop app on
   `127.0.0.1:18765` — reviewers otherwise see no behaviour.

The store pages for Firefox, Chrome and Edge are wired in `src/dialogs/browser-extension.ts`;
set each URL there once the add-on is published.

## Known limitations

These are deliberate, and are the cases the fill does **not** cover (the red-team report, I3):

- **Open shadow roots and cross-origin iframes.** Fields inside a page's own shadow DOM, or in an
  iframe the extension does not run in, are not detected — so no widget appears. Many SSO and
  embedded-login forms are built this way.
- **IP addresses and host names are different sites.** Matching is by host, so `http://127.0.0.1/`
  and `http://localhost/` do not match each other: save the entry under the exact host you use.
- **Forms with several password fields.** On a change-password form only the first password field is
  filled; the "new" and "confirm" boxes are left for you (the extension never invents a password).
- **One match per fill.** The widget fills the username and password of the entry you choose; it is
  not a general form filler.

## Notes

- Firefox requires the fixed `browser_specific_settings.gecko.id`; Chromium needs `manifest_version: 3` with `background.service_worker`.
- The extension only talks to `http://127.0.0.1:18765` (loopback). It sets no CORS header, so web pages cannot read its responses.
