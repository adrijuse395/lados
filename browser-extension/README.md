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
| `manifest.json` | MV2 (Firefox). Minimal permissions: only `http://127.0.0.1:18765/*`. |
| `background.js` | Privileged bridge client (status + entry lookup, with a timeout). |
| `content.js` | Detects login forms, draws the in-page widget (Shadow DOM), fills inputs. |
| `popup.html` / `popup.js` | Toolbar popup: connection state + "Open Lados". |
| `icons/` | 16 / 48 / 128 px icons. |

## Load it (development)

1. Firefox → `about:debugging` → **This Firefox** → **Load Temporary Add-on…**
2. Select `browser-extension/manifest.json`.
3. Run Lados and unlock your vault.

Reload the add-on after editing any file (Firefox does not hot-reload).

## Publishing (Firefox Add-ons — free)

1. Bump `version` in `manifest.json`.
2. Zip the **contents** of this folder (not the folder itself):
   `cd browser-extension && zip -r ../lados-firefox.zip .`
3. Upload at <https://addons.mozilla.org/en-US/developers/>.
4. After approval, set the resulting URL in
   `src/dialogs/browser-extension.ts` (`AMO_URL`).

## Notes

- Firefox requires the fixed `browser_specific_settings.gecko.id` in the manifest.
- Chromium will need an MV3 build (separate manifest with `background.service_worker`).
