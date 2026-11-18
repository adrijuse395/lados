# Lados — pre-release security assessment (red team)

This is the plan for an **authorized, adversarial assessment of Lados before it ships** with sync
and paid billing. Two independent agents work it: one as a relentless user, one as an attacker.
Everything runs against a **staging** deployment, so nothing here can touch the data the app will
ship with.

> **Scope in one line:** our code, our **staging** server, our website, our designated **test
> accounts**. Nothing else.

The prompts handed to the agents are [`red-team/PROMPT-explorer.md`](../red-team/PROMPT-explorer.md)
and [`red-team/PROMPT-attacker.md`](../red-team/PROMPT-attacker.md).

---

## 1. Why we do this, and what an attacker actually gains

Lados is offline-first and zero-knowledge. That shapes where the real risk lives, and it is worth
stating so the agents do not waste effort on the wrong thing:

- **The vault file is not a remote target.** A `.lados` file is opened with Argon2id +
  XChaCha20-Poly1305 entirely on the device. An attacker who already has the file has the file; the
  assessment cannot "attack the vault" over the network.
- **The server never sees plaintext.** It stores opaque ciphertext blobs plus minimal metadata.
- **So the realistic attack surface is:** the **account** (OPAQUE auth, tokens, sessions), the
  **authorization** on vault rows and blobs (can A read B's data?), the **entitlement** logic
  (paywall / `402` / quota / comp), the **billing webhook**, **enumeration** and **rate limits**,
  and the **human paths** (the website, the extension, the desktop binaries and their distribution).

Attackers also go *around* the code — email accounts, the MoR, the store accounts, DNS. Those are
**out of scope here** and are covered instead by the operational checklist in `GO-LIVE.md`
(2FA, secret rotation, least privilege).

## 2. The two assessments

| # | Assessment | The question it answers | Prompt |
| - | ---------- | ----------------------- | ------ |
| 1 | **Functional explorer** | Does every feature hold up under normal-but-relentless use? Bugs, edge cases, robustness, data-loss risks. | `red-team/PROMPT-explorer.md` |
| 2 | **Adversarial (red team)** | Can an attacker read data it should not, bypass payment, or take over an account? | `red-team/PROMPT-attacker.md` |

They are **separate sessions** on purpose: the explorer must behave like a user, the attacker must
not.

## 3. Scope

### In scope

- The **Lados desktop app**, as the **staging build** (Windows/macOS/Linux).
- The **browser extension** (the Chromium/MV3 build).
- The **staging sync Worker** and every endpoint it exposes.
- The **public website** (the `lados-web` project).
- The **designated test accounts** — and only those.

### Out of scope — hard boundaries

- **Cloudflare's own infrastructure** (only our Worker on it).
- **Email providers** (Gmail, Outlook) and any third-party account.
- **Paddle production**, and any payment system handling real money.
- **Any other user's data.** If the tester can see something that looks like another person's data,
  **stop and report it** — do not open, copy or download it.
- **Denial of service, flooding, brute-force at scale, spam.** (A handful of requests to prove a
  rate limit is missing is fine and expected; hammering is not.)
- **Social engineering of real people.**
- **Publishing any finding** before it is fixed.

### Authorized by

The owner of the project authorizes testing of the assets above, within the stated window, for the
purpose of improving the product. This is the owner's own system; the boundaries above are what keep
it a legitimate test rather than an attack.

## 4. Environment

### 4.1 The staging server (separate from the app's server)

Staging is a **second Cloudflare Worker** with its **own D1 database and R2 bucket**, so no test can
reach the data the shipped app uses.

- Worker name: `lados-sync-staging`
- URL: `https://lados-sync-staging.<subdomain>.workers.dev` — recorded out of band.
- Paddle: **sandbox** keys only. No real money exists on this environment.

Created from `sync-server/wrangler.toml` (`[env.staging]`); the setup and deploy commands are in the
comment block there. **Generate a new `OPAQUE_SERVER_SETUP` for staging** — never reuse production's.

> The app's own server (`lados-sync`) is **not** in scope and must not be probed. The agents are told
> its URL is off-limits; watch its logs during the window anyway (see §6).

### 4.2 The client build that points at staging

The sync server URL is baked in at compile time; a normal build always targets production. The
staging build sets two values:

```powershell
$env:LADOS_SYNC_SERVER = "https://lados-sync-staging.<subdomain>.workers.dev"
$env:LADOS_CHANNEL    = "staging"
npx tauri build --bundles nsis --features sync      # or the platform's bundle
```

- `LADOS_SYNC_SERVER` points the app at staging.
- `LADOS_CHANNEL=staging` marks the build: the UI shows it, and the **in-app updater is disabled**,
  so a test build can never quietly replace itself with a released one.

**Deliver it** as a **GitHub pre-release** (clearly marked *pre-release*), or a short-lived private
link. It is **not** a release and must never look like one.

### 4.3 Test accounts and data

Create the accounts **through the staging build** (OPAQUE means they cannot be inserted by hand):

1. Register `explorer@<test-domain>` and `attacker@<test-domain>` on staging.
2. In each, create a vault with a handful of entries, folders and at least one custom protected
   field. Unlock it so autofill can be tested.
3. Optionally seed one `comp` account to exercise the free-access path.

Credentials are shared **out of band** (never committed). Anything the agent needs day to day goes in
`red-team/local.md`, which is git-ignored.

### 4.4 The extension

The Chromium build is `browser-extension/build/lados-chrome.zip` (rebuild with
`python scripts/build-extension.py`). Load it unpacked in the test browser. It talks only to
`127.0.0.1:18765`, so it needs the desktop app running with the vault unlocked.

## 5. Rules of engagement (given to both agents)

1. **Only the assets in §3.** If a path leads outside it, stop.
2. **Do not destroy data.** The test account is disposable; everything else is not.
3. **Report, do not exploit.** Finding a way in is the deliverable; using it against anyone is not.
4. **Stop and report** if you find evidence of someone else's data, or if a technique could not be
   stopped once started (a flood, a mail loop, anything with a real cost).
5. **No third-party systems.** No Cloudflare, no mail providers, no Paddle production, no DNS.
6. **Be honest about what you could not test** (for example, if you could not run the desktop app).

## 6. Observability — watch the experiment

- Live staging requests: `cd sync-server && npx wrangler tail --env staging`
- Also tail **production** for the duration of the window: the repository is public, so the
  production URL is discoverable, and the point is to *see* whether anything strays there.
- Keep the window short (48–72 h) and keep the stop switch handy:
  `npx wrangler delete --env staging` ends it immediately.

## 7. Reporting format

Every finding, from either agent, in this shape:

```
Title:
Severity: informational | low | medium | high | critical
Component: app | extension | API | website
Environment: staging build <version>, staging Worker, <browser/OS>
Preconditions:
Steps to reproduce:      (numbered, exact)
Observed:
Expected:
Evidence:               (screenshot, request/response, log line)
Reproducible:           always | intermittent | once
Impact:                 what an attacker or a user actually gains
Suggested fix (optional):
```

Plus a short **summary**: what was tested, what was not, and the top three things worth fixing first.

**No finding without a reproduction** — a plausible hunch is not a finding. Say "could not verify"
instead.

## 8. Triage → fix → re-test

1. Triage every finding against the **actual** threat model in §1 (the agents will report some
   non-issues; that is normal and expected).
2. Fix on `main`. Add a regression test where the fix is testable.
3. **Re-test the fix on staging**, and note it in the finding.
4. Only then is the finding closed. Nothing changes in production until this loop is done.

## 9. Teardown

- Rotate **every** staging secret (`OPAQUE_SERVER_SETUP`, `PADDLE_API_KEY`,
  `PADDLE_WEBHOOK_SECRET`).
- Delete the staging Worker, D1 database and R2 bucket if they are not wanted for the next round.
- Confirm production was untouched (`funnel.mjs` numbers unchanged).
- Keep the findings; delete the test accounts.

## 10. Known limitations — do not spend time on these

Stated so the agents do not rediscover by design:

- **No password reset.** There is none, on purpose; recovery is via the recovery key.
- **`/admin/comp` returns 404** unless `ADMIN_TOKEN` is set. On staging it is not set.
- **The server never sees the master password** (OPAQUE) and never sees vault plaintext.
- **Being rate-limited (`429`) is expected.** It is only a finding if the limit is trivially
  bypassable, or absent on an endpoint that needs one.
- **The extension is read-only autofill**; it does not save passwords.
- **A compromised device is out of scope** — malware on the machine can read what the user can.
