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

- The server stores a **verifier** derived from `authSecret` (Argon2id with a
  server-side salt). It never stores `authSecret` itself.
- **v1 login:** TLS + verifier. **v1.1 (optional):** upgrade to OPAQUE (RFC 9807) so the
  server never sees anything password-equivalent.
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

- **One annual plan, ~€12/year** (proposed). Avoid monthly charges: fixed per-transaction
  fees swallow small amounts.
- **Merchant of Record:** Paddle (or Lemon Squeezy). They are the legal seller, handle
  global VAT/sales tax, and pay out monthly. No tax paperwork on our side.
- **No artificial limits:** unlimited devices and vaults.
- **Trial:** short free trial to evaluate, then subscribe to keep syncing. Lados without
  an account remains free forever.
- Optional early **“founder” lifetime** tier to bootstrap — only if we accept that it
  funds ongoing costs.

## 12. Hosting and technology

- **Cloudflare Workers** (TypeScript — it does no crypto, so it stays tiny) for the API.
- **R2** for encrypted blobs (no egress fees), **D1** for accounts/versions.
- Expected cost: near-zero initially; a few €/month at thousands of users.
- Server does no cryptography beyond hashing the auth verifier.

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
