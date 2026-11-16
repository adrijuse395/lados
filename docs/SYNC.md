# Lados Sync — design

Status: **draft (design only — no code yet)**

Lados Sync is an **optional, paid, zero-knowledge sync service** for multi-device users.
Lados itself stays **free, offline-first, and fully functional without an account**.

This document is the contract we build against. Nothing here changes the current app
until it is deliberately implemented behind a feature flag.

## 1. Goals and non-goals

**Goals**
- Keep one vault in sync across a user's own devices (PC, laptop, phone).
- The server can **never** read vault contents (zero-knowledge).
- Reuse what Lados already has: the `.lados` file format, its cryptography, and `merge.rs`.
- Be cheap to run and trivial to operate (serverless).
- Be understandable and self-hostable by anyone.

**Non-goals (v1)**
- Sharing between different users; teams; roles.
- Web access to the vault.
- Real-time push / live collaboration.
- Fancy conflict UIs (merging stays automatic).
- Cross-region replication, SLAs, enterprise features.

## 2. Guiding principles

1. **Offline-first.** The app works exactly as today with no account and no network.
2. **The server is a dumb box.** It stores opaque encrypted blobs and never decrypts.
3. **Merge on the device, before upload.** The server never resolves conflicts.
4. **Pay for the service, not for artificial limits.** One account = unlimited devices
   and vaults.
5. **Open and self-hostable.** The server is open source; paying is for convenience.

## 3. Threat model

Protect against:
- A **compromised or malicious server** (must see only ciphertext).
- **Network attackers** (TLS is the floor).
- **Lost/stolen device** (the vault is encrypted at rest; an unlocked session is the
  device owner's problem, as today).

Explicitly out of scope for v1:
- A compromised client device at the moment the vault is unlocked.
- Traffic analysis of *when* a user syncs.
- Denial of service.

Accepted metadata leakage (server sees): account id, vault id, version, byte size,
timestamps. Mitigated by padding (already 64 KiB blocks) and opaque identifiers.

## 4. Key derivation

From the user's master password (and optional key file — see §9) the client derives two
**independent** secrets with HKDF and distinct labels:

| Value | Where it lives | Purpose |
| --- | --- | --- |
| `encKey` | device only, never sent | Encrypts/decrypts the vault (as today) |
| `authSecret` | exchanged during login only | Authenticates the account |

- The server stores a **verifier**: `SHA-256(salt ‖ authSecret)` with a per-account
  salt. `authSecret` is `Argon2id(compositeKey, salt = f(email))` — **per-account salted**, so
  two users with the same password derive different secrets and a leaked table cannot be
  attacked with one Argon2 run per guess across all accounts. The server never stores
  `authSecret`.
- **Migration:** older accounts are upgraded in place on their next successful login. The
  client also sends the previous derivations (a single-salt Argon2, and the original fast
  hash) so any pre-existing account can authenticate once and be re-keyed.
- **v1 login:** **OPAQUE** (RFC 9807). The client and server run a PAKE, so the server never
  sees the password or anything replayable. The client’s OPAQUE input is the Argon2 `authSecret`,
  which keeps offline guessing Argon2-priced. The server’s OPAQUE module is the same `opaque-ke`
  crate compiled to WebAssembly (`sync-server/opaque-wasm`), so the two sides interoperate by
  construction. Accounts created before OPAQUE still authenticate with the schemes above and are
  **converted on their next sign-in** (`/auth/upgrade/*`).
- The encryption key derivation stays exactly as it is today (Argon2id + per-file salt
  in the `.lados` header). Sync does not change the file format.

## 5. Server data model

Minimal. The server is a key/value store with a version counter.

```
Account   { id, email, authVerifier, createdAt, plan, planValidUntil }
Vault     { id, accountId, name(ciphertext opt.), version, size, updatedAt, blobRef }
Blob      { vaultId, version -> encrypted .lados bytes }   // object storage (R2)
```

- `blobRef` points to the encrypted `.lados` bytes in object storage.
- `version` is a monotonically increasing integer per vault, used for optimistic
  concurrency.
- Nothing here is secret except the ciphertext.

## 6. Sync protocol (v1)

All endpoints are HTTPS, JSON, authenticated with a bearer token (JWT).

```
POST /auth/register   { email, authVerifier }            -> { token }
POST /auth/login      { email, authSecret }              -> { token }   // server verifies
GET  /vaults                                             -> [ { id, name, version } ]
POST /vaults          { name }                           -> { id }
GET  /vaults/{id}                                        -> { version, blob }
PUT  /vaults/{id}     If-Match: {version}  body=blob     -> { version: n+1 }
DELETE /vaults/{id}
```

**Optimistic concurrency:** `PUT` succeeds only if the client's `If-Match` version
equals the server's current version. Otherwise the server replies **409 Conflict** and
returns the current version. The client then downloads, merges, and retries (§7).

## 7. Conflict resolution (the important part)

There is no merge logic on the server. Sync is a client loop:

```
sync(vault):
    remote = GET /vaults/{id}
    if remote.version == local.lastSyncedVersion:
        if local changed: upload()          # our only change
        return
    decryptedRemote = decrypt(remote.blob, encKey)
    merged = merge::merge(local.vault, decryptedRemote)   # reuses src-tauri/src/merge.rs
    write merged to local vault
    upload(merged)

upload(vaultBytes):
    try PUT /vaults/{id} If-Match: version
    on 409: goto sync(vault)   # somebody else wrote; merge and retry
```

Because both `merge.rs` rules already exist (newest change wins per entry, deletions
remembered via tombstones, move-vs-edit tracked separately), **both devices' edits
survive**. Nothing is lost; worst case is a retry.

**Sync triggers (v1):** a **“Sync now”** button, and automatically **on unlock** and
**on lock/close**. No background daemon, no push.

## 8. Client module (Rust)

New, self-contained module — no changes to existing crypto/storage/session code.

```
src-tauri/src/sync/
  mod.rs        orchestration (sync loop above)
  remote.rs     trait Remote { list, get, put, create, delete }
  http.rs       the HTTP Remote (talks to Lados Sync)
  account.rs    login/register, token handling, key derivation (encKey/authSecret)
  model.rs      request/response types
```

- `Remote` is a trait so alternative backends (BYO storage: WebDAV/S3) can be added later
  without touching the sync loop.
- Authorization tokens are stored in the OS keychain when available; never in the vault.

## 9. Key files

A synced vault must be openable on every device. The simplest, safest v1 rule:

> **Synced vaults use a master password only (no key file).**

Rationale: a key file lives on disk and would have to be copied to every device; if it is
missing, sync silently fails to open. v1 disables the key-file option for vaults that
enable sync, and says so in the UI. (A later version may allow key files with an explicit
"copy this file to your other devices" flow.)

## 10. Recovery

- **No server-side password reset** — impossible by design (the server can't read data).
- Optional **recovery key**: a high-entropy value shown once that can unlock the vault
  (implemented locally in Lados, like a key file). Stored by the user offline.

## 11. Pricing and billing (proposed)

> Full design — seller/MoR, entitlement states, webhook contract, identity and legal steps —
> lives in [`BILLING.md`](./BILLING.md). The notes below are the short version.

- **One annual plan, ~€12/year** (proposed). Avoid monthly charges: fixed per-transaction
  fees swallow small amounts.
- **Merchant of Record:** Paddle (or Lemon Squeezy). They are the legal seller, handle
  global VAT/sales tax, and pay out monthly. No tax paperwork on our side.
- **No artificial limits:** unlimited devices and vaults.
- **Trial:** short free trial to evaluate, then subscribe to keep syncing. Lados without
  an account remains free forever.
- Optional early **“founder” lifetime** tier to bootstrap — only if we accept that it
  funds ongoing costs.
- **Comp accounts:** the maintainer (and later, contributors/testers) get a **comp**
  plan — a single flag, treated as active and never expiring, exempt from payment.
  Deliberately **not** a roles/permissions system: one user doesn't justify that
  complexity. The client just sees an active plan.

## 12. Hosting and technology

**Decision: Cloudflare Workers + R2 + D1.**

| Piece | Role | Notes |
| --- | --- | --- |
| **Workers** | API: auth, vault metadata, entitlement, billing webhook | Serverless, global, zero ops. Free tier: 100k requests/day |
| **R2** | Encrypted blobs (the `.lados` files) | **No egress fees.** Free tier: 10 GB storage |
| **D1** | Accounts, plans, vault versions | SQLite. Free tier: 5 GB, 5M reads/day |

- The API is written in **TypeScript**: it performs **no cryptography** (it stores
  ciphertext and hashes the auth verifier), so it stays small and simple.
  (Rust via `workers-rs` is possible but adds build complexity for no benefit here.)
- **No servers to operate**: no OS patching, no load balancers, no disks.

### Why not the alternatives

- **Own Linux VPS** — single point of failure, manual ops and scaling. Still allowed as
  *self-hosting* for users who accept the trade-off, but not our hosted plan.
- **Supabase / Neon (Postgres)** — nice, but **egress is metered** and blobs-in-DB is a
  poor fit for repeated downloads.
- **Fly.io / Render** — better than a VPS, still "a server" to manage.
- **AWS S3 + Lambda + DynamoDB** — S3 charges **egress**; more IAM complexity; no
  permanent free tier.
- **Vercel / Netlify functions** — fine for the API, but blob storage is paid and
  egress-metered.

### Cost model

- **Personal / early use: €0.** The free tiers (100k req/day, 10 GB R2, 5 GB D1) cover a
  single user and early adopters with large headroom (one vault ≈ 64–256 KB).
- **Growth is funded by subscriptions.** At thousands of users the bill is roughly the
  Workers paid plan (~$5/mo) plus a few GB of R2, so one **€12/year** subscription covers
  infrastructure for many users.
- **Scales automatically** — no capacity planning.

### Concurrency

Version increments use an atomic compare-and-set
(`UPDATE ... WHERE id = ? AND version = ?`) whose affected-row count distinguishes success
from a 409 conflict — no cross-request locks required.

### Consistent with the design

The server is a **dumb box**: it authenticates accounts and stores opaque blobs. All
cryptography and all conflict resolution happen on the user's devices (§7).

## 13. Self-hosting

- The server repository is standalone and documented; anyone can run it (Cloudflare, or
  a small VPS/Docker image) and point the app at their own URL.
- This is the trust guarantee: the paid plan buys convenience, not permission.

## 14. Licensing

- **App (`lados`): GPL-3.0-only** — unchanged.
- **Server (`lados-sync`): AGPL-3.0** — so it can be self-hosted but not operated as a
  closed competing service. Separate repository to keep licenses unambiguous.

## 15. Roadmap

| Phase | Deliverable | Touches the shipped app? |
| --- | --- | --- |
| **0 (this doc)** | Design | No |
| **1** | Client sync engine behind a `sync` Cargo feature, with a fake in-memory `Remote` + unit tests | No (feature off) |
| **2** | Minimal Lados Sync server (auth + blobs) + docs | No (separate repo) |
| **3** | Account UI behind the feature flag; manual + unlock/close sync | Only when feature enabled |
| **4** | Billing (Paddle), self-host docs, recovery key | Separate |
| **5** | Enable the feature in releases; announce | Yes (intentional) |

## 16. How we avoid breaking the current release

- Sync code is compiled only with the **`sync` Cargo feature**, which stays **off** for
  normal builds until Phase 5. The shipping binaries are unchanged.
- No existing module (crypto, storage, session, bridge, extension routes) is modified for
  sync; the engine composes them.
- The server lives in a **separate repository**; nothing in the app depends on it to build
  or run.
- While the browser-extension work is in progress, sync stays entirely dormant.

## 17. Open questions

- Exact price and trial length.
- Provider: Paddle vs Lemon Squeezy.
- Do we want a small **free sync tier** (e.g. one vault / two devices) or none?
- OPAQUE auth in v1 or v1.1?
- Account password = derived from the master password (one secret) vs a separate login
  password (two secrets). Current proposal: derived from the master (one secret).

## 18. Entitlement: how the paid module is enabled

**Principle: the code is not the product; the hosted service is.** Because Lados is open
source, we do not hide or DRM the sync code. The app ships with the sync module present;
what makes it "paid" is that the feature is **useless without the server**, and **the
server decides** whether an account may sync.

There is nothing to "crack": a patched client that enables sync still has no data source
without a server, and our server checks the account's plan and refuses if it is expired.
A user who wants it for free can **self-host the server** (by design).

Two distinct switches (people conflate them):

1. **Build-time feature (`sync`)** — keeps the shipping binary unchanged *while we build
   it*. When the feature ships, it is enabled for **everyone**; one binary, no "pro
   build". This is intentional and open-source friendly.
2. **Runtime entitlement** — decided **by the server**, per account, on every request.
   This is the real gate.

### Flow

```
Client                                        Server
------                                        ------
"Enable Sync"
  -> register/login (email + authSecret)        verify stored verifier
                                              <- JWT { accountId, plan, planValidUntil }
   token in OS keychain

each sync call carries the token:
  GET/PUT /vaults/...   -------------------->   check token + plan
                                                expired  -> 402 Payment Required
                                                active   -> proceed

Pay (Paddle Checkout)  ->  Paddle  ->  webhook  ->  set plan / planValidUntil
next sync             ---------------------->   allowed again
```

- The **client never decides** entitlement. It may cache the last known plan to render the
  UI, but the server is authoritative on every request.
- **Payment is tied to the account** (email + authSecret), not to a device. One account =
  unlimited devices and vaults.

### UI states

| State | Shown | Sync allowed |
| --- | --- | --- |
| No account | “Sync across your devices (subscription) — Set up” | No (app fully works offline) |
| Account, active | “Last synced …”, *Sync now*, device list | Yes |
| Account, expired | “Renew to keep syncing”, billing link | No (server returns 402) |
| Offline / server down | No error spam; simply does not sync | N/A |

The local application is **never** disabled — only the cloud part stops when unpaid.

### Deliberately avoided

- Local "license keys" with expiry checks (breakable, annoying, against the philosophy).
- Hiding or obfuscating the sync source.
- Per-device limits, fingerprinting, or artificial vault caps.
- Blocking the free offline app for any reason.

## 19. Vault model and binding

- An account can own **N cloud vaults**, and each local `.lados` binds to a cloud vault through
  the vault's own **UUID** (not its name). A copy of a file matches; a *different* vault is a
  different cloud vault and is never merged by accident.
- The **standard vaults folder** is `Documents/Lados` (changeable in Settings). New vaults are
  created there and downloads land there, named after the vault; the user chooses only the
  folder.
- The **start screen** lists each vault with **In the cloud** and **In this device**, so a new
  device sees at once which vaults still have to be downloaded.
- **New device:** sign in, pick a cloud vault, and the app downloads it, derives the key from
  the blob's own header, binds it and syncs.

### Password model

- A sync user has **one master password**: it is the **account password** and the password of
  **every vault they sync**.
- It must be **very strong** — enforced for the account and for every vault master password
  (creating a vault, and *Change master key*). Individual **entry** passwords are never subject
  to this rule.
- A vault whose master password differs from the account's **cannot be synced silently**: the
  client verifies by attempting an OPAQUE login with that vault's credentials and refuses if it
  fails, telling the user to set the vault to the master password first. This prevents a cloud
  copy that could never be downloaded.
- *Re-key-on-add*: when a vault's master password is not the account's, the app offers to
  re-encrypt it with the master password (after confirming that password) instead of just
  refusing.
- *Coordinated change*: changing the master password once re-encrypts the account credential and
  **every synced vault** under it — including cloud vaults that are not on this device, which are
  re-encrypted in memory from their ciphertext. It is a single blocking operation with a progress
  screen; the user is told their other devices must sign in with the new password afterwards. The
  server still only ever receives ciphertext: each vault is decrypted on the device, re-encrypted
  with the new password and re-uploaded, and labels are re-sealed through a dedicated
  `PATCH /vaults/:id` endpoint that never sees a plaintext name.

## 20. Threat model

**Design:** zero-knowledge. The server stores only ciphertext; every key is derived on the
device and never transmitted.

### A remote attacker (only API access)
- **Cannot read any vault.** Every query is scoped by the authenticated account; there is no
  bulk endpoint; blobs are opaque ciphertext.
- Online credential guessing is **throttled** (fixed-window rate limits on `/auth/*` and on
  uploads), and a **per-account storage quota** (100 MB) bounds abuse.
- **Email enumeration** is possible (register returns `409` for a taken address) — known
  limitation, to be closed with email verification.

### A breach of D1/R2, a stolen token, or a malicious operator
- Yields **metadata** (account email, vault sizes, timestamps) and **ciphertext** — never
  plaintext. Vault **names are stored only as ciphertext**, encrypted with an account label key
  that stays on the device, so they cannot be read from the server. Labels uploaded in clear by
  older versions are re-sealed (and the plaintext column cleared) on the next password change or
  through the one-time *Encrypt names* action.
- **Passwords are at risk only if a blob leaks *and* the master password is weak.** That is
  inherent to zero-knowledge; it is mitigated by a strong KDF, enforced strong account
  passwords, and minimising the chance of any leak.
- A stolen bearer token allows **sabotage** (overwrite or delete the cloud copy), not reading.
- The account credential is **per-account salted Argon2**, so a leaked verifier table cannot be
  attacked with a single Argon2 run per guess across all accounts.
- **Rollback protection:** each device remembers the last cloud version it saw and refuses a
  cloud copy older than that, so a server cannot silently revert a vault.

### Operator trust
- The **Cloudflare account** and the **release-signing key** are the root of trust; they need
  2FA, scoped tokens and alerts. A malicious server can serve data and even logins, but it still
  cannot decrypt vaults.
- **Planned:** OPAQUE login, so the server never sees a password-equivalent.

### Known limitations / next steps
1. **Remaining metadata** — the account email, vault sizes and timestamps are still visible.
2. **Email verification** — removes enumeration and enables billing.
3. **Password rotation** — changing the account password must re-key the cloud vaults; not
   built yet. (Sessions can already be revoked: “Sign out other devices”.)

