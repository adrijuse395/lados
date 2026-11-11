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

**Firefox Add-ons (AMO)** — free:
1. Bump `version`, run `python3 scripts/build-extension.py`.
2. Upload `build/lados-firefox.zip` at <https://addons.mozilla.org/en-US/developers/>.
3. Distribution: **On this site**.

**Chrome Web Store** — one-time $5 developer fee:
1. Upload `build/lados-chrome.zip` at <https://chrome.google.com/webstore/devconsole>.
2. Fill the listing (Privacy: no data collected).

**Edge Add-ons** — free, same `lados-chrome.zip`: <https://partner.microsoft.com/dashboard/microsoftedge>

After approval, set the resulting URLs in `src/dialogs/browser-extension.ts`.

## Notes

- Firefox requires the fixed `browser_specific_settings.gecko.id`; Chromium needs `manifest_version: 3` with `background.service_worker`.
- The extension only talks to `http://127.0.0.1:18765` (loopback). It sets no CORS header, so web pages cannot read its responses.
