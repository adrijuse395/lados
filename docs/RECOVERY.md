# Lados recovery keys

A vault is encrypted with a key derived from your master password. That is the point of Lados —
nobody, including us, can decrypt it — and the flip side is that **forgetting the master password
means losing the vault**. A recovery key is the deliberate way out of that, and it is opt-in.

## What it is

- A **32-byte random code**, written to a file you keep. Lados never stores it and never sends it
  anywhere: the file is the only copy.
- Anyone holding that file can open the vault **without the master password**, so it is exactly as
  sensitive as the password itself. Keep it away from the vault, its backups and any sync folder.
- It is **per vault** and replaceable: creating a new one makes the previous file stop working.

## How it works (the vault format)

`.lados` files have two versions; current Lados reads and writes both.

| Version | How the payload is encrypted |
| --- | --- |
| 1 | with `Argon2id(compositeKey, salt)` — a key that changes with the master password |
| 2 | with a **random 32-byte secret**, which is *wrapped*: once under the password key, and optionally once under the recovery key |

A file only becomes version 2 when a recovery key is created for it; version-1 files are untouched.

The wraps sit between the 80-byte header and the payload (the exact layout is the table at the top
of `src-tauri/src/storage.rs`). Each wrap is XChaCha20-Poly1305 over the 32-byte secret, keyed by an
Argon2id key — `RECOVERY_KDF` for the recovery wrap, which is fixed and never stored, so every
device derives the same key from the same code.

**Why wrap a random secret instead of the password-derived key?** Because a wrapped
password-derived key would break the moment the master password changed: you would believe you had
a recovery key and find out too late that it was dead. With this design every password change
(`VaultKey::rewrap`) re-wraps the *same* secret, so a file you already have keeps working. The tests
in `storage.rs` pin that property down.

## Scope: one key for the account, not one per vault

A recovery code is not tied to a single vault. Each vault stores its own salt, nonce and wrap, so the
**same code** can wrap the secret of every vault that shares your master password — that is what
"one key per account" means here:

- **Create** produces **one code** and applies it to the open vault plus **every synced vault on this
  device**, with **one file** to keep.
- **Recover** takes that file and restores **every** vault that carries the code: the ones on this
  device, and — while a session is still valid — the cloud copies that are not here, pulled and
  re-encrypted under the new master password.
- It then re-registers the **account credential** with the new password and revokes the other
  sessions, so recovery leaves you signed in and syncing: the same reach as a normal password change.

The master password already opens all of those vaults, so the recovery key now matches its scope
instead of being narrower. It follows that the file is worth exactly what the password is worth.

*(A **local-only** vault may use a different password, so it keeps getting a key of its own.)*

> Implementation status: the create side and the account-wide recovery are being built. Until they
> land, a key covers the single vault it was made for.

## Using it

- **Create:** vault menu → **Create a recovery key…** → choose where to save the file.
  (The menu says *Replace* when the open vault already has one.)
- **Open with it:** on the unlock screen → **Use a recovery key…** → pick the file → **set a new
  master password**. The vault is re-encrypted under it, and the same recovery key keeps working.

## What it does not do

- It does **not** recover a **sync account**. The account's password is proved with OPAQUE, so a
  forgotten password cannot authenticate. Recover the vault locally, then sign in and change the
  account password from Sync settings — or connect a fresh account.
- It does **not** help if the recovery file is lost. There is no third copy.
- It is **not** a second factor: it is the same vault secret, reached a different way.
