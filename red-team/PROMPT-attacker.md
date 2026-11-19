# Task — Adversarial red team (Lados, pre-release)

You are a penetration tester. You have been **authorized by the owners** to attack their own product:
the Lados password manager and its **staging** sync server. Your objective is to **find** ways to
read data you should not, bypass the paywall, or take over an account — and to **report** them.

This is a test of the owners' own system. Everything below the line in §3 is off-limits, and crossing
it turns a legitimate test into an attack.

---

## 1. What Lados is, and where the risk actually is

An offline-first, **zero-knowledge** password manager.

- Each vault is an encrypted `.lados` file on the user's device (Argon2id + XChaCha20-Poly1305).
  **The vault file is not a remote target** — an attacker who has the file has the file. Do not spend
  time trying to "decrypt the vault over the network"; that is not how it works.
- The optional sync server stores an **opaque, encrypted** copy of each vault plus minimal metadata.
  It never sees the master password or vault plaintext.
- Accounts use **OPAQUE** (aPAKE): the client proves knowledge of the master password without
  sending it; the server stores only an OPAQUE registration record. Auth is a bearer token (stored
  hashed, 30-day TTL).

So your realistic attack surface is: **authorization** on vault rows/blobs, the **entitlement**
logic (paywall, `402`, quota, comp), **session/token** handling, **enumeration**, **rate limits**,
**input validation**, the **billing webhook**, and the **client/extension** paths.

## 2. Your environment

- **Target server:** `<STAGING_URL>` — a **staging** Worker with its own database and **sandbox**
  Paddle. No real money exists here.
- **Test accounts:** `<TEST_ACCOUNT_EMAIL>` / `<TEST_ACCOUNT_PASSWORD>`, and a second account
  `<TEST_ACCOUNT_2_EMAIL>` / `<TEST_ACCOUNT_2_PASSWORD>` for cross-account tests. Create more if
  useful.
- **Client (optional):** most of your work is HTTP against `<STAGING_URL>` — that needs no app. If you
  also want to drive the desktop app and extension, see *Obtaining the client* below.
- **Source:** the repository is public — **read it**. That is part of being a realistic attacker.
- **Getting a bearer token (for direct API work):** either sign in through the app and read the token
  from its local settings file, **or** script the OPAQUE handshake with `@serenity-kit/opaque` — the
  public repo's `sync-server/scripts/opaque-e2e.mjs` is a working example that takes a `BASE_URL`.

### Obtaining the client

You do **not** need the app to attack the API: `curl` / scripts / a proxy against `<STAGING_URL>` is
the main path. If you also want the app and the extension, note there is **no "sandbox" switch** — the
server is compiled into the build, so use one made for staging:

1. **Pre-release build (preferred):** download with `curl -L -O` from `<PRE_RELEASE_BASE>`.
   - **Linux x86_64:** `<PRE_RELEASE_BASE>/Lados_0.2.1_amd64.deb` (Debian/Ubuntu) or
     `<PRE_RELEASE_BASE>/Lados_0.2.1_amd64.AppImage`. The app is a **GUI**, so on a headless machine
     run it under `xvfb-run -a …`; if FUSE is missing, use `--appimage-extract-and-run`.
   - **Windows x64:** `<PRE_RELEASE_BASE>/Lados_0.2.1_x64-setup.exe` — unsigned, so SmartScreen warns.
   - All assets: `<PRE_RELEASE_URL>`. No asset for your platform? Use option 2.
2. **Build from source** (the repository is public):

   ```bash
   git clone https://github.com/ladospwm/lados.git && cd lados
   npm install
   LADOS_SYNC_SERVER=<STAGING_URL> LADOS_CHANNEL=staging npx tauri build --features sync
   ```

   **Do not build without both values** — a default build points at the *production* server.

**Verify the target before using the app:** Settings must show **`staging`** and the staging URL; if
not, **stop**. For raw HTTP you are already aimed at `<STAGING_URL>`.

### The API you may probe (staging only)

| Method | Path | Notes |
| ------ | ---- | ----- |
| POST | `/auth/register/start` · `/finish` | OPAQUE registration |
| POST | `/auth/login/start` · `/finish` | OPAQUE login |
| POST | `/auth/upgrade/start` · `/finish` | re-register the credential (needs a token) |
| POST | `/auth/login` | legacy path, kept only to upgrade old accounts |
| GET | `/me` | account + plan |
| GET | `/vaults` | list your vaults |
| POST | `/vaults` | create (name is stored encrypted) |
| GET | `/vaults/:id` | `{ version, blob }` |
| PUT | `/vaults/:id` | raw bytes + `If-Match: <version>` → `409` on conflict |
| DELETE | `/vaults/:id` · `/sessions` · `/account` | |
| POST | `/billing/checkout` · `/cancel` · `/resume` | sandbox |
| POST | `/billing/webhook` | signed by Paddle |
| GET | `/health` | |

## 3. First, verify the previous round's fixes

These landed after the first report. **Confirm or refute each**, then move on to the attack. A
regression here is a finding.

| Was | Check |
| --- | --- |
| **C1** — a web page could read entries | From a **sandboxed iframe** (`Origin: null`) and from a plain page, request `/v1/entries`. Both must be refused (`403`), and the page must see nothing. |
| **H1** — any local program could read passwords | `curl` to `/v1/entries` with **no pairing header** → `403`. With the code from Settings ▸ Browser pairing → answers, but `/v1/reveal` must **wait for an Allow click in the app**. Try to obtain a password without that click. |
| **M1** — raw OPAQUE error on sign-in | Wrong password → “Incorrect email or password…”, **not** `System error: OPAQUE protocol error`. |
| **M2** — leftover key file | Sign in with a **vault** key file selected in the Key file field of an account that has none → the message should point at the account's key file. |
| **M9** — truncated file read as a wrong password | Truncate a `.lados` file and open it with the right password → “damaged or incomplete”, **not** “wrong password”. |
| **M12 / M13** — corrupt CSV imports | A Latin-1 (`Cañón`) CSV → refused. An unclosed quote → reported, not imported as one entry. |
| **L5** — `.txt` forced, temp folder | Create a recovery key named `x.key` → it must **not** become `x.key.txt`; the dialog must not open in an AppImage temp path. |
| **L6** — check-dialog wording | “Check a recovery key” must say *Uses a different key*, not “an older file”. |
| **L18** — stale “Sync failed” chip | Delete the account → the chip must clear. |
| **H2** — silent re-encryption of many vaults | Creating **and** using a key must **list every file it will change before writing**, and an existing `.bak` must survive a second run. |
| **H4** — unsaved entry lost on auto-lock | Type an entry, let auto-lock fire, unlock → you are asked to carry on, and the text is there. |
| **M8** — deleted file recreated silently | Delete the open `.lados` file, edit → a notice names the file that was saved again. |
| **H3** — valid KeePass file rejected | Import a `.kdbx` whose `<String>` comes **after** `<AutoType>` (the previous fixture) → it must import. |

## 4. Rules (do not cross)

1. **Staging only.** Never the production server (`lados-sync.*`), Cloudflare itself, any mail
   provider, or Paddle production.
2. **No denial of service / flooding / brute-force at scale.** A few requests to demonstrate a
   missing limit is fine; sustained hammering is not.
3. **Do not destroy data** beyond your own test account.
4. **Report, do not exploit.** Finding and documenting a way in is the goal; using it against anyone
   is not.
5. **Stop immediately and report** if you reach data that looks like another person's, or if a
   technique cannot be stopped once started.
6. **No finding without a reproduction.** Say "could not verify" rather than speculate.

## 5. Objectives — try to actually achieve these

Work through them; a negative result is also a result. For each, keep the exact requests.

**Account takeover**
- Can you obtain or forge a session token? Is the token ever leaked (logs, error responses, URLs)?
- Is the **legacy `/auth/login`** path weaker than OPAQUE? Can it be used to downgrade an account,
  or does `/auth/upgrade/*` let a stolen token rotate the credential (as designed) — and what else
  does that enable?
- Session lifetime, revocation (`/sessions`), and what happens to sessions after a password change
  or account deletion.

**Reading data you should not (authorization / IDOR)**
- With account A's token, can you read B's vault (`GET /vaults/:id`, `PUT`, `DELETE`) by guessing or
  obtaining an id? Are ids unguessable and always scoped to the owner?
- Can you list or fetch blobs outside your account? Try manipulated ids, path tricks, encoding
  variations.
- Are vault ids (the UUIDs) leaked anywhere they should not be?

**Payment and entitlement**
- Can you reach synced data or upload **without** a paid/trialing plan (bypass the `402` /
  entitlement gate)?
- Can you make the server believe you are `comp`/`active`?
- Can you exceed the quota, or inflate it?
- Can you **forge the billing webhook**? The server verifies a Paddle signature — try to defeat it
  (wrong signature, replay, body tampering, timing). On sandbox, real money is not at risk.
- `checkout`/`cancel`/`resume`: any state you can put the subscription in that a user should not
  reach (double subscription, resume after expiry, cancel someone else's)?

**Enumeration and abuse**
- Does registration or login reveal whether an email exists?
- Can you register the same email twice, or create unbounded accounts?
- Are the rate limits (per IP, per email, per account) actually enforced? Where are they absent?
- Any input that causes an error with a stack trace, SQL error, or internal detail.

**Input validation**
- Unusual `email` values, huge bodies, wrong content types, missing/invalid `If-Match`, malformed
  OPAQUE messages, oversized blobs, `NaN`/negative/inconsistent numeric fields.

**Client and extension**
- The extension's bridge on `127.0.0.1:18765`: can a web page reach it? Does it set CORS? Can a
  page trigger a fill it should not, or read a password without a user action?
- The desktop app's local behaviour around files and the clipboard (no malware scenarios — assume a
  normal machine).
- The website: checkout flow, exposed tokens, client-side trust.

**Dependency and supply chain (light touch)**
- Anything obviously outdated or misconfigured in the public source. Report, do not fix.

## 6. Report format

For each finding:

```
Title:
Severity: informational | low | medium | high | critical
Component: app | extension | API | website
Environment: staging build <version>, staging Worker, <browser/OS>
CWE (if applicable):
Preconditions:
Steps to reproduce:      (numbered, exact requests/commands)
Observed:
Expected:
Evidence:               (raw request/response, screenshot, log line)
Reproducible:           always | intermittent | once
Impact:                 exactly what an attacker gains, and how bad it is
Suggested fix (optional):
```

End with a **summary**: what you attempted, what you achieved, what you could not test and why, and
the **top three** risks.

Rank findings by **real impact**, not cleverness. A theoretical issue you could not reproduce is
"informational" at best.

## 7. Deliverable

One Markdown report, most severe first. Include the raw evidence for anything rated medium or above.
